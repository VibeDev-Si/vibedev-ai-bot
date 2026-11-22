use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::header::HeaderMap;
use hyper::{Method, Request, StatusCode};
use mxlink::matrix_sdk::ruma::events::room::member::MembershipState;
use mxlink::matrix_sdk::ruma::events::room::power_levels::{
    RoomPowerLevels, RoomPowerLevelsEventContent, UserPowerLevel,
};
use mxlink::matrix_sdk::ruma::room_version_rules::AuthorizationRules;
use mxlink::matrix_sdk::ruma::{Int, OwnedUserId, RoomId, UserId};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::assistant::{
    AGENT_ID, Backend, ConfigChange, MemberInfo, RULE_ID, UpdateOutcome, access_rule,
    agent_identifier, check_member,
};
use super::types::{Access, ApiError, ApiKey, PutAssistantRequest, ReplyStyle};
use super::upstream::{KeyNotPermitted, UpstreamFailure};
use super::{ApiState, MAX_BODY_BYTES, handle};
use crate::agent::provider::openai_compat;
use crate::agent::{AgentDefinition, AgentPurpose, PublicIdentifier};
use crate::authorization::{Authorizer, Permission};
use crate::entity::RoomConfigContext;
use crate::entity::globalconfig::GlobalConfig;
use crate::entity::roomconfig::{RoomConfig, TextGenerationReplyStyle};
use crate::vibedev::config::Secret;
use crate::vibedev::net::{BaseUrl, UrlError};
use crate::vibedev::redact;
use crate::vibedev::redact::tests::{LogBuffer, raw_subscriber};

const SECRET: &str = "test-secret-0123456789abcdef0123456789";
const KEY: &str = "sk-test-0123456789abcdefghij";
const OTHER_KEY: &str = "sk-other-9876543210zyxwvutsrq";
const BASE_URL: &str = "https://api.vibedev.example/v1";
const OTHER_BASE_URL: &str = "https://llm.other.example/v1";

const ASSISTANT: &str = "/_vibedev/v1/rooms/!room:example.com/assistant";
const MODELS: &str = "/_vibedev/v1/models";
const ROOM: &str = "!room:example.com";

const ADMIN: &str = "@admin:example.com";
const MODERATOR: &str = "@moderator:example.com";
const FORMER_ADMIN: &str = "@former:example.com";
const CREATOR: &str = "@creator:example.com";
const STRANGER: &str = "@stranger:example.com";

/// Users who aren't room admins (power level below 100, not joined, or not a member at all).
const NOT_ADMINS: [&str; 3] = [MODERATOR, FORMER_ADMIN, STRANGER];

fn user(user_id: &str) -> OwnedUserId {
    UserId::parse(user_id).unwrap()
}

fn level(value: i64) -> UserPowerLevel {
    UserPowerLevel::Int(Int::new(value).unwrap())
}

/// A [`Backend`] keeping one room's configuration in memory, recording pings and model list requests.
struct Fake {
    joined: bool,
    members: HashMap<OwnedUserId, MemberInfo>,
    room_config: Mutex<RoomConfig>,
    host_check: Mutex<Result<(), UrlError>>,
    ping_result: Mutex<Result<(), UpstreamFailure>>,
    pinged: Mutex<Vec<AgentDefinition>>,
    model_requests: Mutex<Vec<(String, String)>>,
}

impl Fake {
    fn new() -> Self {
        let joined = |power_level| MemberInfo {
            membership: Some(MembershipState::Join),
            power_level,
        };

        let members = HashMap::from([
            (user(ADMIN), joined(level(100))),
            (user(MODERATOR), joined(level(50))),
            // Power levels outlive membership: a former admin who left still has 100.
            (
                user(FORMER_ADMIN),
                MemberInfo {
                    membership: Some(MembershipState::Leave),
                    power_level: level(100),
                },
            ),
            // Room creators of room version 12 rooms.
            (user(CREATOR), joined(UserPowerLevel::Infinite)),
        ]);

        Self {
            joined: true,
            members,
            room_config: Mutex::new(RoomConfig::default()),
            host_check: Mutex::new(Ok(())),
            ping_result: Mutex::new(Ok(())),
            pinged: Mutex::default(),
            model_requests: Mutex::default(),
        }
    }

    fn not_joined() -> Self {
        Self {
            joined: false,
            ..Self::new()
        }
    }

    fn context(&self) -> RoomConfigContext {
        RoomConfigContext::new(
            GlobalConfig::new(vec![]),
            self.room_config.lock().unwrap().clone(),
        )
    }

    fn stored_definition(&self) -> Option<AgentDefinition> {
        let room_config = self.room_config.lock().unwrap();
        room_config
            .agents
            .iter()
            .find(|agent| agent.id == AGENT_ID)
            .cloned()
    }

    fn stored_config(&self) -> Option<openai_compat::Config> {
        let definition = self.stored_definition()?;
        Some(serde_yaml_ng::from_value(definition.config).unwrap())
    }

    fn pinged_keys(&self) -> Vec<String> {
        self.pinged
            .lock()
            .unwrap()
            .iter()
            .map(|definition| {
                let config: openai_compat::Config =
                    serde_yaml_ng::from_value(definition.config.clone()).unwrap();
                config.api_key.unwrap_or_default()
            })
            .collect()
    }
}

impl Backend for Fake {
    fn bot_joined(&self, _room_id: &RoomId) -> bool {
        self.joined
    }

    async fn member(&self, _room_id: &RoomId, user_id: &UserId) -> Result<MemberInfo, ApiError> {
        Ok(self.members.get(user_id).cloned().unwrap_or(MemberInfo {
            membership: None,
            power_level: level(0),
        }))
    }

