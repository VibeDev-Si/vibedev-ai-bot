//! Keeps API keys out of the logs.
//!
//! API keys the bot handles (see [`register`]) are replaced with [`REDACTED`] in every log line, whichever code
//! logs them: provider errors may echo keys in their response bodies, and debug output of configurations
//! contains them. [`RedactingMakeWriter`] does the replacing, below the log formatter.

use std::borrow::Cow;
use std::collections::VecDeque;
use std::io;
use std::sync::{OnceLock, RwLock};

use tracing_subscriber::fmt::MakeWriter;

/// What secrets are replaced with.
pub const REDACTED: &str = "[REDACTED]";

/// Shorter secrets aren't registered: they'd turn ordinary words into [`REDACTED`].
pub const MIN_SECRET_LENGTH: usize = 8;

/// The most secrets kept (the oldest ones are forgotten first).
const MAX_SECRETS: usize = 4096;

fn secrets() -> &'static RwLock<VecDeque<String>> {
    static SECRETS: OnceLock<RwLock<VecDeque<String>>> = OnceLock::new();
    SECRETS.get_or_init(|| RwLock::new(VecDeque::new()))
}

/// Makes sure that the secret is replaced in logs from now on.
pub fn register(secret: &str) {
    if secret.len() < MIN_SECRET_LENGTH {
        return;
    }

    if secrets()
        .read()
        .unwrap_or_else(|err| err.into_inner())
        .iter()
        .any(|known| known == secret)
    {
        return;
    }

    // Nothing is logged while the lock is held: the log writer takes it too.
    let mut secrets = secrets().write().unwrap_or_else(|err| err.into_inner());
    if secrets.iter().any(|known| known == secret) {
        return;
    }
    if secrets.len() >= MAX_SECRETS {
        secrets.pop_front();
    }
    secrets.push_back(secret.to_owned());
}

/// Replaces registered secrets (and the given ones) in the text.
pub fn redact<'a>(text: &'a str, extra_secrets: &[&str]) -> Cow<'a, str> {
    let mut text = Cow::Borrowed(text);

    let replace = |text: &mut Cow<'a, str>, secret: &str| {
        if !secret.is_empty() && text.contains(secret) {
            *text = Cow::Owned(text.replace(secret, REDACTED));
        }
    };

    for secret in extra_secrets {
        replace(&mut text, secret);
    }

    for secret in secrets()
        .read()
        .unwrap_or_else(|err| err.into_inner())
        .iter()
    {
        replace(&mut text, secret);
    }

    text
}

/// Makes a message from elsewhere (e.g. a provider's error) safe to pass on: secrets are replaced, control
/// characters (other than line breaks) are dropped and it's shortened to at most `max_chars` characters.
pub fn sanitize_message(message: &str, extra_secrets: &[&str], max_chars: usize) -> String {
    let redacted = redact(message, extra_secrets);

    let mut result: String = redacted
        .chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .take(max_chars)
        .collect();

    if redacted.chars().count() > max_chars {
        result.push('…');
    }

    result
}

/// Wraps a log writer factory (e.g. `std::io::stdout`), redacting registered secrets from what's written.
#[derive(Clone, Copy, Debug)]
pub struct RedactingMakeWriter<M>(M);

impl<M> RedactingMakeWriter<M> {
    pub fn new(inner: M) -> Self {
        Self(inner)
    }
}

impl<'a, M: MakeWriter<'a>> MakeWriter<'a> for RedactingMakeWriter<M> {
    type Writer = RedactingWriter<M::Writer>;

    fn make_writer(&'a self) -> Self::Writer {
        RedactingWriter(self.0.make_writer())
    }
}

/// Redacts registered secrets from what's written through it.
///
/// The log formatter writes each event with a single call, so secrets aren't split between calls.
pub struct RedactingWriter<W>(W);

impl<W: io::Write> io::Write for RedactingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let text = String::from_utf8_lossy(buf);
        self.0.write_all(redact(&text, &[]).as_bytes())?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

/// The log writer used by `main`: standard output, with secrets redacted.
pub fn stdout() -> RedactingMakeWriter<fn() -> io::Stdout> {
    RedactingMakeWriter::new(io::stdout)
}

#[cfg(test)]
pub(crate) mod tests {
    use std::io::Write;
    use std::sync::{Arc, Mutex};

    use super::{REDACTED, RedactingMakeWriter, redact, register, sanitize_message};

    /// Collects log output, for checking what's logged.
    #[derive(Clone, Default)]
    pub(crate) struct LogBuffer(Arc<Mutex<Vec<u8>>>);

    impl LogBuffer {
        pub(crate) fn contents(&self) -> String {
            String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
        }
    }

    impl Write for LogBuffer {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogBuffer {
        type Writer = LogBuffer;

        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    /// A subscriber logging everything (trace level) to the buffer, without redaction.
    pub(crate) fn raw_subscriber(buffer: &LogBuffer) -> impl tracing::Subscriber + Send + Sync {
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .with_writer(buffer.clone())
            .finish()
    }

    #[test]
    fn registered_secrets_are_redacted() {
        let secret = "sk-test-registered-0001";
        assert_eq!(
            redact(&format!("key={secret}"), &[]),
            format!("key={secret}")
        );

        register(secret);
        assert_eq!(
            redact(&format!("key={secret}!"), &[]),
            format!("key={REDACTED}!")
        );

        // Short values are never registered.
        register("short");
        assert_eq!(redact("a short word", &[]), "a short word");

        // Extra secrets are redacted too.
        assert_eq!(redact("abc", &["b"]), format!("a{REDACTED}c"));
    }

    #[test]
    fn messages_are_sanitized() {
        let message = sanitize_message("bad key sk-extra-0002\u{7}\nnext", &["sk-extra-0002"], 100);
        assert_eq!(message, format!("bad key {REDACTED}\nnext"));

        assert_eq!(sanitize_message("abcdef", &[], 3), "abc…");
    }

    #[test]
    fn the_log_writer_redacts_secrets_whoever_logs_them() {
        let secret = "sk-test-logged-0003";
        register(secret);

        let buffer = LogBuffer::default();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .with_writer(RedactingMakeWriter::new(buffer.clone()))
            .finish();

        tracing::subscriber::with_default(subscriber, || {
            // Like a provider error echoing the key, logged by code outside this module.
            let body =
                format!(r#"{{"error": {{"message": "Incorrect API key provided: {secret}"}}}}"#);
            tracing::warn!(body, "request failed");
            tracing::debug!(config = ?serde_json::json!({"api_key": secret}), "config");
        });

        let logs = buffer.contents();
        assert!(logs.contains("request failed"), "{logs}");
        assert!(!logs.contains(secret), "{logs}");
        assert!(logs.contains(REDACTED), "{logs}");
    }
}
