//! The assistant's configuration in a room: reading, setting and removing it, and listing an API's models.
//!
//! The assistant is a room-local agent (`room-local/vibedev-assistant`, provider `openai-compatible`), created the way
//! `agent create-room-local` creates agents (checked with the same ping first), made the room's text-generation handler,
//! together with a room access rule (`vibedev-assistant`) saying who may use it.
//!
//! Matrix access goes through [`Backend`], so that this can be tested without a homeserver.

use std::future::Future;

use mxlink::matrix_sdk::ruma::events::room::member::MembershipState;
use mxlink::matrix_sdk::ruma::events::room::power_levels::UserPowerLevel;
use mxlink::matrix_sdk::ruma::{Int, OwnedRoomId, OwnedUserId, RoomId, UserId};
use serde::{Deserialize, Serialize};

use crate::agent::provider::openai_compat;
use crate::agent::{AgentDefinition, AgentProvider, AgentPurpose, PublicIdentifier};
use crate::authorization::{AccessRule, Permission, RuleEffect};
use crate::entity::RoomConfigContext;
use crate::entity::roomconfig::RoomConfig;
use crate::vibedev::net::{self, BaseUrl, UrlError};
use crate::vibedev::redact;

use super::types::{
    Access, AdminView, ApiError, ApiKey, AssistantView, DeletedView, LimitedView, ModelsRequest,
    ModelsView, PutAssistantRequest, ReplyStyle,
};

/// The id of the room-local agent the API manages.
pub const AGENT_ID: &str = "vibedev-assistant";

/// The id of the room access rule the API manages.
pub const RULE_ID: &str = "vibedev-assistant";

/// The power level needed for seeing and changing the assistant's configuration
/// (and, with [`Access::Admins`], for using it).
pub const ADMIN_POWER_LEVEL: i64 = 100;

/// Room access rules match users by pattern. Room members are the only ones who can talk to the bot in a room.
const ALL_USERS_PATTERN: &str = "@*:*";

/// Where the agent's configuration keeps who set the key and when it was saved (ignored by the provider).
const META_KEY: &str = "vibedev";

pub const MAX_MODEL_CHARS: usize = 256;

pub const MAX_SYSTEM_PROMPT_CHARS: usize = 16_000;

pub const MIN_API_KEY_LENGTH: usize = redact::MIN_SECRET_LENGTH;

pub const MAX_API_KEY_LENGTH: usize = 1024;

/// A user's membership and power level in a room.
#[derive(Clone, Debug)]
pub struct MemberInfo {
    pub membership: Option<MembershipState>,
    pub power_level: UserPowerLevel,
}

/// Changes a room's configuration, returning whether anything changed.
pub type ConfigChange = Box<dyn FnOnce(&mut RoomConfig) -> bool + Send>;

pub struct UpdateOutcome {
    pub changed: bool,
    /// The configuration after the change.
    pub context: RoomConfigContext,
}

/// What the API needs from the bot.
pub trait Backend: Send + Sync + 'static {
    /// Tells whether the bot is joined to the room.
    fn bot_joined(&self, room_id: &RoomId) -> bool;

    /// Reads a user's membership and power level from the room's state (the bot must be joined).
    fn member(
        &self,
        room_id: &RoomId,
        user_id: &UserId,
    ) -> impl Future<Output = Result<MemberInfo, ApiError>> + Send;

    /// Loads the global and the room's configuration (the bot must be joined).
    fn load(
        &self,
        room_id: &RoomId,
    ) -> impl Future<Output = Result<RoomConfigContext, ApiError>> + Send;

    /// Changes the room's configuration without losing concurrent changes, persisting it if it changed.
    fn update(
        &self,
        room_id: &RoomId,
        change: ConfigChange,
    ) -> impl Future<Output = Result<UpdateOutcome, ApiError>> + Send;

    /// Checks that the base URL's host resolves to public addresses only.
    fn check_host(&self, base_url: &BaseUrl) -> impl Future<Output = Result<(), UrlError>> + Send;

    /// Checks that the agent works, with baibot's ping (as when creating agents).
    /// Errors say what went wrong without passing on what the model service answered.
    fn ping(
        &self,
        definition: &AgentDefinition,
    ) -> impl Future<Output = Result<(), ApiError>> + Send;

    /// Lists the models of an OpenAI-compatible API (`GET {base_url}/models`).
    fn list_models(
        &self,
        base_url: &BaseUrl,
        api_key: &ApiKey,
    ) -> impl Future<Output = Result<Vec<String>, ApiError>> + Send;
}

