use clap::Parser;

#[derive(Debug, Parser)]
#[command(
    name = "bbot",
    about = "B-Bot: a local-first tool-oriented CLI assistant"
)]
pub struct Cli {
    #[arg(
        long,
        env = "OLLAMA_HOST",
        default_value = "http://127.0.0.1:11434",
        help = "Base URL for the local Ollama server"
    )]
    pub ollama_host: String,

    #[arg(
        long,
        env = "OLLAMA_MODEL",
        default_value = "qwen3:8b",
        help = "Ollama model to use for planning"
    )]
    pub model: String,

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
}