    async fn load(&self, _room_id: &RoomId) -> Result<RoomConfigContext, ApiError> {
        Ok(self.context())
    }

    async fn update(
        &self,
        _room_id: &RoomId,
        change: ConfigChange,
    ) -> Result<UpdateOutcome, ApiError> {
        let changed = {
            let mut room_config = self.room_config.lock().unwrap();
            change(&mut room_config)
        };

        Ok(UpdateOutcome {
            changed,
            context: self.context(),
        })
    }

    async fn check_host(&self, _base_url: &BaseUrl) -> Result<(), UrlError> {
        *self.host_check.lock().unwrap()
    }

    async fn ping(&self, definition: &AgentDefinition) -> Result<(), ApiError> {
        self.pinged.lock().unwrap().push(definition.clone());
        (*self.ping_result.lock().unwrap()).map_err(ApiError::from)
    }

    async fn list_models(
        &self,
        base_url: &BaseUrl,
        api_key: &ApiKey,
    ) -> Result<Vec<String>, ApiError> {
        self.model_requests
            .lock()
            .unwrap()
            .push((base_url.as_str().to_owned(), api_key.expose().to_owned()));

        Ok(vec!["model-a".to_owned(), "model-b".to_owned()])
    }
}

/// The API's state, with the test's own log subscriber.
///
/// Every test calling the API logs through a subscriber of its own (at the trace level): tracing caches across
/// threads whether a log statement is enabled, so a test running without a subscriber could otherwise turn off
/// the statements [`keys_never_reach_the_logs`] checks.
struct TestState {
    state: ApiState<Fake>,
    _logging: tracing::subscriber::DefaultGuard,
}

impl std::ops::Deref for TestState {
    type Target = ApiState<Fake>;

    fn deref(&self) -> &Self::Target {
        &self.state
    }
}

fn api_state(fake: Fake) -> ApiState<Fake> {
    ApiState::new(fake, Secret::new(SECRET.to_owned()))
}

fn logging_to(buffer: &LogBuffer) -> tracing::subscriber::DefaultGuard {
    tracing::subscriber::set_default(raw_subscriber(buffer))
}

fn state(fake: Fake) -> TestState {
    TestState {
        _logging: logging_to(&LogBuffer::default()),
        state: api_state(fake),
    }
}

struct TestResponse {
    status: StatusCode,
    headers: HeaderMap,
    body: String,
    json: serde_json::Value,
}

async fn call_raw(
    state: &ApiState<Fake>,
    method: Method,
    path: &str,
    authorization: Option<&str>,
    content_type: Option<&str>,
    body: String,
) -> TestResponse {
    let mut request = Request::builder().method(method).uri(path);
    if let Some(authorization) = authorization {
        request = request.header("authorization", authorization);
    }
    if let Some(content_type) = content_type {
        request = request.header("content-type", content_type);
    }
    let request = request.body(Full::new(Bytes::from(body))).unwrap();

    let response = handle(state, request).await;

    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body = String::from_utf8(bytes.to_vec()).unwrap();
    let json = serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);

    TestResponse {
        status,
        headers,
        body,
        json,
    }
}

async fn call(
    state: &ApiState<Fake>,
    method: Method,
    path: &str,
    body: Option<serde_json::Value>,
) -> TestResponse {
    call_raw(
        state,
        method,
        path,
        Some(&format!("Bearer {SECRET}")),
        Some("application/json"),
        body.map(|body| body.to_string()).unwrap_or_default(),
    )
    .await
}

async fn get(state: &ApiState<Fake>, acting_user: &str) -> TestResponse {
    let path = format!(
        "{ASSISTANT}?acting_user={}",
        url::form_urlencoded::byte_serialize(acting_user.as_bytes()).collect::<String>()
    );
    call(state, Method::GET, &path, None).await
}

fn put_body(acting_user: &str, api_key: Option<&str>) -> serde_json::Value {
    let mut body = json!({
        "acting_user": acting_user,
        "base_url": BASE_URL,
        "model": "deepseek-chat",
        "system_prompt": "你是本群的助手。",
        "access": "everyone",
        "reply_style": "thread",
    });

    if let Some(api_key) = api_key {
        body["api_key"] = json!(api_key);
    }

    body
}

async fn put(state: &ApiState<Fake>, body: serde_json::Value) -> TestResponse {
    call(state, Method::PUT, ASSISTANT, Some(body)).await
}

async fn delete(state: &ApiState<Fake>, acting_user: &str) -> TestResponse {
    call(
        state,
        Method::DELETE,
        ASSISTANT,
        Some(json!({"acting_user": acting_user})),
    )
    .await
}

async fn models(state: &ApiState<Fake>, body: serde_json::Value) -> TestResponse {
    call(state, Method::POST, MODELS, Some(body)).await
}

fn assert_error(response: &TestResponse, status: StatusCode, errcode: &str) {
    assert_eq!(response.status, status, "{}", response.body);
    assert_eq!(response.json["errcode"], errcode, "{}", response.body);
    assert!(response.json["error"].is_string(), "{}", response.body);
    assert_eq!(
        response.json.as_object().map(|object| object.len()),
        Some(2),
        "errors only have errcode and error: {}",
        response.body
    );
}

