//! Tells what went wrong when talking to a model service (the ping and model lists), without passing on what it
//! answered: errors only carry the HTTP status and what was recognized in the answer, never the response body (nor,
//! thus, anything echoing the key).

use std::time::Duration;

use crate::vibedev::net::UrlError;

use super::types::ApiError;

/// How long the bot waits for a model service.
pub const UPSTREAM_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpstreamFailure {
    /// The key was refused (401, 403, or a 4xx saying so).
    InvalidApiKey { status: u16 },
    /// The key is valid, but the VibeDev gateway doesn't let third-party applications (like this bot) use it.
    KeyNotPermitted(KeyNotPermitted),
    /// The model doesn't exist (a 4xx saying so).
    ModelNotFound { status: u16 },
    /// No answer in time ([`UPSTREAM_TIMEOUT`]), or the service's own gateway timed out (408, 504, 524).
    Timeout,
    /// The host turned out to resolve to a non-public address when connecting.
    BaseUrlNotAllowed(UrlError),
    /// Another error status.
    Status(u16),
    /// No connection could be made.
    Unreachable,
    /// The answer wasn't what an OpenAI-compatible API answers.
    InvalidResponse,
}

/// Why the VibeDev gateway refuses a key for use by third-party applications.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyNotPermitted {
    /// 401 with `MISSING_VIBEDEV_SIGNATURE`: the key only works with VibeDev's signature (in VibeDev's own apps).
    MissingVibeDevSignature,
    /// 403 with `GROUP_PUBLIC_API_ACCESS_DISABLED`: the key's group may not use the public API.
    GroupPublicApiAccessDisabled,
}

impl KeyNotPermitted {
    /// The gateway's error code (what's looked for in its answers).
    pub fn code(self) -> &'static str {
        match self {
            Self::MissingVibeDevSignature => "MISSING_VIBEDEV_SIGNATURE",
            Self::GroupPublicApiAccessDisabled => "GROUP_PUBLIC_API_ACCESS_DISABLED",
        }
    }

    fn status(self) -> u16 {
        match self {
            Self::MissingVibeDevSignature => 401,
            Self::GroupPublicApiAccessDisabled => 403,
        }
    }
}

impl From<UpstreamFailure> for ApiError {
    fn from(failure: UpstreamFailure) -> Self {
        match failure {
            UpstreamFailure::InvalidApiKey { status } => ApiError::invalid_api_key(status),
            UpstreamFailure::KeyNotPermitted(reason) => ApiError::key_not_permitted(reason),
            UpstreamFailure::ModelNotFound { status } => ApiError::model_not_found(status),
            UpstreamFailure::Timeout => ApiError::upstream_timeout(UPSTREAM_TIMEOUT.as_secs()),
            UpstreamFailure::BaseUrlNotAllowed(err) => ApiError::base_url_not_allowed(err),
            UpstreamFailure::Status(status) => {
                ApiError::upstream_error(format!("The model service answered with HTTP {status}"))
            }
            UpstreamFailure::Unreachable => {
                ApiError::upstream_error("Could not connect to the model service")
            }
            UpstreamFailure::InvalidResponse => ApiError::upstream_error(
                "The model service did not answer like an OpenAI-compatible API",
            ),
        }
    }
}

/// Finds the VibeDev gateway's refusals of keys for third-party use in an error answer:
/// 401 with `MISSING_VIBEDEV_SIGNATURE`, and 403 with `GROUP_PUBLIC_API_ACCESS_DISABLED` (anywhere in the body).
pub fn key_not_permitted(status: u16, body: &str) -> Option<KeyNotPermitted> {
    let body = body.to_ascii_uppercase();

    [
        KeyNotPermitted::MissingVibeDevSignature,
        KeyNotPermitted::GroupPublicApiAccessDisabled,
    ]
    .into_iter()
    .find(|reason| status == reason.status() && body.contains(reason.code()))
}