/// The agent's public identifier (`room-local/vibedev-assistant`), as the room's handler setting refers to it.
pub fn agent_identifier() -> String {
    PublicIdentifier::DynamicRoomLocal(AGENT_ID.to_owned()).as_string()
}

/// `GET /_vibedev/v1/rooms/{room_id}/assistant?acting_user=...`
///
/// Room admins (see [`check_member`]) see the whole configuration (without the key), others only whether there is one.
pub async fn get<B: Backend>(
    backend: &B,
    room_id: &RoomId,
    acting_user: &str,
) -> Result<AssistantView, ApiError> {
    let acting_user = parse_user_id(acting_user)?;

    if !backend.bot_joined(room_id) {
        return Ok(AssistantView::Limited(LimitedView {
            configured: false,
            joined: false,
        }));
    }

    let is_admin = check_member(&backend.member(room_id, &acting_user).await?).is_ok();

    let context = backend.load(room_id).await?;

    Ok(if is_admin {
        AssistantView::Admin(admin_view(&context))
    } else {
        AssistantView::Limited(LimitedView {
            configured: stored_agent(&context.room_config).is_some(),
            joined: true,
        })
    })
}

/// `PUT /_vibedev/v1/rooms/{room_id}/assistant`
pub async fn put<B: Backend>(
    backend: &B,
    room_id: &RoomId,
    request: PutAssistantRequest,
) -> Result<AssistantView, ApiError> {
    let input = PutInput::parse(request)?;

    if let Some(api_key) = &input.api_key {
        redact::register(api_key.expose());
    }

    if !backend.bot_joined(room_id) {
        return Err(ApiError::bot_not_in_room());
    }

    authorize(backend, room_id, &input.acting_user).await?;

    backend
        .check_host(&input.base_url)
        .await
        .map_err(ApiError::base_url_not_allowed)?;

    let context = backend.load(room_id).await?;
    let stored = stored_agent(&context.room_config);

    let api_key = choose_api_key(input.api_key.clone(), stored.as_ref(), &input.base_url)?;

    let meta = AssistantMeta {
        // Whoever provided the key last.
        key_set_by: match &input.api_key {
            Some(_) => Some(input.acting_user.to_string()),
            None => stored.and_then(|stored| stored.meta.key_set_by),
        },
        updated_at: Some(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
        updated_by: Some(input.acting_user.to_string()),
    };

    let definition = agent_definition(&input, &api_key, &meta)?;

    if let Err(err) = backend.ping(&definition).await {
        tracing::info!(
            room_id = room_id.as_str(),
            acting_user = input.acting_user.as_str(),
            base_url = input.base_url.as_str(),
            model = input.model,
            errcode = err.errcode,
            "The VibeDev assistant configuration did not pass the ping"
        );

        return Err(err);
    }

    let (access, reply_style) = (input.access, input.reply_style);

    let outcome = backend
        .update(
            room_id,
            Box::new(move |room_config| {
                apply(room_config, definition, access, reply_style);
                true
            }),
        )
        .await?;

    tracing::info!(
        room_id = room_id.as_str(),
        acting_user = input.acting_user.as_str(),
        base_url = input.base_url.as_str(),
        model = input.model,
        ?access,
        ?reply_style,
        api_key_replaced = input.api_key.is_some(),
        "Configured the VibeDev assistant"
    );

    Ok(AssistantView::Admin(admin_view(&outcome.context)))
}

/// `DELETE /_vibedev/v1/rooms/{room_id}/assistant` (succeeds when there's nothing to remove too).
pub async fn delete<B: Backend>(
    backend: &B,
    room_id: &RoomId,
    acting_user: &str,
) -> Result<DeletedView, ApiError> {
    let acting_user = parse_user_id(acting_user)?;

    if !backend.bot_joined(room_id) {
        return Err(ApiError::bot_not_in_room());
    }

    authorize(backend, room_id, &acting_user).await?;

    let outcome = backend.update(room_id, Box::new(remove)).await?;

    if outcome.changed {
        tracing::info!(
            room_id = room_id.as_str(),
            acting_user = acting_user.as_str(),
            "Removed the VibeDev assistant"
        );
    }

    Ok(DeletedView { configured: false })
}

/// `POST /_vibedev/v1/models`: with `{base_url, api_key}`, or with `{room_id, acting_user}` for the room's stored key
/// and address (a `base_url` sent along must be that address).
pub async fn models<B: Backend>(
    backend: &B,
    request: ModelsRequest,
) -> Result<ModelsView, ApiError> {
    let api_key = parse_optional_api_key(request.api_key)?;

    let (base_url, api_key) = match (api_key, request.room_id) {
        (Some(_), Some(_)) => {
            return Err(ApiError::invalid_request(
                "Send either {base_url, api_key} or {room_id, acting_user}, not both",
            ));
        }
        (Some(api_key), None) => {
            if request.acting_user.is_some() {
                return Err(ApiError::invalid_request(
                    "acting_user only goes with room_id (for the room's stored key)",
                ));
            }

            let base_url = request
                .base_url
                .ok_or_else(|| ApiError::invalid_request("base_url is required with api_key"))?;
            let base_url =
                net::parse_base_url(&base_url).map_err(ApiError::base_url_not_allowed)?;

            redact::register(api_key.expose());

            (base_url, api_key)
        }
        (None, Some(room_id)) => {
            let room_id = parse_room_id(&room_id)?;
            let acting_user = request
                .acting_user
                .ok_or_else(|| ApiError::invalid_request("acting_user is required with room_id"))?;
            let acting_user = parse_user_id(&acting_user)?;

            if !backend.bot_joined(&room_id) {
                return Err(ApiError::bot_not_in_room());
            }

            authorize(backend, &room_id, &acting_user).await?;

            let context = backend.load(&room_id).await?;

            let Some((stored_base_url, api_key)) = stored_agent(&context.room_config)
                .and_then(|stored| stored.api_key.map(|api_key| (stored.base_url, api_key)))
            else {
                return Err(ApiError::api_key_required(
                    "No key is stored for this room: send {base_url, api_key}",
                ));
            };

            let stored_base_url = net::parse_base_url(&stored_base_url)
                .map_err(|_| ApiError::internal("The stored base_url is not valid anymore"))?;

            if let Some(base_url) = &request.base_url
                && !is_same_address(base_url, &stored_base_url)
            {
                return Err(ApiError::api_key_required(
                    "api_key is required for another base_url (a stored key is only sent to the address it was entered for)",
                ));
            }

            (stored_base_url, api_key)
        }
        (None, None) => {
            return Err(ApiError::api_key_required(
                "Send {base_url, api_key}, or {room_id, acting_user} to use the room's stored key",
            ));
        }
    };

    backend
        .check_host(&base_url)
        .await
        .map_err(ApiError::base_url_not_allowed)?;

    let models = backend.list_models(&base_url, &api_key).await?;

    Ok(ModelsView { models })
}

/// Checks that the user may change the room's assistant (see [`check_member`]).
///
/// Read from the room's state by the bot: callers only say who acts.
async fn authorize<B: Backend>(
    backend: &B,
    room_id: &RoomId,
    user_id: &UserId,
) -> Result<(), ApiError> {
    let member = backend.member(room_id, user_id).await?;

    let result = check_member(&member);

    if result.is_err() {
        tracing::info!(
            room_id = room_id.as_str(),
            acting_user = user_id.as_str(),
            membership = ?member.membership,
            "Refused a VibeDev assistant change by a user who isn't a room admin"
        );
    }

    result
}

/// Room admins are joined members with a power level of at least 100
/// (room creators of room version 12 rooms have an infinite one).
pub fn check_member(member: &MemberInfo) -> Result<(), ApiError> {
    let required = Int::new(ADMIN_POWER_LEVEL).expect("100 is a valid power level");

    if member.membership == Some(MembershipState::Join) && member.power_level >= required {
        Ok(())
    } else {
        Err(ApiError::forbidden())
    }
}

pub fn parse_room_id(value: &str) -> Result<OwnedRoomId, ApiError> {
    OwnedRoomId::try_from(value.trim()).map_err(|_| {
        ApiError::invalid_request("room_id must be a Matrix room ID (e.g. !abc:example.com)")
    })
}

fn parse_user_id(value: &str) -> Result<OwnedUserId, ApiError> {
    OwnedUserId::try_from(value.trim()).map_err(|_| {
        ApiError::invalid_request("acting_user must be a Matrix user ID (e.g. @alice:example.com)")
    })
}

/// An empty key counts as no key. Keys are trimmed (as pasted keys often end with a line break).
fn parse_optional_api_key(api_key: Option<ApiKey>) -> Result<Option<ApiKey>, ApiError> {
    let Some(api_key) = api_key else {
        return Ok(None);
    };

    let value = api_key.expose().trim();

    if value.is_empty() {
        return Ok(None);
    }

    let valid = (MIN_API_KEY_LENGTH..=MAX_API_KEY_LENGTH).contains(&value.len())
        && value
            .bytes()
            .all(|byte| (0x21..=0x7e).contains(&byte) && byte != b'"' && byte != b'\\');

    if !valid {
        return Err(ApiError::invalid_request(format!(
            "api_key must be {MIN_API_KEY_LENGTH}-{MAX_API_KEY_LENGTH} printable ASCII characters (without spaces, quotes or backslashes)"
        )));
    }

    Ok(Some(ApiKey::new(value.to_owned())))
}

/// Tells whether a base URL is the given (stored) one, as normalized by [`net::parse_base_url`]
/// (so that e.g. a trailing slash or upper-case letters in the host name make no difference).
fn is_same_address(base_url: &str, stored: &BaseUrl) -> bool {
    net::parse_base_url(base_url).is_ok_and(|base_url| base_url.as_str() == stored.as_str())
}

/// A validated `PUT` request.
struct PutInput {
    acting_user: OwnedUserId,
    base_url: BaseUrl,
    api_key: Option<ApiKey>,
    model: String,
    system_prompt: String,
    access: Access,
    reply_style: ReplyStyle,
}

impl PutInput {
    fn parse(request: PutAssistantRequest) -> Result<Self, ApiError> {
        let acting_user = parse_user_id(&request.acting_user)?;

        let base_url =
            net::parse_base_url(&request.base_url).map_err(ApiError::base_url_not_allowed)?;

        let api_key = parse_optional_api_key(request.api_key)?;

        let model = request.model.trim().to_owned();
        if model.is_empty()
            || model.chars().count() > MAX_MODEL_CHARS
            || model.contains(char::is_control)
        {
            return Err(ApiError::invalid_request(format!(
                "model must be 1-{MAX_MODEL_CHARS} characters long (without control characters)"
            )));
        }

        let system_prompt = request.system_prompt.unwrap_or_default().trim().to_owned();
        if system_prompt.chars().count() > MAX_SYSTEM_PROMPT_CHARS {
            return Err(ApiError::invalid_request(format!(
                "system_prompt must be at most {MAX_SYSTEM_PROMPT_CHARS} characters long"
            )));
        }

        Ok(Self {
            acting_user,
            base_url,
            api_key,
            model,
            system_prompt,
            access: request.access,
            reply_style: request.reply_style,
        })
    }
}

/// Who set the key and when the configuration was saved, kept in the agent's configuration (see [`META_KEY`]).
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
struct AssistantMeta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    key_set_by: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    updated_at: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    updated_by: Option<String>,
}