#[tokio::test]
async fn requests_need_the_shared_secret() {
    let state = state(Fake::new());

    for authorization in [
        None,
        Some("Bearer wrong-secret".to_owned()),
        Some(format!("Bearer {SECRET}x")),
        Some(format!("Basic {SECRET}")),
        Some(SECRET.to_owned()),
        Some("Bearer ".to_owned()),
    ] {
        // Unknown paths too: unauthenticated callers learn nothing.
        for path in [ASSISTANT, "/_vibedev/v1/unknown", "/"] {
            let response = call_raw(
                &state,
                Method::GET,
                path,
                authorization.as_deref(),
                None,
                String::new(),
            )
            .await;

            assert_error(&response, StatusCode::UNAUTHORIZED, "VD_UNAUTHORIZED");
            assert_eq!(response.headers["www-authenticate"], "Bearer");
        }
    }

    // The scheme is case-insensitive.
    let response = call_raw(
        &state,
        Method::GET,
        &format!("{ASSISTANT}?acting_user={ADMIN}"),
        Some(&format!("bearer {SECRET}")),
        None,
        String::new(),
    )
    .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);
    assert_eq!(
        response.headers["content-type"],
        "application/json; charset=utf-8"
    );
    assert_eq!(response.headers["cache-control"], "no-store");
}

#[tokio::test]
async fn malformed_requests_are_refused() {
    let state = state(Fake::new());

    let response = call(&state, Method::GET, "/_vibedev/v1/unknown", None).await;
    assert_error(&response, StatusCode::NOT_FOUND, "VD_INVALID_REQUEST");

    let response = call(&state, Method::POST, ASSISTANT, Some(json!({}))).await;
    assert_error(
        &response,
        StatusCode::METHOD_NOT_ALLOWED,
        "VD_INVALID_REQUEST",
    );

    let response = call(&state, Method::GET, MODELS, None).await;
    assert_error(
        &response,
        StatusCode::METHOD_NOT_ALLOWED,
        "VD_INVALID_REQUEST",
    );

    let response = call(
        &state,
        Method::GET,
        "/_vibedev/v1/rooms/not-a-room/assistant?acting_user=@admin:example.com",
        None,
    )
    .await;
    assert_error(&response, StatusCode::BAD_REQUEST, "VD_INVALID_REQUEST");

    let response = call(
        &state,
        Method::GET,
        "/_vibedev/v1/rooms/%ZZ/assistant",
        None,
    )
    .await;
    assert_error(&response, StatusCode::BAD_REQUEST, "VD_INVALID_REQUEST");

    // Reading needs acting_user too.
    let response = call(&state, Method::GET, ASSISTANT, None).await;
    assert_error(&response, StatusCode::BAD_REQUEST, "VD_INVALID_REQUEST");

    let response = get(&state, "not-a-user-id").await;
    assert_error(&response, StatusCode::BAD_REQUEST, "VD_INVALID_REQUEST");

    let response = call(
        &state,
        Method::GET,
        &format!("{ASSISTANT}?acting_user={ADMIN}&acting_user={MODERATOR}"),
        None,
    )
    .await;
    assert_error(&response, StatusCode::BAD_REQUEST, "VD_INVALID_REQUEST");

    // Percent-encoded room ids and query strings work.
    let response = call(
        &state,
        Method::GET,
        "/_vibedev/v1/rooms/%21room%3Aexample.com/assistant?acting_user=%40admin%3Aexample.com",
        None,
    )
    .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);

    // JSON only.
    let response = call_raw(
        &state,
        Method::PUT,
        ASSISTANT,
        Some(&format!("Bearer {SECRET}")),
        Some("text/plain"),
        put_body(ADMIN, Some(KEY)).to_string(),
    )
    .await;
    assert_error(
        &response,
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        "VD_INVALID_REQUEST",
    );

    let response = call_raw(
        &state,
        Method::PUT,
        ASSISTANT,
        Some(&format!("Bearer {SECRET}")),
        Some("application/json; charset=utf-8"),
        "{not json".to_owned(),
    )
    .await;
    assert_error(&response, StatusCode::BAD_REQUEST, "VD_INVALID_REQUEST");

    let response = call(&state, Method::PUT, ASSISTANT, None).await;
    assert_error(&response, StatusCode::BAD_REQUEST, "VD_INVALID_REQUEST");

    // Size limit.
    let mut body = put_body(ADMIN, Some(KEY));
    body["system_prompt"] = json!("a".repeat(MAX_BODY_BYTES));
    let response = put(&state, body).await;
    assert_error(
        &response,
        StatusCode::PAYLOAD_TOO_LARGE,
        "VD_INVALID_REQUEST",
    );

    // Unknown fields (e.g. typos, which would otherwise drop the key silently) are refused.
    let mut body = put_body(ADMIN, None);
    body["apiKey"] = json!(KEY);
    let response = put(&state, body).await;
    assert_error(&response, StatusCode::BAD_REQUEST, "VD_INVALID_REQUEST");
    assert!(response.body.contains("apiKey"), "{}", response.body);
    assert!(!response.body.contains(KEY), "{}", response.body);

    for (field, value) in [
        ("access", json!("moderators")),
        ("reply_style", json!("dm")),
        ("model", json!("")),
        ("api_key", json!("short")),
        ("api_key", json!("sk-has space-0123456789")),
    ] {
        let mut body = put_body(ADMIN, Some(KEY));
        body[field] = value;
        let response = put(&state, body).await;
        assert_error(&response, StatusCode::BAD_REQUEST, "VD_INVALID_REQUEST");
    }

    assert!(state.backend.pinged.lock().unwrap().is_empty());
}