/// Classifies an error answer: its status, the gateway's refusals (see [`key_not_permitted`]), and keywords of its
/// error message. Nothing of the body is passed on.
pub fn classify_response(status: u16, body: &str) -> UpstreamFailure {
    match key_not_permitted(status, body) {
        Some(reason) => UpstreamFailure::KeyNotPermitted(reason),
        None => classify_status(status, &error_message(body).unwrap_or_default()),
    }
}

/// Classifies an error status, looking at (but not passing on) the service's error message.
pub fn classify_status(status: u16, message: &str) -> UpstreamFailure {
    let message = message.to_lowercase();
    let client_error = (400..500).contains(&status);

    match status {
        401 | 403 => UpstreamFailure::InvalidApiKey { status },
        408 | 504 | 524 => UpstreamFailure::Timeout,
        _ if client_error && mentions_bad_key(&message) => {
            UpstreamFailure::InvalidApiKey { status }
        }
        _ if client_error && mentions_unknown_model(&message) => {
            UpstreamFailure::ModelNotFound { status }
        }
        _ => UpstreamFailure::Status(status),
    }
}

fn mentions_bad_key(message: &str) -> bool {
    [
        "api key",
        "api_key",
        "apikey",
        "invalid key",
        "incorrect key",
        "authentication",
        "unauthorized",
        "unauthorised",
    ]
    .iter()
    .any(|phrase| message.contains(phrase))
}

fn mentions_unknown_model(message: &str) -> bool {
    message.contains("model_not_found")
        || (message.contains("model")
            && [
                "not found",
                "not exist",
                "no such",
                "unknown",
                "invalid",
                "not available",
                "not supported",
                "unsupported",
            ]
            .iter()
            .any(|phrase| message.contains(phrase)))
}

/// Finds the error message in the error body shapes OpenAI-compatible APIs use (only for classifying the error).
pub fn error_message(body: &str) -> Option<String> {
    let body: serde_json::Value = serde_json::from_str(body).ok()?;

    [
        &body["error"]["message"],
        &body["error"],
        &body["message"],
        &body["detail"],
    ]
    .into_iter()
    .find_map(|value| value.as_str())
    .map(str::to_owned)
}

/// An error answer of a model service, as the openai-compat provider reports it (its `error_from_body`).
///
/// Shows as the provider's message (as before), and keeps what classifying needs: the status, and whether the
/// VibeDev gateway refused the key for third-party use. The body itself isn't kept.
#[derive(Debug)]
pub struct ProviderError {
    status: u16,
    key_not_permitted: Option<KeyNotPermitted>,
    message: String,
}

impl ProviderError {
    pub fn new(status: u16, body: &str, message: String) -> Self {
        Self {
            status,
            key_not_permitted: key_not_permitted(status, body),
            message,
        }
    }
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ProviderError {}

/// Classifies an error of a request to a model service, e.g. of baibot's ping.
pub fn classify_error(err: &anyhow::Error) -> UpstreamFailure {
    // The resolver refused a non-public address (see `crate::vibedev::net::PublicOnlyResolver`).
    if let Some(url_error) = err
        .chain()
        .find_map(|cause| cause.downcast_ref::<UrlError>())
    {
        return UpstreamFailure::BaseUrlNotAllowed(*url_error);
    }

    if let Some(err) = err
        .chain()
        .find_map(|cause| cause.downcast_ref::<ProviderError>())
    {
        return match err.key_not_permitted {
            Some(reason) => UpstreamFailure::KeyNotPermitted(reason),
            None => classify_status(err.status, &err.message),
        };
    }

    if let Some(err) = err
        .chain()
        .find_map(|cause| cause.downcast_ref::<reqwest::Error>())
    {
        return classify_reqwest_error(err);
    }

    // Errors only worded like the openai-compat provider's:
    // "The OpenAI-compat chat completion API request failed (401 Unauthorized): <message>".
    let text = format!("{err:#}");
    match status_in_message(&text) {
        Some(status) => classify_status(status, &text),
        None => UpstreamFailure::InvalidResponse,
    }
}

pub fn classify_reqwest_error(err: &reqwest::Error) -> UpstreamFailure {
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(err);
    while let Some(cause) = source {
        if let Some(url_error) = cause.downcast_ref::<UrlError>() {
            return UpstreamFailure::BaseUrlNotAllowed(*url_error);
        }
        source = cause.source();
    }

    if err.is_timeout() {
        UpstreamFailure::Timeout
    } else if let Some(status) = err.status() {
        classify_status(status.as_u16(), "")
    } else if err.is_decode() || err.is_body() {
        UpstreamFailure::InvalidResponse
    } else {
        UpstreamFailure::Unreachable
    }
}

/// Finds the status in "... request failed (401 Unauthorized) ...".
fn status_in_message(text: &str) -> Option<u16> {
    let (_, rest) = text.split_once("request failed (")?;
    let digits = rest.get(..3)?;

    if !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }

    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::{
        KeyNotPermitted, ProviderError, UpstreamFailure, classify_error, classify_response,
        classify_status, key_not_permitted,
    };
    use crate::vibedev::api::types::ApiError;
    use crate::vibedev::net::UrlError;