/// What's stored about the room's assistant agent.
struct StoredAgent {
    base_url: String,
    api_key: Option<ApiKey>,
    model: String,
    prompt: Option<String>,
    meta: AssistantMeta,
}

fn stored_agent(room_config: &RoomConfig) -> Option<StoredAgent> {
    let definition = room_config
        .agents
        .iter()
        .find(|agent| agent.id == AGENT_ID)?;

    if !matches!(definition.provider, AgentProvider::OpenAICompat) {
        return None;
    }

    let config: openai_compat::Config =
        serde_yaml_ng::from_value(definition.config.clone()).ok()?;
    let text_generation = config.text_generation?;

    let meta = definition
        .config
        .get(META_KEY)
        .and_then(|meta| serde_yaml_ng::from_value(meta.clone()).ok())
        .unwrap_or_default();

    Some(StoredAgent {
        base_url: config.base_url,
        api_key: config
            .api_key
            .filter(|api_key| !api_key.is_empty())
            .map(ApiKey::new),
        model: text_generation.model_id,
        prompt: text_generation.prompt,
        meta,
    })
}

/// Picks the key to use: the one sent, or else the stored one - but only for the address it was entered for, so that
/// another room admin changing `base_url` can't send someone else's key to their own server.
fn choose_api_key(
    provided: Option<ApiKey>,
    stored: Option<&StoredAgent>,
    base_url: &BaseUrl,
) -> Result<ApiKey, ApiError> {
    if let Some(api_key) = provided {
        return Ok(api_key);
    }

    let Some((stored, api_key)) =
        stored.and_then(|stored| stored.api_key.as_ref().map(|api_key| (stored, api_key)))
    else {
        return Err(ApiError::api_key_required(
            "api_key is required: no key is stored for this room yet",
        ));
    };

    let same_base_url = net::parse_base_url(&stored.base_url)
        .is_ok_and(|stored_base_url| stored_base_url.as_str() == base_url.as_str());

    if !same_base_url {
        return Err(ApiError::api_key_required(
            "api_key is required when base_url changes (a stored key is only sent to the address it was entered for)",
        ));
    }

    Ok(api_key.clone())
}