#[tokio::test]
async fn reading_shows_everything_to_room_admins_only_and_never_the_key() {
    let state = state(Fake::new());

    // Not configured yet.
    let response = get(&state, ADMIN).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);
    assert_eq!(
        response.json,
        json!({
            "configured": false,
            "joined": true,
            "base_url": null,
            "model": null,
            "system_prompt": null,
            "access": null,
            "reply_style": "thread",
            "key_set": false,
            "key_set_by": null,
            "updated_at": null,
        })
    );

    for acting_user in NOT_ADMINS {
        let response = get(&state, acting_user).await;
        assert_eq!(response.status, StatusCode::OK);
        assert_eq!(response.json, json!({"configured": false, "joined": true}));
    }

    assert_eq!(
        put(&state, put_body(ADMIN, Some(KEY))).await.status,
        StatusCode::OK
    );

    for acting_user in [ADMIN, CREATOR] {
        let response = get(&state, acting_user).await;
        assert_eq!(response.status, StatusCode::OK);
        assert!(!response.body.contains(KEY), "{}", response.body);

        let updated_at = response.json["updated_at"].as_str().unwrap().to_owned();
        assert!(chrono::DateTime::parse_from_rfc3339(&updated_at).is_ok());

        assert_eq!(
            response.json,
            json!({
                "configured": true,
                "joined": true,
                "base_url": BASE_URL,
                "model": "deepseek-chat",
                "system_prompt": "你是本群的助手。",
                "access": "everyone",
                "reply_style": "thread",
                "key_set": true,
                "key_set_by": ADMIN,
                "updated_at": updated_at,
            })
        );
    }

    for acting_user in NOT_ADMINS {
        let response = get(&state, acting_user).await;
        assert_eq!(response.json, json!({"configured": true, "joined": true}));
    }

    // Without the bot in the room, nothing can be read (not even by admins).
    let state = self::state(Fake::not_joined());
    let response = get(&state, ADMIN).await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.json, json!({"configured": false, "joined": false}));
}

#[tokio::test]
async fn put_sets_up_the_assistant() {
    let state = state(Fake::new());

    let response = put(&state, put_body(ADMIN, Some(KEY))).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);
    assert!(!response.body.contains(KEY), "{}", response.body);
    assert_eq!(response.json["configured"], true);
    assert_eq!(response.json["key_set"], true);
    assert_eq!(response.json["key_set_by"], ADMIN);

    // Checked with baibot's ping before saving.
    assert_eq!(state.backend.pinged_keys(), [KEY]);

    // Saved like `agent create-room-local` saves agents, restricted to public addresses.
    let stored = state.backend.stored_config().unwrap();
    assert_eq!(stored.base_url, BASE_URL);
    assert_eq!(stored.api_key.as_deref(), Some(KEY));
    assert!(stored.public_network_only);
    assert!(stored.speech_to_text.is_none());
    assert!(stored.image_generation.is_none());
    let text_generation = stored.text_generation.unwrap();
    assert_eq!(text_generation.model_id, "deepseek-chat");
    assert_eq!(text_generation.prompt.as_deref(), Some("你是本群的助手。"));

    // baibot itself can use the stored agent (the extra `vibedev` metadata doesn't get in the way).
    let definition = state.backend.stored_definition().unwrap();
    let agent = crate::agent::create_from_provider_and_yaml_value_config(
        &definition.provider,
        &PublicIdentifier::DynamicRoomLocal(definition.id.clone()),
        definition.config.clone(),
    )
    .expect("the stored agent should be usable");
    assert_eq!(agent.identifier().as_string(), agent_identifier());

    let room_config = state.backend.room_config.lock().unwrap().clone();
    assert_eq!(
        room_config
            .settings
            .handler
            .get_by_purpose(AgentPurpose::TextGeneration),
        Some(agent_identifier())
    );
    assert_eq!(room_config.access.rules, [access_rule(Access::Everyone)]);
    assert_eq!(
        room_config.settings.text_generation.reply_style,
        Some(TextGenerationReplyStyle::Thread)
    );

    // Changing access, reply style and prompt.
    let mut body = put_body(ADMIN, None);
    body["access"] = json!("admins");
    body["reply_style"] = json!("reply");
    body["system_prompt"] = json!("");
    let response = put(&state, body).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);
    assert_eq!(response.json["access"], "admins");
    assert_eq!(response.json["reply_style"], "reply");
    // An empty prompt means the default one.
    assert_eq!(response.json["system_prompt"], "");
    assert_eq!(
        state
            .backend
            .stored_config()
            .unwrap()
            .text_generation
            .unwrap()
            .prompt
            .as_deref(),
        Some(crate::agent::default_prompt())
    );

    let room_config = state.backend.room_config.lock().unwrap().clone();
    assert_eq!(
        room_config.agents.len(),
        1,
        "the agent is replaced, not added again"
    );
    assert_eq!(room_config.access.rules, [access_rule(Access::Admins)]);
    assert_eq!(
        room_config.settings.text_generation.reply_style,
        Some(TextGenerationReplyStyle::Reply)
    );
}

#[tokio::test]
async fn only_room_admins_may_change_the_assistant() {
    let state = state(Fake::new());

    for acting_user in NOT_ADMINS {
        let response = put(&state, put_body(acting_user, Some(KEY))).await;
        assert_error(&response, StatusCode::FORBIDDEN, "VD_FORBIDDEN");
    }

    // Nothing was pinged or saved.
    assert!(state.backend.pinged.lock().unwrap().is_empty());
    assert!(state.backend.stored_config().is_none());

    // Room creators (room version 12) have an infinite power level.
    assert_eq!(
        put(&state, put_body(CREATOR, Some(KEY))).await.status,
        StatusCode::OK
    );

    for acting_user in NOT_ADMINS {
        assert_error(
            &delete(&state, acting_user).await,
            StatusCode::FORBIDDEN,
            "VD_FORBIDDEN",
        );

        assert_error(
            &models(&state, json!({"room_id": ROOM, "acting_user": acting_user})).await,
            StatusCode::FORBIDDEN,
            "VD_FORBIDDEN",
        );
    }

    assert!(state.backend.stored_config().is_some());
    assert!(state.backend.model_requests.lock().unwrap().is_empty());
}

