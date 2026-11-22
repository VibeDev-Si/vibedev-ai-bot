//! Marks what AI models generate as AI-generated (`【AI 生成】` by default).
//!
//! Text answers get the label as a paragraph of its own (at the start by default), so that the answer's
//! Markdown stays intact. Media (images, generated speech) get it as their caption: the `body` of a media
//! message whose `filename` differs from it is a caption (Matrix v1.10+), which clients show next to it.
//!
//! The label is not part of the conversation: it's removed from the bot's own messages before they are
//! passed on to models (see `strip_from_matrix_message`), so that models don't learn to write it.

use std::sync::OnceLock;

use mxlink::matrix_sdk::ruma::events::room::message::{MessageType, RoomMessageEventContent};
use serde::Deserialize;

use crate::conversation::matrix::{MatrixMessage, MatrixMessageContent};

/// The label used unless configured otherwise.
pub const DEFAULT_TEXT: &str = "【AI 生成】";

/// The longest label accepted (in characters).
pub const MAX_TEXT_CHARS: usize = 64;

/// Where text answers get the label.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LabelPosition {
    /// A paragraph before the answer.
    #[default]
    Start,
    /// A paragraph after the answer.
    End,
}

impl LabelPosition {
    pub fn from_str(value: &str) -> Option<Self> {
        match value.trim() {
            "start" => Some(Self::Start),
            "end" => Some(Self::End),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AiLabel {
    text: String,
    position: LabelPosition,
}

impl Default for AiLabel {
    fn default() -> Self {
        Self::new(DEFAULT_TEXT, LabelPosition::Start)
    }
}

impl AiLabel {
    /// Creates a label. An empty text means the default one: the label can be changed, but not turned off.
    pub fn new(text: &str, position: LabelPosition) -> Self {
        let text = text.trim();

        Self {
            text: if text.is_empty() { DEFAULT_TEXT } else { text }.to_owned(),
            position,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn position(&self) -> LabelPosition {
        self.position
    }

    /// Labels a text answer (Markdown). Text which already carries the label isn't labeled again.
    pub fn apply_to_text(&self, text: &str) -> String {
        let text = text.trim();

        match self.position {
            LabelPosition::Start if text.starts_with(&self.text) => text.to_owned(),
            LabelPosition::Start => format!("{}\n\n{}", self.text, text),
            LabelPosition::End if text.ends_with(&self.text) => text.to_owned(),
            LabelPosition::End => format!("{}\n\n{}", text, self.text),
        }
    }

    /// Removes the label from a text labeled by [`Self::apply_to_text`] (other text is returned as is).
    pub fn strip_from_text<'a>(&self, text: &'a str) -> &'a str {
        match self.position {
            LabelPosition::Start => text
                .strip_prefix(self.text.as_str())
                .map_or(text, str::trim_start),
            LabelPosition::End => text
                .strip_suffix(self.text.as_str())
                .map_or(text, str::trim_end),
        }
    }

    /// Makes the label the caption of a media message (image, audio, video or file),
    /// keeping its file name in `filename`. Other messages are left alone.
    pub fn apply_to_media(&self, content: &mut RoomMessageEventContent) {
        let (body, filename) = match &mut content.msgtype {
            MessageType::Image(content) => (&mut content.body, &mut content.filename),
            MessageType::Audio(content) => (&mut content.body, &mut content.filename),
            MessageType::Video(content) => (&mut content.body, &mut content.filename),
            MessageType::File(content) => (&mut content.body, &mut content.filename),
            _ => return,
        };

        if filename.is_none() {
            *filename = Some(std::mem::take(body));
        }

        *body = self.text.clone();
    }
}

static CURRENT: OnceLock<AiLabel> = OnceLock::new();

/// Sets the label used from now on (once, at startup).
pub fn init(label: AiLabel) {
    if CURRENT.set(label).is_err() {
        tracing::warn!("The AI-generated label was already set - keeping the first one");
    }
}

/// The label in use ([`AiLabel::default`] until [`init`] is called).
pub fn current() -> &'static AiLabel {
    CURRENT.get_or_init(AiLabel::default)
}

/// Labels a text answer with the current label.
pub fn label_text(text: &str) -> String {
    current().apply_to_text(text)
}

/// Labels a media message with the current label.
pub fn label_media(content: &mut RoomMessageEventContent) {
    current().apply_to_media(content)
}

/// Removes the current label from one of the bot's own messages (before it becomes conversation context).
pub fn strip_from_matrix_message(message: &mut MatrixMessage) {
    strip_label_from_matrix_message(current(), message)
}

fn strip_label_from_matrix_message(label: &AiLabel, message: &mut MatrixMessage) {
    match &mut message.content {
        MatrixMessageContent::Text(text) | MatrixMessageContent::Notice(text) => {
            let stripped = label.strip_from_text(text);
            if stripped.len() != text.len() {
                *text = stripped.to_owned();
            }
        }
        MatrixMessageContent::Image(..) | MatrixMessageContent::File(..) => {}
    }
}

#[cfg(test)]
mod tests {
    use mxlink::matrix_sdk::ruma::events::room::message::{
        ImageMessageEventContent, MessageType, RoomMessageEventContent, TextMessageEventContent,
    };
    use mxlink::matrix_sdk::ruma::{OwnedMxcUri, owned_user_id};

    use super::{AiLabel, DEFAULT_TEXT, LabelPosition, strip_label_from_matrix_message};
    use crate::conversation::matrix::{MatrixMessage, MatrixMessageContent};

    #[test]
    fn answers_get_the_label_as_a_paragraph_of_its_own() {
        let label = AiLabel::default();
        assert_eq!(label.text(), DEFAULT_TEXT);

        assert_eq!(label.apply_to_text("Hello"), "【AI 生成】\n\nHello");
        // Markdown at the start of the answer keeps working.
        assert_eq!(
            label.apply_to_text("# Title\n\n```rust\nfn main() {}\n```\n"),
            "【AI 生成】\n\n# Title\n\n```rust\nfn main() {}\n```"
        );
        // Not twice.
        assert_eq!(
            label.apply_to_text("【AI 生成】\n\nHello"),
            "【AI 生成】\n\nHello"
        );

        let label = AiLabel::new("AI-generated", LabelPosition::End);
        assert_eq!(label.apply_to_text("Hello\n"), "Hello\n\nAI-generated");
    }

    #[test]
    fn the_label_can_be_changed_but_not_removed() {
        assert_eq!(
            AiLabel::new("  ", LabelPosition::Start).text(),
            DEFAULT_TEXT
        );
        assert_eq!(AiLabel::new(" AI ", LabelPosition::Start).text(), "AI");
        assert_eq!(LabelPosition::from_str("end"), Some(LabelPosition::End));
        assert_eq!(LabelPosition::from_str("middle"), None);
    }

    #[test]
    fn stripping_reverses_labeling() {
        for position in [LabelPosition::Start, LabelPosition::End] {
            let label = AiLabel::new(DEFAULT_TEXT, position);

            for text in ["Hello", "# Title\n\n- a\n- b", "第一行\n第二行"] {
                assert_eq!(label.strip_from_text(&label.apply_to_text(text)), text);
            }

            // Text without the label is left alone.
            assert_eq!(label.strip_from_text("Hello"), "Hello");
        }
    }

    #[test]
    fn only_text_and_notices_are_stripped() {
        let label = AiLabel::default();

        let mut message = MatrixMessage {
            sender_id: owned_user_id!("@bot:example.com"),
            content: MatrixMessageContent::Text(label.apply_to_text("Hi")),
            mentioned_users: vec![],
            timestamp: chrono::Utc::now(),
        };
        strip_label_from_matrix_message(&label, &mut message);
        assert!(matches!(&message.content, MatrixMessageContent::Text(text) if text == "Hi"));

        // A labeled transcription becomes a plain transcription again (see `parse_transcribed_message_text`).
        let transcription = crate::utils::text_to_speech::create_transcribed_message_text("Hi");
        message.content = MatrixMessageContent::Notice(label.apply_to_text(&transcription));
        strip_label_from_matrix_message(&label, &mut message);
        assert!(
            matches!(&message.content, MatrixMessageContent::Notice(text) if *text == transcription)
        );
    }

    #[test]
    fn media_get_the_label_as_caption() {
        let label = AiLabel::default();

        let mut content =
            RoomMessageEventContent::new(MessageType::Image(ImageMessageEventContent::plain(
                "generated-image.png".to_owned(),
                OwnedMxcUri::from("mxc://example.com/abc"),
            )));
        label.apply_to_media(&mut content);

        let MessageType::Image(image) = &content.msgtype else {
            panic!("not an image");
        };
        assert_eq!(image.body, DEFAULT_TEXT);
        assert_eq!(image.filename.as_deref(), Some("generated-image.png"));

        // Labeling again keeps the file name.
        label.apply_to_media(&mut content);
        let MessageType::Image(image) = &content.msgtype else {
            panic!("not an image");
        };
        assert_eq!(image.filename.as_deref(), Some("generated-image.png"));

        // Text messages are not media.
        let mut text =
            RoomMessageEventContent::new(MessageType::Text(TextMessageEventContent::plain("Hi")));
        label.apply_to_media(&mut text);
        assert_eq!(text.body(), "Hi");
    }
}
