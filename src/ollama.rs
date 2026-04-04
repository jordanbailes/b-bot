use bon::Builder;
use serde::Serialize;

use crate::error::Result;
use crate::planner::{
    ChatResponse, Message, PlannerBackend, SYSTEM_PROMPT, ToolDefinition, parse_chat_response,
};

pub const DEFAULT_OLLAMA_HOST: &str = "http://127.0.0.1:11434";
pub const DEFAULT_OLLAMA_MODEL: &str = "qwen3:8b";

#[derive(Debug, Clone, Builder)]
pub struct OllamaPlannerConfig {
    #[builder(into)]
    pub base_url: String,
    #[builder(into)]
    pub model: String,
}

pub struct OllamaPlannerBackend {
    http: reqwest::Client,
    config: OllamaPlannerConfig,
}

impl OllamaPlannerBackend {
    pub fn new(config: OllamaPlannerConfig) -> Self {
        Self {
            http: reqwest::Client::new(),
            config,
        }
    }
}

#[async_trait::async_trait]
impl PlannerBackend for OllamaPlannerBackend {
    async fn plan(&self, prompt: &str, tools: &[ToolDefinition]) -> Result<ChatResponse> {
        let messages = vec![
            Message::system(SYSTEM_PROMPT),
            Message::user(prompt.to_owned()),
        ];
        let request = ChatRequest {
            model: &self.config.model,
            messages: &messages,
            tools,
            options: ChatOptions { temperature: 0.0 },
            stream: false,
            think: false,
            keep_alive: -1,
        };
        let url = format!("{}/api/chat", self.config.base_url.trim_end_matches('/'));

        let response = self
            .http
            .post(url)
            .json(&request)
            .send()
            .await?
            .error_for_status()?;

        let body = response.text().await?;
        parse_chat_response("Ollama", &body)
    }
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