/// The agent's definition: baibot's default openai-compatible configuration, for text generation only,
/// restricted to public addresses (see `crate::vibedev::net`), with [`AssistantMeta`] alongside.
fn agent_definition(
    input: &PutInput,
    api_key: &ApiKey,
    meta: &AssistantMeta,
) -> Result<AgentDefinition, ApiError> {
    let mut config = openai_compat::default_config();

    config.base_url = input.base_url.as_str().to_owned();
    config.api_key = Some(api_key.expose().to_owned());
    config.public_network_only = true;
    config.speech_to_text = None;
    config.text_to_speech = None;
    config.image_generation = None;

    let text_generation = config.text_generation.as_mut().ok_or_else(|| {
        ApiError::internal("The default agent configuration lacks text generation")
    })?;

    text_generation.model_id = input.model.clone();

    // Otherwise, the default prompt stays.
    if !input.system_prompt.is_empty() {
        text_generation.prompt = Some(input.system_prompt.clone());
    }

    let build_failed = |_| ApiError::internal("Failed to build the agent configuration");

    let mut config = serde_yaml_ng::to_value(&config).map_err(build_failed)?;
    let meta = serde_yaml_ng::to_value(meta).map_err(build_failed)?;

    let serde_yaml_ng::Value::Mapping(mapping) = &mut config else {
        return Err(ApiError::internal(
            "The agent configuration is not a mapping",
        ));
    };
    mapping.insert(serde_yaml_ng::Value::String(META_KEY.to_owned()), meta);

    Ok(AgentDefinition::new(
        AGENT_ID.to_owned(),
        AgentProvider::OpenAICompat,
        config,
    ))
}

