mod cli;
mod error;
mod gap_store;
mod ollama;
mod tools;

use std::io::{self, Write};

use clap::Parser;

use crate::cli::Cli;
use crate::error::{AppError, Result};
use crate::gap_store::GapStore;
use crate::ollama::{Message, OllamaClient, SYSTEM_PROMPT};
use crate::tools::ToolRegistry;

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    if let Err(err) = run(cli).await {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

async fn run(cli: Cli) -> Result<()> {
    let registry = ToolRegistry::with_defaults();
    let ollama = OllamaClient::new(cli.ollama_host.clone(), cli.model.clone());

    if let Some(prompt) = cli.prompt_text() {
        return handle_prompt(&prompt, cli.dry_run, cli.yes, &ollama, &registry).await;
    }

    interactive_loop(&cli, &ollama, &registry).await
}

async fn interactive_loop(cli: &Cli, ollama: &OllamaClient, registry: &ToolRegistry) -> Result<()> {
    println!("B-Bot interactive mode. Type 'exit' or 'quit' to leave.");

    loop {
        print!("bbot> ");
        io::stdout().flush()?;

        let mut input = String::new();
        let bytes_read = io::stdin().read_line(&mut input)?;
        if bytes_read == 0 {
            println!();
            break;
        }

        let prompt = input.trim();
        if prompt.is_empty() {
            continue;
        }
        if matches!(prompt, "exit" | "quit") {
            break;
        }

        match handle_prompt(prompt, cli.dry_run, cli.yes, ollama, registry).await {
            Ok(()) => {}
            Err(AppError::UserDeclined) => {
                println!("Cancelled.");
            }
            Err(err) => {
                eprintln!("error: {err}");
            }
        }
    }

    Ok(())
}

async fn handle_prompt(
    prompt: &str,
    dry_run: bool,
    yes: bool,
    ollama: &OllamaClient,
    registry: &ToolRegistry,
) -> Result<()> {
    let messages = vec![
        Message::system(SYSTEM_PROMPT),
        Message::user(prompt.to_owned()),
    ];
    let response = ollama.chat(&messages, &registry.specs()).await?;
    let assistant_message = response.message;

    if let Some(tool_calls) = assistant_message.tool_calls.clone() {
        if tool_calls.len() != 1 {
            return Err(AppError::Message(format!(
                "expected exactly one tool call, got {}",
                tool_calls.len()
            )));
        }

        let tool_call = &tool_calls[0];
        println!("Tool: {}", tool_call.function.name);
        println!(
            "Arguments:\n{}",
            serde_json::to_string_pretty(&tool_call.function.arguments)?
        );

        let preview = registry
            .preview(
                &tool_call.function.name,
                tool_call.function.arguments.clone(),
            )
            .await?;

        if let Some(preview_text) = preview.display {
            println!();
            println!("{preview_text}");
        }

        if dry_run {
            return Ok(());
        }

        if preview.is_noop {
            println!();
            println!("No action required. Skipping execution and confirmation.");
            return Ok(());
        }

        if preview.requires_confirmation && !yes {
            println!();
            println!("Confirmation required before execution.");
            prompt_for_confirmation()?;
        }

        let outcome = registry
            .execute(
                &tool_call.function.name,
                tool_call.function.arguments.clone(),
            )
            .await?;
        println!();
        println!("{}", outcome.display);
        return Ok(());
    }

    let content = assistant_message.content.unwrap_or_else(|| {
        "B-Bot could not match that request to its current tool set.".to_owned()
    });
    println!("{content}");

    if let Err(err) = GapStore::default().append(prompt, &content) {
        eprintln!("warning: failed to write tooling gap log: {err}");
    }

    Ok(())
}

fn prompt_for_confirmation() -> Result<()> {
    print!("Run this tool? [y/N] ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    match input.trim().to_ascii_lowercase().as_str() {
        "y" | "yes" => Ok(()),
        _ => Err(AppError::UserDeclined),
    }
}
