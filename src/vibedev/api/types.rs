//! Requests, responses and errors of the internal API (all JSON).

use hyper::StatusCode;
use serde::{Deserialize, Deserializer, Serialize};

use crate::entity::roomconfig::TextGenerationReplyStyle;
use crate::vibedev::net::UrlError;

/// An API key, which never shows up in `Debug` output and is never serialized.
#[derive(Clone, PartialEq, Eq)]
pub struct ApiKey(String);

impl ApiKey {
    pub fn new(value: String) -> Self {
        Self(value)
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ApiKey(REDACTED)")
    }
}

impl<'de> Deserialize<'de> for ApiKey {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(Self)
    }
}

/// Who may use the assistant in the room.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Access {
    /// All room members.
    Everyone,
    /// Room members with a power level of at least 100.
    Admins,
}

/// Where the assistant answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ReplyStyle {
    /// In a thread started from the message (upstream default).
    Thread,
    /// With an in-room reply to the message.
    Reply,
}

impl From<ReplyStyle> for TextGenerationReplyStyle {
    fn from(value: ReplyStyle) -> Self {
        match value {
            ReplyStyle::Thread => Self::Thread,
            ReplyStyle::Reply => Self::Reply,
        }
    }
}

impl From<TextGenerationReplyStyle> for ReplyStyle {
    fn from(value: TextGenerationReplyStyle) -> Self {
        match value {
            TextGenerationReplyStyle::Thread => Self::Thread,
            TextGenerationReplyStyle::Reply => Self::Reply,
        }
    }
}

/// `PUT /_vibedev/v1/rooms/{room_id}/assistant`
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PutAssistantRequest {
    pub acting_user: String,
    pub base_url: String,
    /// Missing, `null` or empty: keep the stored key (only allowed while `base_url` stays the same).
    #[serde(default)]
    pub api_key: Option<ApiKey>,
    pub model: String,
    /// Missing, `null` or empty: the bot's default system prompt.
    #[serde(default)]
    pub system_prompt: Option<String>,
    pub access: Access,
    pub reply_style: ReplyStyle,
}

/// `DELETE /_vibedev/v1/rooms/{room_id}/assistant` (`acting_user` may also come in the query string).
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeleteAssistantRequest {
    #[serde(default)]
    pub acting_user: Option<String>,
}

/// `POST /_vibedev/v1/models`: either `{base_url, api_key}`, or `{room_id, acting_user}` (optionally with `base_url`)
/// for the room's stored key and address.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelsRequest {
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub api_key: Option<ApiKey>,
    #[serde(default)]
    pub room_id: Option<String>,
    #[serde(default)]
    pub acting_user: Option<String>,
}

/// What `GET` (and `PUT`) return: everything (but the key) for room admins, little for others.
#[derive(Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum AssistantView {
    Admin(AdminView),
    Limited(LimitedView),
}

/// The room's assistant configuration as room admins see it. Never contains the key.
#[derive(Debug, PartialEq, Serialize)]
pub struct AdminView {
    pub configured: bool,
    pub joined: bool,
    pub base_url: Option<String>,
    pub model: Option<String>,
    /// Empty when the bot's default system prompt is used.
    pub system_prompt: Option<String>,
    /// `null` when not configured (or if the room's access rule is missing).
    pub access: Option<Access>,
    pub reply_style: ReplyStyle,
    pub key_set: bool,
    /// Who last provided the key.
    pub key_set_by: Option<String>,
    /// When the configuration was last saved (RFC 3339, UTC).
    pub updated_at: Option<String>,
}

/// What other users see.
#[derive(Debug, PartialEq, Serialize)]
pub struct LimitedView {
    pub configured: bool,
    pub joined: bool,
}

/// What `DELETE` returns.
#[derive(Debug, Serialize)]
pub struct DeletedView {
    pub configured: bool,
}

#[derive(Debug, Serialize)]
pub struct ModelsView {
    pub models: Vec<String>,
}

/// An error response: `{"errcode": "VD_...", "error": "..."}`.
///
/// Messages are written here: they never contain keys or what model services answered.
#[derive(Debug, PartialEq, Serialize)]
pub struct ApiError {
    #[serde(skip)]
    pub status: StatusCode,
    pub errcode: &'static str,
    pub error: String,
}

impl ApiError {
    pub fn new(status: StatusCode, errcode: &'static str, error: impl Into<String>) -> Self {
        Self {
            status,
            errcode,
            error: error.into(),
        }
    }

    /// `400 VD_INVALID_REQUEST`. Also used, with their own status, for other malformed requests
    /// (unknown path, wrong method, body too large or too slow, not JSON).
    pub fn invalid_request(error: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "VD_INVALID_REQUEST", error)
    }

    pub fn invalid_request_with_status(status: StatusCode, error: impl Into<String>) -> Self {
        Self::new(status, "VD_INVALID_REQUEST", error)
    }

    pub fn base_url_not_allowed(err: UrlError) -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            "VD_BASE_URL_NOT_ALLOWED",
            err.to_string(),
        )
    }

    pub fn api_key_required(error: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "VD_API_KEY_REQUIRED", error)
    }

    pub fn unauthorized() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            "VD_UNAUTHORIZED",
            "Missing or invalid Authorization: Bearer secret",
        )
    }

    pub fn forbidden() -> Self {
        Self::new(
            StatusCode::FORBIDDEN,
            "VD_FORBIDDEN",
            "acting_user must be a member of the room with a power level of at least 100",
        )
    }

    pub fn bot_not_in_room() -> Self {
        Self::new(
            StatusCode::CONFLICT,
            "VD_BOT_NOT_IN_ROOM",
            "The bot is not in the room (add it to the room first)",
        )
    }

    pub fn invalid_api_key(status: u16) -> Self {
        Self::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "VD_INVALID_API_KEY",
            format!("The model service rejected the API key (HTTP {status})"),
        )
    }

    pub fn key_not_permitted(reason: super::upstream::KeyNotPermitted) -> Self {
        use super::upstream::KeyNotPermitted;

        let explanation = match reason {
            KeyNotPermitted::MissingVibeDevSignature => {
                "This API key only works with a VibeDev signature, so it cannot be used by third-party applications like this bot"
            }
            KeyNotPermitted::GroupPublicApiAccessDisabled => {
                "The group this API key belongs to does not allow public API access"
            }
        };

        Self::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "VD_KEY_NOT_PERMITTED",
            format!("{explanation} ({})", reason.code()),
        )
    }

    pub fn model_not_found(status: u16) -> Self {
        Self::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "VD_MODEL_NOT_FOUND",
            format!("The model service does not know this model (HTTP {status})"),
        )
    }

    pub fn upstream_error(error: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_GATEWAY, "VD_UPSTREAM_ERROR", error)
    }

    pub fn upstream_timeout(seconds: u64) -> Self {
        Self::new(
            StatusCode::GATEWAY_TIMEOUT,
            "VD_UPSTREAM_TIMEOUT",
            format!("The model service did not answer within {seconds} seconds"),
        )
    }

    /// Handling the request took too long for another reason (e.g. Matrix being slow).
    pub fn timeout() -> Self {
        Self::new(
            StatusCode::GATEWAY_TIMEOUT,
            "VD_TIMEOUT",
            "The request took too long",
        )
    }

    pub fn internal(error: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "VD_INTERNAL", error)
    }
}