#[test]
fn power_levels_are_checked() {
    let member = |membership, power_level| MemberInfo {
        membership,
        power_level,
    };

    assert!(check_member(&member(Some(MembershipState::Join), level(100))).is_ok());
    assert!(check_member(&member(Some(MembershipState::Join), level(101))).is_ok());
    assert!(
        check_member(&member(
            Some(MembershipState::Join),
            UserPowerLevel::Infinite
        ))
        .is_ok()
    );

    assert_eq!(
        check_member(&member(Some(MembershipState::Join), level(99))),
        Err(ApiError::forbidden())
    );

    for membership in [
        None,
        Some(MembershipState::Invite),
        Some(MembershipState::Leave),
        Some(MembershipState::Ban),
        Some(MembershipState::Knock),
    ] {
        assert_eq!(
            check_member(&member(membership.clone(), level(100))),
            Err(ApiError::forbidden()),
            "{membership:?}"
        );
    }
}

#[tokio::test]
async fn an_omitted_api_key_keeps_the_stored_one_for_the_same_address() {
    let state = state(Fake::new());

    // Without a stored key, one is needed.
    let response = put(&state, put_body(ADMIN, None)).await;
    assert_error(&response, StatusCode::BAD_REQUEST, "VD_API_KEY_REQUIRED");

    assert_eq!(
        put(&state, put_body(ADMIN, Some(KEY))).await.status,
        StatusCode::OK
    );

    // Missing, null and empty all keep the stored key - also when another admin changes other settings,
    // and the same address written differently is the same address.
    for (acting_user, api_key, base_url) in [
        (ADMIN, None, BASE_URL),
        (ADMIN, Some(json!(null)), BASE_URL),
        (CREATOR, Some(json!("")), BASE_URL),
        (
            CREATOR,
            Some(json!("   ")),
            "https://API.vibedev.example:443/v1/",
        ),
    ] {
        let mut body = put_body(acting_user, None);
        body["model"] = json!("another-model");
        body["base_url"] = json!(base_url);
        if let Some(api_key) = api_key {
            body["api_key"] = api_key;
        }

        let response = put(&state, body).await;
        assert_eq!(response.status, StatusCode::OK, "{}", response.body);
        assert_eq!(response.json["key_set"], true);
        assert_eq!(response.json["model"], "another-model");
        // Still the one who provided the key.
        assert_eq!(response.json["key_set_by"], ADMIN);
        assert!(!response.body.contains(KEY));

        assert_eq!(
            state.backend.stored_config().unwrap().api_key.as_deref(),
            Some(KEY)
        );
    }

    // The stored key was used for each ping.
    assert_eq!(state.backend.pinged_keys(), [KEY; 5]);

    // Another admin can't send the stored key to another address.
    let mut body = put_body(CREATOR, None);
    body["base_url"] = json!(OTHER_BASE_URL);
    let response = put(&state, body).await;
    assert_error(&response, StatusCode::BAD_REQUEST, "VD_API_KEY_REQUIRED");
    assert_eq!(state.backend.pinged_keys().len(), 5, "nothing was pinged");
    assert_eq!(state.backend.stored_config().unwrap().base_url, BASE_URL);

    // With their own key, they can (trimmed, as pasted keys often end with a line break).
    let mut body = put_body(CREATOR, Some(&format!("{OTHER_KEY}\n")));
    body["base_url"] = json!(OTHER_BASE_URL);
    let response = put(&state, body).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);
    assert_eq!(response.json["key_set_by"], CREATOR);
    assert_eq!(
        state.backend.stored_config().unwrap().api_key.as_deref(),
        Some(OTHER_KEY)
    );
}

#[test]
fn access_rules_grant_text_generation_and_nothing_else() {
    let power_levels = {
        let mut content = RoomPowerLevelsEventContent::new(&AuthorizationRules::V1);
        content.users.insert(user(ADMIN), Int::new(100).unwrap());
        content.users.insert(user(MODERATOR), Int::new(50).unwrap());
        RoomPowerLevels::new(content.into(), &AuthorizationRules::V1, vec![])
    };

    // No global rules beyond inviting (see the deployment configuration), no bot administrators among room members.
    let global_config = GlobalConfig::new(vec![]);
    let admin_regexes = mxidwc::parse_patterns_vector(&["@ops:example.com".to_owned()]).unwrap();

    let authorizer = |access| {
        Authorizer::new(&admin_regexes, &global_config)
            .for_room(&[access_rule(access)], Some(power_levels.clone()))
    };

    let everyone = authorizer(Access::Everyone);
    let admins = authorizer(Access::Admins);

    for user_id in [ADMIN, MODERATOR, STRANGER, "@someone:other.example"] {
        assert!(
            everyone.allows(&user(user_id), Permission::TextGeneration),
            "{user_id}"
        );
    }

    assert!(admins.allows(&user(ADMIN), Permission::TextGeneration));
    assert!(!admins.allows(&user(MODERATOR), Permission::TextGeneration));
    assert!(!admins.allows(&user(STRANGER), Permission::TextGeneration));

    // Nobody in the room may manage agents, settings or access rules (so no agents get created in the chat),
    // or use other features (no agents for them are set up).
    for authorizer in [&everyone, &admins] {
        for user_id in [ADMIN, MODERATOR, STRANGER] {
            for permission in [
                Permission::ManageAgents,
                Permission::ManageConfig,
                Permission::ManageAccess,
                Permission::Administer,
                Permission::ImageGeneration,
                Permission::SpeechToText,
                Permission::TextToSpeech,
                Permission::Invite,
            ] {
                assert!(
                    !authorizer.allows(&user(user_id), permission),
                    "{user_id} {permission:?}"
                );
            }
        }
    }
}

