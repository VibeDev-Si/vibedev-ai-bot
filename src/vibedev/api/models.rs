//! Lists the models of an OpenAI-compatible API (`GET {base_url}/models`), for the settings page.

use serde::Deserialize;

use crate::vibedev::net::{self, BaseUrl};
use crate::vibedev::redact;

use super::types::ApiKey;
use super::upstream::{self, UPSTREAM_TIMEOUT, UpstreamFailure};

/// The largest response read (model lists of large providers are a few hundred kilobytes).
pub const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

/// The most model ids returned.
pub const MAX_MODELS: usize = 500;

/// Longer ids are left out.
const MAX_MODEL_ID_CHARS: usize = 256;

pub async fn fetch(base_url: &BaseUrl, api_key: &ApiKey) -> Result<Vec<String>, UpstreamFailure> {
    let client = net::hardened_client(reqwest::Client::builder().timeout(UPSTREAM_TIMEOUT));

    let request = async {
        let mut response = client
            .get(base_url.endpoint("models"))
            .bearer_auth(api_key.expose())
            .header(reqwest::header::ACCEPT, "application/json")
            .send()
            .await
            .map_err(|err| log_failure(base_url, api_key, &err))?;

        let status = response.status();

        let mut body = LimitedBody::new(MAX_RESPONSE_BYTES);
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|err| log_failure(base_url, api_key, &err))?
        {
            body.push(&chunk)
                .map_err(|_| UpstreamFailure::InvalidResponse)?;
        }

        if !status.is_success() {
            let failure =
                upstream::classify_response(status.as_u16(), &String::from_utf8_lossy(&body.bytes));

            tracing::info!(
                base_url = base_url.as_str(),
                status = status.as_u16(),
                ?failure,
                "The model service refused to list models"
            );

            return Err(failure);
        }

        parse_models(&body.bytes).ok_or(UpstreamFailure::InvalidResponse)
    };

    // reqwest's timeout covers the whole request too; this one also covers the resolver.
    tokio::time::timeout(UPSTREAM_TIMEOUT, request)
        .await
        .unwrap_or(Err(UpstreamFailure::Timeout))
}

fn log_failure(base_url: &BaseUrl, api_key: &ApiKey, err: &reqwest::Error) -> UpstreamFailure {
    let failure = upstream::classify_reqwest_error(err);

    tracing::info!(
        base_url = base_url.as_str(),
        ?failure,
        err = %redact::sanitize_message(&format!("{err:#}"), &[api_key.expose()], 300),
        "Failed to request the model list"
    );

    failure
}

/// Collects a response body up to a size limit.
pub struct LimitedBody {
    max: usize,
    bytes: Vec<u8>,
}

#[derive(Debug, PartialEq)]
pub struct TooLarge;

impl LimitedBody {
    pub fn new(max: usize) -> Self {
        Self { max, bytes: vec![] }
    }

    pub fn push(&mut self, chunk: &[u8]) -> Result<(), TooLarge> {
        if self.bytes.len() + chunk.len() > self.max {
            return Err(TooLarge);
        }

        self.bytes.extend_from_slice(chunk);
        Ok(())
    }
}

/// Reads model ids from an OpenAI-style list (`{"data": [{"id": "..."}]}`): sorted, without duplicates,
/// without unusable ids, and at most [`MAX_MODELS`]. `None` if it isn't such a list.
pub fn parse_models(body: &[u8]) -> Option<Vec<String>> {
    #[derive(Deserialize)]
    struct List {
        data: Vec<serde_json::Value>,
    }

    let list: List = serde_json::from_slice(body).ok()?;

    let mut models: Vec<String> = list
        .data
        .iter()
        .filter_map(|model| model.get("id")?.as_str())
        .map(str::trim)
        .filter(|id| {
            !id.is_empty()
                && id.chars().count() <= MAX_MODEL_ID_CHARS
                && !id.contains(char::is_control)
        })
        .map(str::to_owned)
        .collect();

    models.sort();
    models.dedup();
    models.truncate(MAX_MODELS);

    Some(models)
}

#[cfg(test)]
mod tests {
    use super::{LimitedBody, MAX_MODELS, TooLarge, parse_models};
    use crate::vibedev::api::upstream::error_message;

    #[test]
    fn model_lists_are_parsed_sorted_and_cleaned() {
        let body = br#"{"object": "list", "data": [
            {"id": "gpt-4o", "object": "model"},
            {"id": "deepseek-chat"},
            {"id": "gpt-4o"},
            {"id": ""},
            {"id": "bad\u0007id"},
            {"name": "no id"},
            "not an object"
        ]}"#;

        assert_eq!(
            parse_models(body),
            Some(vec!["deepseek-chat".to_owned(), "gpt-4o".to_owned()])
        );

        assert_eq!(parse_models(br#"{"data": []}"#), Some(vec![]));
        assert_eq!(parse_models(br#"{"models": ["a"]}"#), None);
        assert_eq!(parse_models(b"<html>"), None);
    }

    #[test]
    fn at_most_500_models_are_returned() {
        let data: Vec<serde_json::Value> = (0..800)
            .map(|index| serde_json::json!({"id": format!("model-{index:04}")}))
            .collect();
        let body = serde_json::to_vec(&serde_json::json!({ "data": data })).unwrap();

        let models = parse_models(&body).unwrap();
        assert_eq!(models.len(), MAX_MODELS);
        assert_eq!(models[0], "model-0000");
    }

    #[test]
    fn bodies_are_limited() {
        let mut body = LimitedBody::new(10);
        assert_eq!(body.push(b"12345"), Ok(()));
        assert_eq!(body.push(b"67890"), Ok(()));
        assert_eq!(body.push(b"1"), Err(TooLarge));
    }

    #[test]
    fn error_messages_are_found() {
        assert_eq!(
            error_message(r#"{"error": {"message": "Invalid key"}}"#),
            Some("Invalid key".to_owned())
        );
        assert_eq!(
            error_message(r#"{"detail": "Nope"}"#),
            Some("Nope".to_owned())
        );
        assert_eq!(error_message("plain"), None);
    }
}
