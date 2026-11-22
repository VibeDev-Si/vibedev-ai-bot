//! Tells what went wrong when talking to a model service (the ping and model lists), without passing on what it
//! answered: errors only carry the HTTP status, never the response body (nor, thus, anything echoing the key).

use std::time::Duration;

use crate::vibedev::net::UrlError;

use super::types::ApiError;

/// How long the bot waits for a model service.
pub const UPSTREAM_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpstreamFailure {
    /// The key was refused (401, 403, or a 4xx saying so).
    InvalidApiKey { status: u16 },
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

impl From<UpstreamFailure> for ApiError {
    fn from(failure: UpstreamFailure) -> Self {
        match failure {
            UpstreamFailure::InvalidApiKey { status } => ApiError::invalid_api_key(status),
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
        .find_map(|cause| cause.downcast_ref::<reqwest::Error>())
    {
        return classify_reqwest_error(err);
    }

    // The openai-compat provider turns error responses into errors like
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
    use super::{UpstreamFailure, classify_error, classify_status};
    use crate::vibedev::api::types::ApiError;
    use crate::vibedev::net::UrlError;

    /// How the openai-compat provider words errors made from responses (see its `error_from_body`).
    fn provider_error(status: &str, message: &str) -> anyhow::Error {
        anyhow::anyhow!(
            "The OpenAI-compat chat completion API request failed ({status}): {message}"
        )
    }

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
}