    /// How the openai-compat provider words errors made from responses (see its `error_from_body`).
    fn provider_error(status: &str, message: &str) -> anyhow::Error {
        anyhow::anyhow!(
            "The OpenAI-compat chat completion API request failed ({status}): {message}"
        )
    }

    const MISSING_SIGNATURE_BODY: &str = r#"{"error": {"message": "This key requires a VibeDev signature", "type": "invalid_request_error", "code": "MISSING_VIBEDEV_SIGNATURE"}}"#;

    const GROUP_DISABLED_BODY: &str = r#"{"error": {"message": "Public API access is disabled for this group", "code": "GROUP_PUBLIC_API_ACCESS_DISABLED"}}"#;

    #[test]
    fn statuses_and_messages_are_classified() {
        for (status, message, expected) in [
            (
                "401 Unauthorized",
                "Incorrect API key provided: sk-abc",
                UpstreamFailure::InvalidApiKey { status: 401 },
            ),
            (
                "403 Forbidden",
                "",
                UpstreamFailure::InvalidApiKey { status: 403 },
            ),
            (
                "400 Bad Request",
                "API key not valid. Please pass a valid API key.",
                UpstreamFailure::InvalidApiKey { status: 400 },
            ),
            (
                "404 Not Found",
                "The model `gpt-9` does not exist or you do not have access to it.",
                UpstreamFailure::ModelNotFound { status: 404 },
            ),
            (
                "400 Bad Request",
                "Model Not Exist",
                UpstreamFailure::ModelNotFound { status: 400 },
            ),
            ("404 Not Found", "Not found", UpstreamFailure::Status(404)),
            (
                "500 Internal Server Error",
                "model unavailable",
                UpstreamFailure::Status(500),
            ),
            ("504 Gateway Timeout", "", UpstreamFailure::Timeout),
            (
                "429 Too Many Requests",
                "Slow down",
                UpstreamFailure::Status(429),
            ),
        ] {
            assert_eq!(
                classify_error(&provider_error(status, message)),
                expected,
                "{status} {message}"
            );
        }

        assert_eq!(
            classify_error(&anyhow::anyhow!(
                "No response messages choices were returned"
            )),
            UpstreamFailure::InvalidResponse
        );

        assert_eq!(
            classify_status(401, ""),
            UpstreamFailure::InvalidApiKey { status: 401 }
        );
    }

