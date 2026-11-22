//! VibeDev fork additions (VibeDev AI 助手, based on etkecc/baibot, AGPL-3.0).
//!
//! Kept in this module, with small hooks elsewhere (each marked with a `VibeDev fork` comment), so that upstream
//! changes merge easily:
//!
//! - [`api`]: the internal configuration API, with which the VibeDev integration manager sets up a room's assistant
//!   (a room-local openai-compatible agent, paid for with the API key of whoever sets it up).
//! - [`departure`]: forgets a room's assistant (with the API key) as soon as the bot is no longer in the room.
//! - [`label`]: the "AI-generated" label on everything models generate.
//! - [`net`]: keeps requests to user-supplied API addresses on the public internet.
//! - [`redact`]: keeps API keys out of the logs and out of error messages posted in rooms.
//! - [`refuse_revealing_secrets`]: nobody (not even bot administrators) can have secrets shown in chat.
//! - [`source_code_notice`]: the link to this fork's source code (AGPL-3.0 section 13), shown in the bot's help.

pub mod api;
pub mod config;
pub mod departure;
pub mod label;
pub mod net;
pub mod redact;

use std::sync::OnceLock;

pub use config::VibeDevConfig;

static SOURCE_CODE_URL: OnceLock<String> = OnceLock::new();

/// What the bot answers to `agent details-revealing-secrets`.
pub const REVEALING_SECRETS_DISABLED: &str = "This bot never shows secrets (like API keys) in chat, not even to administrators. `agent details` shows an agent's configuration with secrets redacted. API keys of the AI assistant can only be replaced on its settings page.";

/// Applies the configuration which needs no running bot (at startup, before anything is sent).
pub fn init(config: &VibeDevConfig) {
    label::init(config.ai_label());
    let _ = SOURCE_CODE_URL.set(config.source_code_url());
}

/// Starts forgetting the assistants of rooms the bot leaves, and the internal API if it's enabled.
pub async fn start(bot: crate::Bot, config: &VibeDevConfig) -> anyhow::Result<()> {
    departure::attach(&bot);

    match config.internal_api_listen_address()? {
        Some((address, secret)) => api::start(bot, address, secret).await,
        None => {
            tracing::info!("The VibeDev internal API is disabled (no listen address configured)");
            Ok(())
        }
    }
}

/// Refuses showing secrets in chat: returns the refusal to send instead, if secrets were asked for.
///
/// Upstream lets agent managers (and administrators) see API keys with `agent details-revealing-secrets`.
/// Here, API keys belong to the users who set up the assistant, so nobody else may see them.
pub fn refuse_revealing_secrets(reveal_secrets: bool) -> Option<&'static str> {
    reveal_secrets.then_some(REVEALING_SECRETS_DISABLED)
}

/// Says where the source code is (shown in the bot's introduction and help).
pub fn source_code_notice() -> String {
    let url = SOURCE_CODE_URL
        .get()
        .map(String::as_str)
        .unwrap_or(config::DEFAULT_SOURCE_CODE_URL);

    format!("📜 源代码 / Source code (AGPL-3.0): [{url}]({url})")
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_source_code_notice_links_to_the_repository() {
        let notice = super::source_code_notice();
        assert!(notice.contains("AGPL-3.0"), "{notice}");
        assert!(notice.contains("https://"), "{notice}");
    }

    #[test]
    fn secrets_are_never_revealed_in_chat() {
        assert_eq!(
            super::refuse_revealing_secrets(true),
            Some(super::REVEALING_SECRETS_DISABLED)
        );
        assert_eq!(super::refuse_revealing_secrets(false), None);
    }
}
