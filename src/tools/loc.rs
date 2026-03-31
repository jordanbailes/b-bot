use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;
use tokei::{Config, LanguageType, Languages};

use crate::error::{AppError, Result};
use crate::ollama::ToolDefinition;
use crate::tools::{
    Tool, ToolExecutionOutcome, ToolPreview, rust_count_lines_of_code_tool_definition,
};

#[derive(Debug, Deserialize)]
struct RustCountLinesOfCodeArgs {
    path: Option<String>,
}

#[derive(Debug)]
struct RustLocStats {
    path: PathBuf,
    files: usize,
    lines: usize,
    code: usize,
    comments: usize,
    blanks: usize,
}

pub struct RustCountLinesOfCodeTool;

impl RustCountLinesOfCodeTool {
    pub fn new() -> Self {
        Self
    }

    fn parse_args(&self, arguments: Value) -> Result<PathBuf> {
        let args: RustCountLinesOfCodeArgs =
            serde_json::from_value(arguments).map_err(|source| AppError::InvalidToolArguments {
                tool: self.definition().function.name.to_owned(),
                source,
            })?;
        let path = args.path.unwrap_or_else(|| ".".to_owned());
        Ok(PathBuf::from(path))
    }

    fn count_rust_loc(&self, path: &Path) -> Result<RustLocStats> {
        if !path.exists() {
            return Err(AppError::Message(format!(
                "path does not exist: {}",
                path.display()
            )));
        }

        let mut languages = Languages::new();
        let paths = [path];
        let ignored = [".git", "target"];
        languages.get_statistics(&paths, &ignored, &Config::default());

        let resolved_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());

        if let Some(rust) = languages.get(&LanguageType::Rust) {
            return Ok(RustLocStats {
                path: resolved_path,
                files: rust.reports.len(),
                lines: rust.lines(),
                code: rust.code,
                comments: rust.comments,
                blanks: rust.blanks,
            });
        }

        Ok(RustLocStats {
            path: resolved_path,
            files: 0,
            lines: 0,
            code: 0,
            comments: 0,
            blanks: 0,
        })
    }
}

#[async_trait]
impl Tool for RustCountLinesOfCodeTool {
    fn definition(&self) -> ToolDefinition {
        rust_count_lines_of_code_tool_definition()
    }

    async fn preview(&self, arguments: Value) -> Result<ToolPreview> {
        let path = self.parse_args(arguments)?;
        let stats = self.count_rust_loc(&path)?;

        let display = Some(format!(
            "Counting Rust lines of code under {}.",
            stats.path.display()
        ));

        Ok(ToolPreview {
            display,
            is_noop: false,
            requires_confirmation: false,
        })
    }

    async fn execute(&self, arguments: Value) -> Result<ToolExecutionOutcome> {
        let path = self.parse_args(arguments)?;
        let stats = self.count_rust_loc(&path)?;

        let display = if stats.files == 0 {
            format!("No Rust source files found under {}.", stats.path.display())
        } else {
            format!(
                "Rust LOC for {}:\n- files: {}\n- total lines: {}\n- code: {}\n- comments: {}\n- blanks: {}",
                stats.path.display(),
                stats.files,
                stats.lines,
                stats.code,
                stats.comments,
                stats.blanks
            )
        };

        Ok(ToolExecutionOutcome { display })
    }
}
