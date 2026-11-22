//! The `vibedev` section of the configuration file (and the `BAIBOT_VIBEDEV_*` environment variables).

use std::net::SocketAddr;

use serde::{Deserialize, Deserializer};

use super::label::{AiLabel, LabelPosition, MAX_TEXT_CHARS};

pub const BAIBOT_VIBEDEV_INTERNAL_API_LISTEN_ADDRESS: &str =
    "BAIBOT_VIBEDEV_INTERNAL_API_LISTEN_ADDRESS";
pub const BAIBOT_VIBEDEV_INTERNAL_API_SECRET: &str = "BAIBOT_VIBEDEV_INTERNAL_API_SECRET";
pub const BAIBOT_VIBEDEV_AI_LABEL_TEXT: &str = "BAIBOT_VIBEDEV_AI_LABEL_TEXT";
pub const BAIBOT_VIBEDEV_AI_LABEL_POSITION: &str = "BAIBOT_VIBEDEV_AI_LABEL_POSITION";
pub const BAIBOT_VIBEDEV_SOURCE_CODE_URL: &str = "BAIBOT_VIBEDEV_SOURCE_CODE_URL";

/// The prefix of the environment variables handled here.
pub const ENV_PREFIX: &str = "BAIBOT_VIBEDEV_";

/// Where the source code of this fork is (AGPL-3.0 section 13: users interacting with it over a network
/// are offered its source).
pub const DEFAULT_SOURCE_CODE_URL: &str = "https://github.com/VibeDev-Si/vibedev-ai-bot";

/// The shortest shared secret accepted for the internal API.
pub const MIN_SECRET_LENGTH: usize = 32;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VibeDevConfig {
    #[serde(default)]
    pub internal_api: InternalApiConfig,

    #[serde(default)]
    pub ai_label: AiLabelConfig,

    /// Shown in the bot's introduction and help. Empty means [`DEFAULT_SOURCE_CODE_URL`].
    #[serde(default)]
    pub source_code_url: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InternalApiConfig {
    /// The address (`IP:port`) to listen on. Empty (or missing) turns the internal API off.
    ///
    /// It must only be reachable from the services which configure the bot (e.g. a container network),
    /// never from the internet.
    #[serde(default)]
    pub listen_address: Option<String>,

    /// The shared secret callers send as `Authorization: Bearer <secret>`.
    #[serde(default)]
    pub secret: Option<Secret>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AiLabelConfig {
    /// The label (default: `【AI 生成】`). It can be changed, but not removed: empty means the default.
    #[serde(default)]
    pub text: Option<String>,

    /// Where text answers get it: `start` (default) or `end`.
    #[serde(default)]
    pub position: LabelPosition,
}

/// A secret which never shows up in `Debug` output (and thus in logs).
#[derive(Clone, Default)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: String) -> Self {
        Self(value)
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.0.is_empty() {
            "Secret(empty)"
        } else {
            "Secret(REDACTED)"
        })
    }
}

impl<'de> Deserialize<'de> for Secret {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(Self)
    }
}

impl VibeDevConfig {
    /// Applies a `BAIBOT_VIBEDEV_*` environment variable. Unknown ones are errors (likely typos).
    pub fn apply_env_override(&mut self, key: &str, value: String) -> anyhow::Result<()> {
        match key {
            BAIBOT_VIBEDEV_INTERNAL_API_LISTEN_ADDRESS => {
                self.internal_api.listen_address = Some(value);
            }
            BAIBOT_VIBEDEV_INTERNAL_API_SECRET => {
                self.internal_api.secret = Some(Secret::new(value));
            }
            BAIBOT_VIBEDEV_AI_LABEL_TEXT => {
                self.ai_label.text = Some(value);
            }
            BAIBOT_VIBEDEV_AI_LABEL_POSITION => {
                self.ai_label.position = LabelPosition::from_str(&value).ok_or_else(|| {
                    anyhow::anyhow!("{key} must be `start` or `end`, not `{value}`")
                })?;
            }
            BAIBOT_VIBEDEV_SOURCE_CODE_URL => {
                self.source_code_url = Some(value);
            }
            _ => {
                return Err(anyhow::anyhow!(
                    "Unknown environment variable {key}. Known ones: {}",
                    [
                        BAIBOT_VIBEDEV_INTERNAL_API_LISTEN_ADDRESS,
                        BAIBOT_VIBEDEV_INTERNAL_API_SECRET,
                        BAIBOT_VIBEDEV_AI_LABEL_TEXT,
                        BAIBOT_VIBEDEV_AI_LABEL_POSITION,
                        BAIBOT_VIBEDEV_SOURCE_CODE_URL,
                    ]
                    .join(", ")
                ));
            }
        }

        Ok(())
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        self.internal_api_listen_address()?;

        if let Some(text) = &self.ai_label.text
            && text.trim().chars().count() > MAX_TEXT_CHARS
        {
            return Err(anyhow::anyhow!(
                "vibedev.ai_label.text ({BAIBOT_VIBEDEV_AI_LABEL_TEXT}) must be at most {MAX_TEXT_CHARS} characters long"
            ));
        }

        let source_code_url = self.source_code_url();
        if !source_code_url.starts_with("https://") || source_code_url.contains(char::is_whitespace)
        {
            return Err(anyhow::anyhow!(
                "vibedev.source_code_url ({BAIBOT_VIBEDEV_SOURCE_CODE_URL}) must be an https:// URL"
            ));
        }

        Ok(())
    }