#[tokio::test]
async fn the_bot_must_be_in_the_room() {
    let state = state(Fake::not_joined());

    assert_error(
        &put(&state, put_body(ADMIN, Some(KEY))).await,
        StatusCode::CONFLICT,
        "VD_BOT_NOT_IN_ROOM",
    );
    assert_error(
        &delete(&state, ADMIN).await,
        StatusCode::CONFLICT,
        "VD_BOT_NOT_IN_ROOM",
    );
    assert_error(
        &models(&state, json!({"room_id": ROOM, "acting_user": ADMIN})).await,
        StatusCode::CONFLICT,
        "VD_BOT_NOT_IN_ROOM",
    );

    // A new key needs no room.
    let response = models(&state, json!({"base_url": BASE_URL, "api_key": KEY})).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);

    assert!(state.backend.pinged.lock().unwrap().is_empty());
}

#[tokio::test]
async fn only_public_https_addresses_are_accepted() {
    let state = state(Fake::new());

    for base_url in [
        "http://api.vibedev.example/v1",
        "https://127.0.0.1/v1",
        "https://169.254.169.254/latest",
        "https://[fd00::1]/v1",
        "https://localhost/v1",
        "https://synapse:8008",
        "https://user:pass@api.vibedev.example/v1",
        "https://api.vibedev.example/v1?x=1",
    ] {
        let mut body = put_body(ADMIN, Some(KEY));
        body["base_url"] = json!(base_url);

        let response = put(&state, body).await;
        assert_error(
            &response,
            StatusCode::BAD_REQUEST,
            "VD_BASE_URL_NOT_ALLOWED",
        );

        let response = models(&state, json!({"base_url": base_url, "api_key": KEY})).await;
        assert_error(
            &response,
            StatusCode::BAD_REQUEST,
            "VD_BASE_URL_NOT_ALLOWED",
        );
    }

    // Host names resolving to non-public addresses.
    *state.backend.host_check.lock().unwrap() = Err(UrlError::NonPublicAddress);

    let response = put(&state, put_body(ADMIN, Some(KEY))).await;
    assert_error(
        &response,
        StatusCode::BAD_REQUEST,
        "VD_BASE_URL_NOT_ALLOWED",
    );

    let response = models(&state, json!({"base_url": BASE_URL, "api_key": KEY})).await;
    assert_error(
        &response,
        StatusCode::BAD_REQUEST,
        "VD_BASE_URL_NOT_ALLOWED",
    );

    assert!(state.backend.pinged.lock().unwrap().is_empty());
    assert!(state.backend.model_requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn failed_pings_save_nothing_and_say_why() {
    let state = state(Fake::new());

    for (failure, status, errcode) in [
        (
            UpstreamFailure::InvalidApiKey { status: 401 },
            StatusCode::UNPROCESSABLE_ENTITY,
            "VD_INVALID_API_KEY",
        ),
        (
            UpstreamFailure::KeyNotPermitted(KeyNotPermitted::MissingVibeDevSignature),
            StatusCode::UNPROCESSABLE_ENTITY,
            "VD_KEY_NOT_PERMITTED",
        ),
        (
            UpstreamFailure::KeyNotPermitted(KeyNotPermitted::GroupPublicApiAccessDisabled),
            StatusCode::UNPROCESSABLE_ENTITY,
            "VD_KEY_NOT_PERMITTED",
        ),
        (
            UpstreamFailure::ModelNotFound { status: 404 },
            StatusCode::UNPROCESSABLE_ENTITY,
            "VD_MODEL_NOT_FOUND",
        ),
        (
            UpstreamFailure::Status(500),
            StatusCode::BAD_GATEWAY,
            "VD_UPSTREAM_ERROR",
        ),
        (
            UpstreamFailure::Unreachable,
            StatusCode::BAD_GATEWAY,
            "VD_UPSTREAM_ERROR",
        ),
        (
            UpstreamFailure::Timeout,
            StatusCode::GATEWAY_TIMEOUT,
            "VD_UPSTREAM_TIMEOUT",
        ),
    ] {
        *state.backend.ping_result.lock().unwrap() = Err(failure);

        let response = put(&state, put_body(ADMIN, Some(KEY))).await;
        assert_error(&response, status, errcode);
        assert!(!response.body.contains(KEY), "{}", response.body);

        assert!(state.backend.stored_config().is_none());
        assert!(
            state
                .backend
                .room_config
                .lock()
                .unwrap()
                .access
                .rules
                .is_empty()
        );
    }
}

#[tokio::test]
async fn delete_removes_agent_handler_and_rule_and_can_be_repeated() {
    let state = state(Fake::new());

    // Nothing to remove is fine too.
    let response = delete(&state, ADMIN).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);
    assert_eq!(response.json, json!({"configured": false}));

    assert_eq!(
        put(&state, put_body(ADMIN, Some(KEY))).await.status,
        StatusCode::OK
    );

    // Other rules stay.
    {
        let mut room_config = state.backend.room_config.lock().unwrap();
        let mut other_rule = access_rule(Access::Everyone);
        other_rule.id = "other".to_owned();
        room_config.access.rules.push(other_rule);
    }

    // acting_user in the query string.
    let response = call_raw(
        &state,
        Method::DELETE,
        &format!("{ASSISTANT}?acting_user=%40admin%3Aexample.com"),
        Some(&format!("Bearer {SECRET}")),
        None,
        String::new(),
    )
    .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);
    assert_eq!(response.json, json!({"configured": false}));

    let room_config = state.backend.room_config.lock().unwrap().clone();
    assert!(room_config.agents.is_empty());
    assert_eq!(
        room_config
            .settings
            .handler
            .get_by_purpose(AgentPurpose::TextGeneration),
        None
    );
    assert_eq!(
        room_config
            .access
            .rules
            .iter()
            .map(|rule| rule.id.as_str())
            .collect::<Vec<_>>(),
        ["other"]
    );
    assert!(
        room_config
            .access
            .rules
            .iter()
            .all(|rule| rule.id != RULE_ID)
    );

    assert_eq!(get(&state, ADMIN).await.json["configured"], false);

    // Again, with the body.
    let response = delete(&state, ADMIN).await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.json, json!({"configured": false}));

    // acting_user is required, and must not differ between body and query string.
    let response = call(&state, Method::DELETE, ASSISTANT, None).await;
    assert_error(&response, StatusCode::BAD_REQUEST, "VD_INVALID_REQUEST");

    let response = call(
        &state,
        Method::DELETE,
        &format!("{ASSISTANT}?acting_user={MODERATOR}"),
        Some(json!({"acting_user": ADMIN})),
    )
    .await;
    assert_error(&response, StatusCode::BAD_REQUEST, "VD_INVALID_REQUEST");
}

