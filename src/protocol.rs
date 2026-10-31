use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize)]
pub struct Request {
    pub id: u64,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Response {
    pub id: Option<u64>,
    pub ok: Option<bool>,
    pub result: Option<Value>,
    pub error: Option<String>,
    pub event: Option<String>,
    #[serde(flatten)]
    pub extra: Value,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Profile {
    pub mid: String,
    #[serde(rename = "displayName")]
    pub display_name: String,
    #[serde(rename = "statusMessage", default)]
    pub status_message: String,
    #[serde(rename = "picturePath", default)]
    pub picture_path: Option<String>,
    #[serde(rename = "avatarPath", default)]
    pub avatar_path: Option<String>,
    #[serde(rename = "pictureUrl", default)]
    pub picture_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChatInfo {
    pub mid: String,
    pub name: String,
    #[serde(default)]
    pub kind: String,
    #[serde(rename = "avatarPath", default)]
    pub avatar_path: Option<String>,
    #[serde(rename = "lastActivity", default)]
    pub last_activity: i64,
    #[serde(default)]
    pub unread: i64,
    #[serde(default)]
    pub preview: String,
    #[serde(rename = "muted", default)]
    pub muted: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FlexAction {
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub data: Option<String>,
    #[serde(default)]
    pub uri: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FlexInfo {
    #[serde(default)]
    pub contents: Option<Value>,
    #[serde(rename = "altText", default)]
    pub alt_text: String,
    #[serde(default)]
    pub texts: Vec<String>,
    #[serde(default)]
    pub actions: Vec<FlexAction>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AlbumRef {
    #[serde(rename = "chatMid")]
    pub chat_mid: String,
    #[serde(rename = "albumId")]
    pub album_id: String,
    pub title: String,
}
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AlbumMediaKind {
    Image,
    Video,
}
#[derive(Debug, Clone, Deserialize)]
pub struct AlbumPhoto {
    pub oid: String,
    pub kind: AlbumMediaKind,
}
#[derive(Debug, Clone, Deserialize)]
pub struct AlbumPage {
    pub photos: Vec<AlbumPhoto>,
    #[serde(rename = "nextCursor")]
    pub next_cursor: Option<String>,
}
#[derive(Debug, Clone, serde::Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplyTarget {
    pub chat_mid: String,
    pub id: String,
    #[serde(default)]
    pub sender: String,
    #[serde(default)]
    pub preview: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LocationInfo {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub address: String,
    pub latitude: f64,
    pub longitude: f64,
}

impl LocationInfo {
    pub fn is_valid(&self) -> bool {
        self.latitude.is_finite()
            && self.longitude.is_finite()
            && (-90.0..=90.0).contains(&self.latitude)
            && (-180.0..=180.0).contains(&self.longitude)
    }

    pub fn maps_url(&self) -> Option<String> {
        self.is_valid().then(|| {
            format!(
                "https://www.google.com/maps/search/?api=1&query={}%2C{}",
                self.latitude, self.longitude
            )
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct MessageReaction {
    #[serde(default)]
    pub from: String,
    #[serde(rename = "type", deserialize_with = "deserialize_reaction_type")]
    pub kind: u8,
}

fn deserialize_reaction_type<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<u8, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(match value {
        serde_json::Value::Number(n) => n.as_u64().and_then(|n| u8::try_from(n).ok()).unwrap_or(0),
        serde_json::Value::String(s) => match s.as_str() {
            "NICE" => 2,
            "LOVE" => 3,
            "FUN" => 4,
            "AMAZING" => 5,
            "SAD" => 6,
            "OMG" => 7,
            _ => 0,
        },
        _ => 0,
    })
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HistoryCursor {
    pub message_id: String,
    pub delivered_time: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MessageInfo {
    #[serde(rename = "historyCursor", default)]
    pub history_cursor: Option<HistoryCursor>,
    #[serde(default)]
    pub reactions: Vec<MessageReaction>,
    #[serde(default)]
    pub location: Option<Box<LocationInfo>>,
    #[serde(default)]
    pub reply: Option<Box<ReplyTarget>>,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub from: String,
    #[serde(default)]
    pub to: String,
    #[serde(default)]
    pub mine: bool,
    #[serde(rename = "createdTime", default)]
    pub created_time: i64,
    #[serde(rename = "contentType", default)]
    pub content_type: String,
    #[serde(rename = "imagePath", default)]
    pub image_path: Option<String>,
    #[serde(rename = "audioPath", default)]
    pub audio_path: Option<String>,
    #[serde(rename = "fileName", default)]
    pub file_name: Option<String>,
    #[serde(rename = "fileSize", default)]
    pub file_size: Option<u64>,
    #[serde(rename = "filePath", default)]
    pub file_path: Option<String>,
    #[serde(rename = "durationMs", default)]
    pub duration_ms: Option<i64>,
    #[serde(default)]
    pub flex: Option<FlexInfo>,
    #[serde(default)]
    pub album: Option<Box<AlbumRef>>,
}

impl MessageInfo {
    pub fn chat_mid(&self) -> &str {
        if self.mine || self.to.starts_with(['c', 'r']) {
            &self.to
        } else {
            &self.from
        }
    }
}

#[derive(Debug, Clone)]
pub enum ProtocolEvent {
    Ready {
        has_auth: bool,
    },
    Session {
        mid: String,
        display_name: String,
        status_message: String,
        picture_path: Option<String>,
        avatar_path: Option<String>,
        picture_url: Option<String>,
    },
    SessionFailed {
        error: String,
    },
    Qr {
        url: String,
    },
    Pin {
        pin: String,
    },
    Listening,
    ReactionChanged,
    Message(Box<MessageInfo>),
    Chats {
        chats: Vec<ChatInfo>,
        cached: bool,
    },
    Messages {
        chat_mid: String,
        messages: Vec<MessageInfo>,
    },
    AvatarReady {
        mid: String,
        avatar_path: String,
    },
    FriendsUpdated {
        friends: Vec<ChatInfo>,
    },
    ChatUpsert {
        chat: ChatInfo,
    },
    ChatPreview {
        mid: String,
        preview: String,
        last_activity: i64,
    },
    ChatMute {
        mid: String,
        muted: bool,
    },
    MediaReady {
        chat_mid: String,
        message_id: String,
        image_path: String,
        audio_path: Option<String>,
        file_path: Option<String>,
    },
    MediaFailed {
        chat_mid: String,
        message_id: String,
    },
    Progress {
        scope: String,
        chat_mid: Option<String>,
        state: String,
        error: Option<String>,
    },
    UploadProgress {
        chat_mid: String,
        progress: f64,
        label: String,
        done: bool,
    },
    ReadReceipt {
        chat_mid: String,
        message_id: String,
    },
    CallIncoming {
        call_id: String,
        from: String,
        kind: String,
    },
    CallCanceled {
        call_id: String,
        from: String,
        reason: String,
    },
    CallState {
        call_id: String,
        peer: String,
        state: String,
        error: Option<String>,
    },
    Response {
        id: u64,
        ok: bool,
        result: Value,
        error: Option<String>,
    },
    Error(String),
    Exited(i32),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_event_envelope_and_extra_fields() {
        let response: Response =
            serde_json::from_str(r#"{"event":"ready","hasAuth":true,"dataDir":"/tmp/line-gtk"}"#)
                .unwrap();
        assert_eq!(response.event.as_deref(), Some("ready"));
        assert_eq!(response.extra["hasAuth"], true);
        assert_eq!(response.extra["dataDir"], "/tmp/line-gtk");
    }

    #[test]
    fn message_defaults_tolerate_protocol_evolution() {
        let message: MessageInfo = serde_json::from_str(r#"{"id":"42","text":"hello"}"#).unwrap();
        assert_eq!(message.id, "42");
        assert_eq!(message.text, "hello");
        assert_eq!(message.created_time, 0);
        assert!(!message.mine);
        assert!(message.image_path.is_none());
        assert!(message.file_name.is_none());
        assert!(message.file_size.is_none());
    }

    #[test]
    fn message_routes_to_its_conversation() {
        for (from, to, mine, expected) in [
            ("u-peer", "u-self", false, "u-peer"),
            ("u-self", "u-peer", true, "u-peer"),
            ("u-member", "c-group", false, "c-group"),
            ("u-self", "c-group", true, "c-group"),
            ("u-member", "r-room", false, "r-room"),
            ("u-self", "r-room", true, "r-room"),
        ] {
            let message: MessageInfo = serde_json::from_value(serde_json::json!({
                "from": from, "to": to, "mine": mine,
            }))
            .unwrap();
            assert_eq!(message.chat_mid(), expected, "{from} -> {to}, mine={mine}");
        }
    }

    #[test]
    fn message_reads_original_attachment_metadata() {
        let message: MessageInfo =
            serde_json::from_str(r#"{"id":"42","fileName":"รายงาน.docx","fileSize":2048}"#)
                .unwrap();
        assert_eq!(message.file_name.as_deref(), Some("รายงาน.docx"));
        assert_eq!(message.file_size, Some(2048));
        let message: MessageInfo =
            serde_json::from_str(r#"{"fileName":null,"fileSize":null}"#).unwrap();
        assert!(message.file_name.is_none());
        assert!(message.file_size.is_none());
    }

    #[test]
    fn request_omits_absent_params() {
        let request = Request {
            id: 7,
            method: "list_chats".into(),
            params: None,
        };
        let value = serde_json::to_value(request).unwrap();
        assert_eq!(
            value,
            serde_json::json!({ "id": 7, "method": "list_chats" })
        );
    }
}
