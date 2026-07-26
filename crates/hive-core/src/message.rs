use serde::{Deserialize, Serialize};

/// Who authored a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

/// Where an image comes from. Either a remote URL or inline base64 bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImageSource {
    Url(String),
    Base64 { media_type: String, data: String },
}

impl ImageSource {
    /// Render as an OpenAI-style `image_url` data string.
    pub fn as_data_url(&self) -> String {
        match self {
            ImageSource::Url(u) => u.clone(),
            ImageSource::Base64 { media_type, data } => {
                format!("data:{media_type};base64,{data}")
            }
        }
    }
}

/// A single piece of message content. A message can mix text and images.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ContentPart {
    Text(String),
    Image(ImageSource),
}

/// A tool call requested by the assistant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// Raw JSON string of arguments exactly as the model emitted them.
    pub arguments: String,
}

/// A chat message in hive's internal representation. `hive-llm` converts this
/// to/from the provider wire format so the rest of the app never sees JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Vec<ContentPart>,
    /// Populated for assistant messages that requested tools.
    pub tool_calls: Vec<ToolCall>,
    /// Populated for `Role::Tool` messages: which call this answers.
    pub tool_call_id: Option<String>,
    /// Tool name, for `Role::Tool` messages.
    pub name: Option<String>,
}

impl Message {
    fn new(role: Role) -> Self {
        Message {
            role,
            content: Vec::new(),
            tool_calls: Vec::new(),
            tool_call_id: None,
            name: None,
        }
    }

    pub fn system(text: impl Into<String>) -> Self {
        let mut m = Message::new(Role::System);
        m.content.push(ContentPart::Text(text.into()));
        m
    }

    pub fn user(text: impl Into<String>) -> Self {
        let mut m = Message::new(Role::User);
        m.content.push(ContentPart::Text(text.into()));
        m
    }

    pub fn user_parts(parts: Vec<ContentPart>) -> Self {
        let mut m = Message::new(Role::User);
        m.content = parts;
        m
    }

    pub fn assistant(text: impl Into<String>) -> Self {
        let mut m = Message::new(Role::Assistant);
        m.content.push(ContentPart::Text(text.into()));
        m
    }

    pub fn tool_result(
        tool_call_id: impl Into<String>,
        name: impl Into<String>,
        text: impl Into<String>,
    ) -> Self {
        let mut m = Message::new(Role::Tool);
        m.tool_call_id = Some(tool_call_id.into());
        m.name = Some(name.into());
        m.content.push(ContentPart::Text(text.into()));
        m
    }

    /// Concatenate all textual content, ignoring images.
    pub fn text(&self) -> String {
        let mut out = String::new();
        for part in &self.content {
            if let ContentPart::Text(t) = part {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(t);
            }
        }
        out
    }

    pub fn images(&self) -> Vec<&ImageSource> {
        self.content
            .iter()
            .filter_map(|p| match p {
                ContentPart::Image(src) => Some(src),
                _ => None,
            })
            .collect()
    }
}