#[tokio::test]
async fn models_are_listed_with_a_new_key_or_the_stored_key_and_address() {
    let state = state(Fake::new());

    // A new key.
    let response = models(&state, json!({"base_url": BASE_URL, "api_key": OTHER_KEY})).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);
    assert_eq!(response.json, json!({"models": ["model-a", "model-b"]}));

    // Without a key, a stored one is needed.
    for body in [
        json!({"base_url": BASE_URL}),
        json!({"room_id": ROOM, "acting_user": ADMIN}),
    ] {
        let response = models(&state, body).await;
        assert_error(&response, StatusCode::BAD_REQUEST, "VD_API_KEY_REQUIRED");
    }

    assert_eq!(
        put(&state, put_body(ADMIN, Some(KEY))).await.status,
        StatusCode::OK
    );

    // The stored key goes to the stored address, which a base_url sent along must be.
    for body in [
        json!({"room_id": ROOM, "acting_user": CREATOR}),
        json!({"room_id": ROOM, "acting_user": ADMIN, "base_url": BASE_URL}),
        json!({"room_id": ROOM, "acting_user": ADMIN, "base_url": "https://api.vibedev.example/v1/"}),
    ] {
        let response = models(&state, body).await;
        assert_eq!(response.status, StatusCode::OK, "{}", response.body);
        assert!(!response.body.contains(KEY));
    }

    let response = models(
        &state,
        json!({"room_id": ROOM, "acting_user": ADMIN, "base_url": OTHER_BASE_URL}),
    )
    .await;
    assert_error(&response, StatusCode::BAD_REQUEST, "VD_API_KEY_REQUIRED");

    // Either a new key or a room, not both.
    for body in [
        json!({"base_url": BASE_URL, "api_key": KEY, "room_id": ROOM, "acting_user": ADMIN}),
        json!({"base_url": BASE_URL, "api_key": KEY, "acting_user": ADMIN}),
        json!({"api_key": KEY}),
        json!({"room_id": ROOM}),
    ] {
        let response = models(&state, body).await;
        assert_error(&response, StatusCode::BAD_REQUEST, "VD_INVALID_REQUEST");
    }

    assert_eq!(
        *state.backend.model_requests.lock().unwrap(),
        [
            (BASE_URL.to_owned(), OTHER_KEY.to_owned()),
            (BASE_URL.to_owned(), KEY.to_owned()),
            (BASE_URL.to_owned(), KEY.to_owned()),
            (BASE_URL.to_owned(), KEY.to_owned()),
        ]
    );
}

#[tokio::test]
async fn keys_never_reach_the_logs() {
    let buffer = LogBuffer::default();

    // Logs as written, without redaction: the API itself must not log keys.
    let state = TestState {
        _logging: logging_to(&buffer),
        state: api_state(Fake::new()),
    };

    assert_eq!(
        put(&state, put_body(ADMIN, Some(KEY))).await.status,
        StatusCode::OK
    );
    assert_eq!(
        put(&state, put_body(ADMIN, None)).await.status,
        StatusCode::OK
    );
    get(&state, ADMIN).await;

    // A failing ping.
    *state.backend.ping_result.lock().unwrap() =
        Err(UpstreamFailure::InvalidApiKey { status: 401 });
    let response = put(&state, put_body(ADMIN, Some(OTHER_KEY))).await;
    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(!response.body.contains(OTHER_KEY));

    // A key in the wrong field doesn't come back in the error.
    let mut body = put_body(ADMIN, Some(KEY));
    body["access"] = json!(KEY);
    let response = put(&state, body).await;
    assert_eq!(response.status, StatusCode::BAD_REQUEST);
    assert!(!response.body.contains(KEY), "{}", response.body);

    // Refused requests and model lists.
    put(&state, put_body(MODERATOR, Some(KEY))).await;
    models(&state, json!({"base_url": BASE_URL, "api_key": KEY})).await;
    models(&state, json!({"room_id": ROOM, "acting_user": ADMIN})).await;
    delete(&state, ADMIN).await;

    let logs = buffer.contents();
    assert!(logs.contains("VibeDev internal API request"), "{logs}");
    assert!(logs.contains("Configured the VibeDev assistant"), "{logs}");
    assert!(!logs.contains(KEY), "{logs}");
    assert!(!logs.contains(OTHER_KEY), "{logs}");

    // Keys the API handled are redacted from all other logs (e.g. provider errors logged by baibot itself).
    assert_eq!(
        redact::redact(&format!("echo {KEY} {OTHER_KEY}"), &[]),
        format!("echo {} {}", redact::REDACTED, redact::REDACTED)
    );
}

