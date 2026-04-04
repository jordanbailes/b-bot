mod cli;
mod codex;
mod error;
mod gap_store;
mod ollama;
mod planner;
mod tools;

use std::io::{self, Write};

use clap::Parser;

use crate::cli::{Backend, Cli};
use crate::codex::{CodexPlannerBackend, CodexPlannerConfig};
use crate::error::{AppError, Result};
use crate::gap_store::GapStore;
use crate::ollama::{OllamaPlannerBackend, OllamaPlannerConfig};
use crate::planner::PlannerBackend;
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
    let planner = build_planner(&cli)?;

    if let Some(prompt) = cli.prompt_text() {
        return handle_prompt(&prompt, cli.dry_run, cli.yes, planner.as_ref(), &registry).await;
    }

    interactive_loop(&cli, planner.as_ref(), &registry).await
}

fn build_planner(cli: &Cli) -> Result<Box<dyn PlannerBackend>> {
    match cli.backend {
        Backend::Ollama => {
            let config = OllamaPlannerConfig::builder()
                .base_url(cli.ollama_host.clone())
                .model(cli.ollama_model())
                .build();
            Ok(Box::new(OllamaPlannerBackend::new(config)))
        }
        Backend::Codex => {
            let cwd = std::env::current_dir()?.canonicalize()?;
            let config = CodexPlannerConfig::builder()
                .binary(cli.codex_binary.clone())
                .cwd(cwd)
                .maybe_model(cli.model.clone())
                .effort(cli.codex_effort)
                .network_access(cli.codex_network)
                .build();
            Ok(Box::new(CodexPlannerBackend::new(config)))
        }
    }
}

async fn interactive_loop(
    cli: &Cli,
    planner: &dyn PlannerBackend,
    registry: &ToolRegistry,
) -> Result<()> {
    println!("B-Bot interactive mode. Type 'exit' or 'quit' to leave.");
    println!("Planner backend: {:?}", cli.backend);

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

        match handle_prompt(prompt, cli.dry_run, cli.yes, planner, registry).await {
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
    planner: &dyn PlannerBackend,
    registry: &ToolRegistry,
) -> Result<()> {
    let tool_specs = registry.specs();
    let response = planner.plan(prompt, &tool_specs).await?;
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