    #[test]
    fn the_gateways_refusals_of_keys_for_third_party_use_are_recognized() {
        assert_eq!(
            key_not_permitted(401, MISSING_SIGNATURE_BODY),
            Some(KeyNotPermitted::MissingVibeDevSignature)
        );
        assert_eq!(
            key_not_permitted(403, GROUP_DISABLED_BODY),
            Some(KeyNotPermitted::GroupPublicApiAccessDisabled)
        );

        // Anywhere in the body, whatever its shape or case.
        assert_eq!(
            key_not_permitted(401, "error: missing_vibedev_signature"),
            Some(KeyNotPermitted::MissingVibeDevSignature)
        );
        assert_eq!(
            key_not_permitted(403, r#"{"code": "GROUP_PUBLIC_API_ACCESS_DISABLED"}"#),
            Some(KeyNotPermitted::GroupPublicApiAccessDisabled)
        );

        // Only with the status each comes with.
        assert_eq!(key_not_permitted(403, MISSING_SIGNATURE_BODY), None);
        assert_eq!(key_not_permitted(401, GROUP_DISABLED_BODY), None);
        assert_eq!(key_not_permitted(400, MISSING_SIGNATURE_BODY), None);
        assert_eq!(
            key_not_permitted(401, r#"{"error": {"message": "Invalid API key"}}"#),
            None
        );

        // Answers classified as such.
        assert_eq!(
            classify_response(401, MISSING_SIGNATURE_BODY),
            UpstreamFailure::KeyNotPermitted(KeyNotPermitted::MissingVibeDevSignature)
        );
        assert_eq!(
            classify_response(403, GROUP_DISABLED_BODY),
            UpstreamFailure::KeyNotPermitted(KeyNotPermitted::GroupPublicApiAccessDisabled)
        );
        assert_eq!(
            classify_response(401, r#"{"error": {"message": "Invalid API key"}}"#),
            UpstreamFailure::InvalidApiKey { status: 401 }
        );
        assert_eq!(
            classify_response(
                404,
                r#"{"error": {"message": "The model `x` does not exist"}}"#
            ),
            UpstreamFailure::ModelNotFound { status: 404 }
        );

        // Errors of the openai-compat provider keep what was recognized.
        let error = |status, body| {
            anyhow::Error::new(ProviderError::new(
                status,
                body,
                format!("The OpenAI-compat chat completion API request failed ({status}): ..."),
            ))
        };
        assert_eq!(
            classify_error(&error(401, MISSING_SIGNATURE_BODY)),
            UpstreamFailure::KeyNotPermitted(KeyNotPermitted::MissingVibeDevSignature)
        );
        assert_eq!(
            classify_error(&error(403, GROUP_DISABLED_BODY)),
            UpstreamFailure::KeyNotPermitted(KeyNotPermitted::GroupPublicApiAccessDisabled)
        );
        assert_eq!(
            classify_error(&error(403, "{}")),
            UpstreamFailure::InvalidApiKey { status: 403 }
        );
    }

    #[test]
    fn refused_keys_get_their_own_error_saying_why() {
        let missing_signature = ApiError::from(UpstreamFailure::KeyNotPermitted(
            KeyNotPermitted::MissingVibeDevSignature,
        ));
        let group_disabled = ApiError::from(UpstreamFailure::KeyNotPermitted(
            KeyNotPermitted::GroupPublicApiAccessDisabled,
        ));

        for error in [&missing_signature, &group_disabled] {
            assert_eq!(error.errcode, "VD_KEY_NOT_PERMITTED");
            assert_eq!(error.status.as_u16(), 422);
        }

        assert!(
            missing_signature.error.contains("VibeDev signature"),
            "{}",
            missing_signature.error
        );
        assert!(
            missing_signature
                .error
                .contains("MISSING_VIBEDEV_SIGNATURE"),
            "{}",
            missing_signature.error
        );
        assert!(
            group_disabled.error.contains("group"),
            "{}",
            group_disabled.error
        );
        assert!(
            group_disabled
                .error
                .contains("GROUP_PUBLIC_API_ACCESS_DISABLED"),
            "{}",
            group_disabled.error
        );
        assert_ne!(missing_signature.error, group_disabled.error);

        // Nothing of what the gateway answered is passed on.
        assert!(!missing_signature.error.contains("type"));
        assert!(
            !group_disabled
                .error
                .contains("Public API access is disabled")
        );
    }

    #[test]
    fn errors_say_nothing_the_service_answered() {
        let key = "sk-echoed-0123456789";
        let failure = classify_error(&provider_error(
            "401 Unauthorized",
            &format!("Incorrect API key provided: {key}. <html>body</html>"),
        ));

        let error = ApiError::from(failure);
        assert_eq!(error.errcode, "VD_INVALID_API_KEY");
        assert_eq!(error.status.as_u16(), 422);
        assert!(!error.error.contains(key), "{}", error.error);
        assert!(!error.error.contains("html"), "{}", error.error);

        assert_eq!(
            ApiError::from(UpstreamFailure::ModelNotFound { status: 404 }).errcode,
            "VD_MODEL_NOT_FOUND"
        );
        assert_eq!(
            ApiError::from(UpstreamFailure::Timeout).status.as_u16(),
            504
        );
        assert_eq!(
            ApiError::from(UpstreamFailure::Timeout).errcode,
            "VD_UPSTREAM_TIMEOUT"
        );
        assert_eq!(
            ApiError::from(UpstreamFailure::Status(500)).errcode,
            "VD_UPSTREAM_ERROR"
        );
        assert_eq!(
            ApiError::from(UpstreamFailure::Unreachable).status.as_u16(),
            502
        );
        assert_eq!(
            ApiError::from(UpstreamFailure::BaseUrlNotAllowed(
                UrlError::NonPublicAddress
            ))
            .errcode,
            "VD_BASE_URL_NOT_ALLOWED"
        );
    }

    #[tokio::test]
    async fn refused_resolutions_are_found_in_request_errors() {
        // A real request error whose cause is the resolver refusing a loopback address.
        let client = crate::vibedev::net::hardened_client(reqwest::Client::builder());
        let err = client
            .get("https://localhost:9/v1/models")
            .send()
            .await
            .expect_err("loopback must be refused");

        assert_eq!(
            classify_error(&anyhow::Error::new(err)),
            UpstreamFailure::BaseUrlNotAllowed(UrlError::NonPublicAddress)
        );
    }

    /// Serves one HTTP answer (on a loopback address), returning its base URL.
    async fn answer_once(status_line: &'static str, body: &'static str) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();

        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();

            // Reads the request (headers and body), so that the answer isn't cut off by a reset.
            let mut request = Vec::new();
            let mut buffer = [0u8; 4096];
            loop {
                let read = stream.read(&mut buffer).await.unwrap_or(0);
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);

                let text = String::from_utf8_lossy(&request).to_lowercase();
                if let Some(headers_end) = text.find("\r\n\r\n") {
                    let content_length = text[..headers_end]
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length:"))
                        .and_then(|value| value.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    if request.len() >= headers_end + 4 + content_length {
                        break;
                    }
                }
            }

            let answer = format!(
                "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(answer.as_bytes()).await;
            let _ = stream.shutdown().await;
        });

        format!("http://{address}/v1")
    }

    /// The whole way: baibot's ping (of an openai-compatible agent) against a gateway refusing the key.
    #[tokio::test]
    async fn the_ping_reports_keys_the_gateway_refuses_for_third_party_use() {
        use crate::agent::provider::ControllerTrait;
        use crate::agent::provider::openai_compat;

        for (status_line, body, expected) in [
            (
                "401 Unauthorized",
                MISSING_SIGNATURE_BODY,
                UpstreamFailure::KeyNotPermitted(KeyNotPermitted::MissingVibeDevSignature),
            ),
            (
                "403 Forbidden",
                GROUP_DISABLED_BODY,
                UpstreamFailure::KeyNotPermitted(KeyNotPermitted::GroupPublicApiAccessDisabled),
            ),
            (
                "401 Unauthorized",
                r#"{"error": {"message": "Incorrect API key provided"}}"#,
                UpstreamFailure::InvalidApiKey { status: 401 },
            ),
        ] {
            // A plain (not public-network-only) agent, as the test server is on a loopback address.
            let mut config = openai_compat::default_config();
            config.base_url = answer_once(status_line, body).await;
            config.api_key = Some("sk-test-gateway-0123456789".to_owned());
            config.speech_to_text = None;

            let controller = openai_compat::Controller::new(config);
            let Err(err) = controller.ping().await else {
                panic!("the gateway refuses the key: {status_line} {body}");
            };

            assert_eq!(classify_error(&err), expected, "{status_line} {body}");

            // The error still reads as before.
            assert!(
                err.to_string()
                    .starts_with("The OpenAI-compat chat completion API request failed"),
                "{err}"
            );
        }
    }
}