    /// Where the internal API listens (with its secret), or `None` if it's turned off.
    pub fn internal_api_listen_address(&self) -> anyhow::Result<Option<(SocketAddr, Secret)>> {
        let Some(listen_address) = self
            .internal_api
            .listen_address
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return Ok(None);
        };

        let address: SocketAddr = listen_address.parse().map_err(|_| {
            anyhow::anyhow!(
                "vibedev.internal_api.listen_address ({BAIBOT_VIBEDEV_INTERNAL_API_LISTEN_ADDRESS}) must be an IP:port address (e.g. 0.0.0.0:9101), not `{listen_address}`"
            )
        })?;

        let secret = self.internal_api.secret.clone().unwrap_or_default();
        if secret.expose().len() < MIN_SECRET_LENGTH
            || secret
                .expose()
                .contains(|c: char| c.is_whitespace() || c.is_control())
        {
            return Err(anyhow::anyhow!(
                "vibedev.internal_api.secret ({BAIBOT_VIBEDEV_INTERNAL_API_SECRET}) must be set to at least {MIN_SECRET_LENGTH} characters (without spaces) when the internal API is enabled"
            ));
        }

        Ok(Some((address, secret)))
    }

    pub fn ai_label(&self) -> AiLabel {
        AiLabel::new(
            self.ai_label.text.as_deref().unwrap_or_default(),
            self.ai_label.position,
        )
    }

    pub fn source_code_url(&self) -> String {
        self.source_code_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(DEFAULT_SOURCE_CODE_URL)
            .to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_SOURCE_CODE_URL, Secret, VibeDevConfig};
    use crate::vibedev::label::{DEFAULT_TEXT, LabelPosition};

    const SECRET: &str = "0123456789abcdef0123456789abcdef";

    fn parse(yaml: &str) -> VibeDevConfig {
        serde_yaml_ng::from_str(yaml).expect("valid yaml")
    }

    #[test]
    fn everything_is_optional_and_the_api_is_off_by_default() {
        let config = parse("{}");
        config.validate().unwrap();

        assert!(config.internal_api_listen_address().unwrap().is_none());
        assert_eq!(config.ai_label().text(), DEFAULT_TEXT);
        assert_eq!(config.ai_label().position(), LabelPosition::Start);
        assert_eq!(config.source_code_url(), DEFAULT_SOURCE_CODE_URL);
    }

    #[test]
    fn the_api_needs_a_strong_secret() {
        let mut config = parse("internal_api:\n  listen_address: 0.0.0.0:9101\n");
        assert!(config.validate().is_err());

        config.internal_api.secret = Some(Secret::new("short".to_owned()));
        assert!(config.validate().is_err());

        config.internal_api.secret = Some(Secret::new(format!("{SECRET} x")));
        assert!(config.validate().is_err());

        config.internal_api.secret = Some(Secret::new(SECRET.to_owned()));
        config.validate().unwrap();
        let (address, secret) = config.internal_api_listen_address().unwrap().unwrap();
        assert_eq!(address.to_string(), "0.0.0.0:9101");
        assert_eq!(secret.expose(), SECRET);

        config.internal_api.listen_address = Some("localhost:9101".to_owned());
        assert!(config.validate().is_err());
    }

    #[test]
    fn environment_variables_override_the_file() {
        let mut config = parse("{}");

        for (key, value) in [
            ("BAIBOT_VIBEDEV_INTERNAL_API_LISTEN_ADDRESS", "[::]:9101"),
            ("BAIBOT_VIBEDEV_INTERNAL_API_SECRET", SECRET),
            ("BAIBOT_VIBEDEV_AI_LABEL_TEXT", "AI"),
            ("BAIBOT_VIBEDEV_AI_LABEL_POSITION", "end"),
            (
                "BAIBOT_VIBEDEV_SOURCE_CODE_URL",
                "https://example.com/source",
            ),
        ] {
            config.apply_env_override(key, value.to_owned()).unwrap();
        }

        config.validate().unwrap();
        assert!(config.internal_api_listen_address().unwrap().is_some());
        assert_eq!(config.ai_label().text(), "AI");
        assert_eq!(config.ai_label().position(), LabelPosition::End);
        assert_eq!(config.source_code_url(), "https://example.com/source");

        assert!(
            config
                .apply_env_override("BAIBOT_VIBEDEV_AI_LABEL_POSITION", "middle".to_owned())
                .is_err()
        );
        assert!(
            config
                .apply_env_override("BAIBOT_VIBEDEV_INTERNAL_API_SECRETS", SECRET.to_owned())
                .is_err()
        );
    }

    #[test]
    fn secrets_stay_out_of_debug_output() {
        let mut config = parse("{}");
        config.internal_api.secret = Some(Secret::new(SECRET.to_owned()));

        let debug = format!("{config:?}");
        assert!(!debug.contains(SECRET), "{debug}");
        assert!(debug.contains("REDACTED"));
    }

    #[test]
    fn unknown_keys_are_rejected() {
        assert!(serde_yaml_ng::from_str::<VibeDevConfig>("internal_api:\n  secrets: x\n").is_err());
    }

    #[test]
    fn the_deployment_sample_is_valid_and_grants_no_management() {
        let mut config: crate::entity::cfg::Config =
            serde_yaml_ng::from_str(include_str!("../../etc/vibedev/config.sample.yml"))
                .expect("the deployment sample should parse");

        // Secrets come from environment variables.
        assert!(config.vibedev.validate().is_err());
        config
            .vibedev
            .apply_env_override(super::BAIBOT_VIBEDEV_INTERNAL_API_SECRET, SECRET.to_owned())
            .unwrap();

        config.vibedev.validate().unwrap();
        config.access.validate().unwrap();
        config.initial_global_config.clone().validate().unwrap();

        assert_eq!(config.user.mxid_localpart, "vibedev-ai");
        assert_eq!(config.user.name, "VibeDev AI 助手");
        assert!(config.user.password.is_none());
        assert!(config.user.encryption.recovery_passphrase.is_none());
        assert!(config.persistence.session_encryption_key.is_none());
        assert!(config.persistence.config_encryption_key.is_none());
        assert!(config.agents.static_definitions.is_empty());
        assert!(
            config
                .vibedev
                .internal_api_listen_address()
                .unwrap()
                .is_some()
        );
        assert_eq!(config.vibedev.ai_label().text(), DEFAULT_TEXT);
        assert_eq!(config.vibedev.source_code_url(), DEFAULT_SOURCE_CODE_URL);

        // Global rules only let people invite the bot: nobody may manage agents, settings or access in chat.
        assert!(!config.initial_global_config.access_rules.is_empty());
        for rule in &config.initial_global_config.access_rules {
            assert_eq!(rule.permissions, ["invite"]);
        }

        // Upstream needs an administrator pattern; the sample's matches nobody (a reserved, unusable domain).
        assert_eq!(config.access.admin_patterns, ["@nobody:admin.invalid"]);
        let admins = config.access.admin_pattern_regexes().unwrap();
        for user_id in [
            "@admin:matrix.example.com",
            "@nobody:example.com",
            "@ops:admin.example",
        ] {
            assert!(!mxidwc::match_user_id(user_id, &admins), "{user_id}");
        }
    }

    #[test]
    fn the_source_code_url_must_be_https() {
        let mut config = parse("source_code_url: http://example.com\n");
        assert!(config.validate().is_err());

        config.source_code_url = Some(" ".to_owned());
        config.validate().unwrap();
        assert_eq!(config.source_code_url(), DEFAULT_SOURCE_CODE_URL);
    }
}
