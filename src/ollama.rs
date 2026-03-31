use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::error::{AppError, Result};

pub const SYSTEM_PROMPT: &str = r#"You are B-Bot's planner for a Linux command-line assistant.

You must follow these rules:
- B-Bot is tool-first. Do not output shell commands, pipes, or command-by-command choreography.
- The user should not need to remember Docker flags, container listing steps, stop/remove sequencing, docker compose cleanup mechanics, or how to invoke tokei or Tarpaulin.
- You have exactly three available tools: docker.remove_all_running_containers, rust.count_lines_of_code, and rust.run_coverage.
- That tool is the right tool only when the user wants all currently running Docker containers deleted, removed, wiped, or cleaned up.
- If the user asks to only stop containers, do not call the tool because stopping without removing is not currently supported.
- That tool already handles the internal multi-step work needed to list running containers, stop them gracefully, and remove them.
- rust.count_lines_of_code is the right tool when the user asks to count Rust lines of code, code size, file count, comments, blanks, or to use tokei on a Rust repository or directory.
- For rust.count_lines_of_code, use path "." when the user refers to the current repo or does not specify a path.
- rust.run_coverage is the right tool when the user asks to run coverage, check test coverage, use Tarpaulin, or see uncovered Rust code in a repository or crate.
- For rust.run_coverage, use path "." when the user refers to the current repo or does not specify a path.
- Default stop_timeout_secs to 10 unless the user asks for a different timeout.
- Set remove_anonymous_volumes to true only if the user explicitly asks to remove volumes too. Otherwise set it to false.
- Only call the tool if you are highly confident it matches the user's intent.
- If the request is ambiguous, ask a brief clarification question instead of calling a tool.
- If the request cannot be satisfied with the available tool, say so briefly and do not invent tools.
- Never recommend shell pipelines unless the user explicitly asks for command-line equivalents.

When you can satisfy the request, call the tool.
When you cannot, respond with a short plain-language explanation."#;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".to_owned(),
            content: Some(content.into()),
            thinking: None,
            tool_calls: None,
            tool_name: None,
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".to_owned(),
            content: Some(content.into()),
            thinking: None,
            tool_calls: None,
            tool_name: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolDefinition {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub function: FunctionDefinition,
}

impl ToolDefinition {
    pub fn new(name: &'static str, description: &'static str, parameters: Value) -> Self {
        Self {
            kind: "function",
            function: FunctionDefinition {
                name,
                description,
                parameters,
            },
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct FunctionDefinition {
    pub name: &'static str,
    pub description: &'static str,
    pub parameters: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
    pub function: CalledFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalledFunction {
    pub name: String,
    #[serde(deserialize_with = "deserialize_arguments")]
    pub arguments: Value,
}

#[derive(Debug, Deserialize)]
pub struct ChatResponse {
    pub message: Message,
}

#[derive(Debug, Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: &'a [Message],
    tools: &'a [ToolDefinition],
    options: ChatOptions,
    stream: bool,
    think: bool,
    keep_alive: i32,
}

#[derive(Debug, Serialize)]
struct ChatOptions {
    temperature: f32,
}

pub struct OllamaClient {
    http: reqwest::Client,
    base_url: String,
    model: String,
}

impl OllamaClient {
    pub fn new(base_url: String, model: String) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url,
            model,
        }
    }

    pub async fn chat(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> Result<ChatResponse> {
        let request = ChatRequest {
            model: &self.model,
            messages,
            tools,
            options: ChatOptions { temperature: 0.0 },
            stream: false,
            think: false,
            keep_alive: -1,
        };
        let url = format!("{}/api/chat", self.base_url.trim_end_matches('/'));

        let response = self
            .http
            .post(url)
            .json(&request)
            .send()
            .await?
            .error_for_status()?;

        let body = response.text().await?;
        let parsed = serde_json::from_str::<ChatResponse>(&body).map_err(|source| {
            AppError::OllamaParse {
                source,
                body: body.clone(),
            }
        })?;
        if parsed.message.content.is_none() && parsed.message.tool_calls.is_none() {
            return Err(AppError::InvalidOllamaResponse);
        }

        Ok(parsed)
    }
}

fn deserialize_arguments<'de, D>(deserializer: D) -> std::result::Result<Value, D::Error>
where
    D: Deserializer<'de>,
{
    let raw = Value::deserialize(deserializer)?;
    Ok(match raw {
        Value::String(text) => match serde_json::from_str(&text) {
            Ok(parsed) => parsed,
            Err(_) => Value::String(text),
        },
        other => other,
    })
}
