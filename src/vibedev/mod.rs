//! VibeDev fork additions (VibeDev AI 助手, based on etkecc/baibot, AGPL-3.0).
//!
//! Kept in this module, with small hooks elsewhere (each marked with a `VibeDev fork` comment), so that upstream
//! changes merge easily:
//!
//! - [`api`]: the internal configuration API, with which the VibeDev integration manager sets up a room's assistant
//!   (a room-local openai-compatible agent, paid for with the API key of whoever sets it up).
//! - [`label`]: the "AI-generated" label on everything models generate.
//! - [`net`]: keeps requests to user-supplied API addresses on the public internet.
//! - [`redact`]: keeps API keys out of the logs.
//! - [`source_code_notice`]: the link to this fork's source code (AGPL-3.0 section 13), shown in the bot's help.

pub mod api;
pub mod config;
pub mod label;
pub mod net;
pub mod redact;

use std::sync::OnceLock;

pub use config::VibeDevConfig;

static SOURCE_CODE_URL: OnceLock<String> = OnceLock::new();

/// Applies the configuration which needs no running bot (at startup, before anything is sent).
pub fn init(config: &VibeDevConfig) {
    label::init(config.ai_label());
    let _ = SOURCE_CODE_URL.set(config.source_code_url());
}

/// Starts the internal API if it's enabled.
pub async fn start(bot: crate::Bot, config: &VibeDevConfig) -> anyhow::Result<()> {
    match config.internal_api_listen_address()? {
        Some((address, secret)) => api::start(bot, address, secret).await,
        None => {
            tracing::info!("The VibeDev internal API is disabled (no listen address configured)");
            Ok(())
        }
    }
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
}