#[tokio::test]
async fn leaving_the_room_forgets_the_assistant_and_its_key() {
    let state = state(Fake::new());

    let mut body = put_body(ADMIN, Some(KEY));
    body["reply_style"] = json!("reply");
    assert_eq!(put(&state, body).await.status, StatusCode::OK);

    // Things the API didn't set up stay.
    let other_agent = AgentDefinition::new(
        "other".to_owned(),
        crate::agent::AgentProvider::OpenAICompat,
        serde_yaml_ng::Value::Null,
    );
    let mut room_config = state.backend.room_config.lock().unwrap().clone();
    room_config.agents.push(other_agent);
    let mut other_rule = access_rule(Access::Everyone);
    other_rule.id = "other".to_owned();
    room_config.access.rules.push(other_rule);
    room_config.settings.text_generation.prompt_override = Some("Be brief.".to_owned());

    // What gets saved (encrypted) in the room's account data contains the key...
    assert!(serde_json::to_string(&room_config).unwrap().contains(KEY));

    assert!(super::assistant::forget(&mut room_config));

    // ...and no longer does once the bot has left.
    let saved = serde_json::to_string(&room_config).unwrap();
    assert!(!saved.contains(KEY), "{saved}");

    assert_eq!(
        room_config
            .agents
            .iter()
            .map(|agent| agent.id.as_str())
            .collect::<Vec<_>>(),
        ["other"]
    );
    assert_eq!(
        room_config
            .settings
            .handler
            .get_by_purpose(AgentPurpose::TextGeneration),
        None
    );
    assert_eq!(
        room_config
            .access
            .rules
            .iter()
            .map(|rule| rule.id.as_str())
            .collect::<Vec<_>>(),
        ["other"]
    );
    assert_eq!(room_config.settings.text_generation.reply_style, None);
    assert_eq!(
        room_config
            .settings
            .text_generation
            .prompt_override
            .as_deref(),
        Some("Be brief.")
    );

    // Nothing left to forget (so nothing gets saved again).
    assert!(!super::assistant::forget(&mut room_config));
    assert!(!super::assistant::forget(&mut RoomConfig::default()));
}

#[test]
fn error_messages_posted_in_rooms_hide_keys() {
    let key = "sk-test-error-message-0123456789";
    redact::register(key);

    let message = crate::strings::agent::error_while_serving_purpose(
        &PublicIdentifier::DynamicRoomLocal(AGENT_ID.to_owned()),
        &AgentPurpose::TextGeneration,
        format!(
            "The OpenAI-compat chat completion API request failed (401 Unauthorized): Incorrect API key provided: {key}"
        ),
    );

    assert!(!message.contains(key), "{message}");
    assert!(message.contains(redact::REDACTED), "{message}");
    assert!(message.contains("401 Unauthorized"), "{message}");

    let message = crate::strings::agent::configuration_does_not_result_in_a_working_agent(
        anyhow::anyhow!("Invalid key: {key}"),
    );
    assert!(!message.contains(key), "{message}");
}

#[test]
fn requests_hide_keys_in_debug_output() {
    let request: PutAssistantRequest = serde_json::from_value(put_body(ADMIN, Some(KEY))).unwrap();
    let debug = format!("{request:?}");
    assert!(!debug.contains(KEY), "{debug}");
    assert_eq!(request.reply_style, ReplyStyle::Thread);
}

#[tokio::test]
async fn the_server_enforces_its_limits_over_tcp() {
    let _logging = logging_to(&LogBuffer::default());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(super::serve(listener, Arc::new(api_state(Fake::new()))));

    async fn exchange(address: SocketAddr, request: &[u8]) -> String {
        let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
        let _ = stream.write_all(request).await;
        let mut response = Vec::new();
        let _ =
            tokio::time::timeout(Duration::from_secs(10), stream.read_to_end(&mut response)).await;
        String::from_utf8_lossy(&response).into_owned()
    }

    let response = exchange(
        address,
        b"GET /_vibedev/v1/models HTTP/1.1\r\nHost: baibot\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert!(response.starts_with("HTTP/1.1 401"), "{response}");

    let request = format!(
        "GET /_vibedev/v1/rooms/%21room%3Aexample.com/assistant?acting_user=%40moderator%3Aexample.com HTTP/1.1\r\nHost: baibot\r\nAuthorization: Bearer {SECRET}\r\nConnection: close\r\n\r\n"
    );
    let response = exchange(address, request.as_bytes()).await;
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(
        response.ends_with(r#"{"configured":false,"joined":true}"#),
        "{response}"
    );

    let request = format!(
        "PUT {ASSISTANT} HTTP/1.1\r\nHost: baibot\r\nAuthorization: Bearer {SECRET}\r\nContent-Type: application/json\r\nContent-Length: 1000000\r\nConnection: close\r\n\r\n{{"
    );
    let response = exchange(address, request.as_bytes()).await;
    assert!(response.starts_with("HTTP/1.1 413"), "{response}");

    let request = format!(
        "GET /_vibedev/v1/models HTTP/1.1\r\nHost: baibot\r\nX-Padding: {}\r\nConnection: close\r\n\r\n",
        "a".repeat(64 * 1024)
    );
    let response = exchange(address, request.as_bytes()).await;
    assert!(
        response.is_empty() || response.starts_with("HTTP/1.1 431"),
        "{response}"
    );

    server.abort();
}
