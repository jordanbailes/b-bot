use clap::{Parser, ValueEnum};

use crate::{
    codex::ReasoningEffort,
    ollama::{DEFAULT_OLLAMA_HOST, DEFAULT_OLLAMA_MODEL},
};

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq, ValueEnum)]
pub enum Backend {
    #[default]
    Ollama,
    Codex,
}

#[derive(Debug, Parser)]
#[command(
    name = "bbot",
    about = "B-Bot: a local-first tool-oriented CLI assistant"
)]
pub struct Cli {
    #[arg(
        long,
        env = "BBOT_BACKEND",
        value_enum,
        default_value_t = Backend::Ollama,
        help = "Planner backend to use"
    )]
    pub backend: Backend,

    #[arg(
        long,
        env = "OLLAMA_HOST",
        default_value = DEFAULT_OLLAMA_HOST,
        help = "Base URL for the Ollama server when --backend=ollama"
    )]
    pub ollama_host: String,

    #[arg(
        long,
        env = "BBOT_MODEL",
        help = "Planner model to use. Defaults to qwen3:8b for Ollama, or Codex's preferred model selection for --backend=codex"
    )]
    pub model: Option<String>,

    #[arg(
        long,
        env = "CODEX_BINARY",
        default_value = "codex",
        help = "Codex CLI binary to launch when --backend=codex"
    )]
    pub codex_binary: String,

    #[arg(
        long,
        env = "CODEX_EFFORT",
        value_enum,
        default_value_t = ReasoningEffort::Medium,
        help = "Reasoning effort to use when --backend=codex"
    )]
    pub codex_effort: ReasoningEffort,

    #[arg(
        long,
        env = "CODEX_NETWORK",
        help = "Allow network access inside the Codex sandbox when --backend=codex"
    )]
    pub codex_network: bool,

    #[arg(long, help = "Show the planned tool call but do not execute it")]
    pub dry_run: bool,

    #[arg(long, help = "Skip the confirmation prompt")]
    pub yes: bool,

    #[arg(value_name = "PROMPT", trailing_var_arg = true)]
    prompt: Vec<String>,
}

impl Cli {
    pub fn prompt_text(&self) -> Option<String> {
        if self.prompt.is_empty() {
            None
        } else {
            Some(self.prompt.join(" "))
        }
    }

    pub fn ollama_model(&self) -> String {
        self.model
            .clone()
            .unwrap_or_else(|| DEFAULT_OLLAMA_MODEL.to_owned())
    }
}