/// The room access rule for the given access.
pub fn access_rule(access: Access) -> AccessRule {
    AccessRule {
        id: RULE_ID.to_owned(),
        effect: RuleEffect::Allow,
        permissions: vec![Permission::TextGeneration.name().to_owned()],
        users: vec![ALL_USERS_PATTERN.to_owned()],
        min_power_level: match access {
            Access::Everyone => None,
            Access::Admins => Some(ADMIN_POWER_LEVEL),
        },
    }
}

/// Sets up the assistant in the room's configuration (replacing an earlier one).
fn apply(
    room_config: &mut RoomConfig,
    definition: AgentDefinition,
    access: Access,
    reply_style: ReplyStyle,
) {
    room_config.agents.retain(|agent| agent.id != AGENT_ID);
    room_config.agents.push(definition);

    room_config
        .settings
        .handler
        .set_by_purpose(AgentPurpose::TextGeneration, Some(agent_identifier()));

    room_config.access.rules.retain(|rule| rule.id != RULE_ID);
    room_config.access.rules.push(access_rule(access));

    room_config.settings.text_generation.reply_style = Some(reply_style.into());
}

/// Removes the assistant (agent, handler setting and access rule), returning whether there was anything to remove.
fn remove(room_config: &mut RoomConfig) -> bool {
    let agents_before = room_config.agents.len();
    room_config.agents.retain(|agent| agent.id != AGENT_ID);
    let mut changed = room_config.agents.len() != agents_before;

    let identifier = agent_identifier();
    if room_config
        .settings
        .handler
        .get_by_purpose(AgentPurpose::TextGeneration)
        .as_deref()
        == Some(identifier.as_str())
    {
        room_config
            .settings
            .handler
            .set_by_purpose(AgentPurpose::TextGeneration, None);
        changed = true;
    }

    let rules_before = room_config.access.rules.len();
    room_config.access.rules.retain(|rule| rule.id != RULE_ID);
    changed |= room_config.access.rules.len() != rules_before;

    changed
}

/// The room's assistant configuration as room admins see it (never with the key).
fn admin_view(context: &RoomConfigContext) -> AdminView {
    let reply_style = context.text_generation_reply_style().into();

    let Some(stored) = stored_agent(&context.room_config) else {
        return AdminView {
            configured: false,
            joined: true,
            base_url: None,
            model: None,
            system_prompt: None,
            access: None,
            reply_style,
            key_set: false,
            key_set_by: None,
            updated_at: None,
        };
    };

    let access = context
        .room_config
        .access
        .rules
        .iter()
        .find(|rule| rule.id == RULE_ID)
        .map(|rule| {
            if rule
                .min_power_level
                .is_some_and(|level| level >= ADMIN_POWER_LEVEL)
            {
                Access::Admins
            } else {
                Access::Everyone
            }
        });

    AdminView {
        configured: true,
        joined: true,
        base_url: Some(stored.base_url),
        model: Some(stored.model),
        system_prompt: Some(
            stored
                .prompt
                .filter(|prompt| prompt != crate::agent::default_prompt())
                .unwrap_or_default(),
        ),
        access,
        reply_style,
        key_set: stored.api_key.is_some(),
        key_set_by: stored.meta.key_set_by,
        updated_at: stored.meta.updated_at,
    }
}
