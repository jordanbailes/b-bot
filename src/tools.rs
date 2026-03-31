use std::collections::HashMap;

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::error::{AppError, Result};
use crate::ollama::ToolDefinition;

mod coverage;
mod docker;
mod loc;

pub use coverage::RustRunCoverageTool;
pub use docker::DockerRemoveAllRunningContainersTool;
pub use loc::RustCountLinesOfCodeTool;

#[derive(Debug, Clone)]
pub struct ToolExecutionOutcome {
    pub display: String,
}

#[derive(Debug, Clone)]
pub struct ToolPreview {
    pub display: Option<String>,
    pub is_noop: bool,
    pub requires_confirmation: bool,
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn definition(&self) -> ToolDefinition;

    async fn preview(&self, arguments: Value) -> Result<ToolPreview>;

    async fn execute(&self, arguments: Value) -> Result<ToolExecutionOutcome>;
}

pub struct ToolRegistry {
    specs: Vec<ToolDefinition>,
    tools: HashMap<String, Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn with_defaults() -> Self {
        let mut tools: HashMap<String, Box<dyn Tool>> = HashMap::new();
        let docker_tool = Box::new(DockerRemoveAllRunningContainersTool::new());
        tools.insert(
            docker_tool.definition().function.name.to_owned(),
            docker_tool,
        );
        let loc_tool = Box::new(RustCountLinesOfCodeTool::new());
        tools.insert(loc_tool.definition().function.name.to_owned(), loc_tool);
        let coverage_tool = Box::new(RustRunCoverageTool::new());
        tools.insert(
            coverage_tool.definition().function.name.to_owned(),
            coverage_tool,
        );

        let specs = tools.values().map(|tool| tool.definition()).collect();
        Self { specs, tools }
    }

    pub fn specs(&self) -> Vec<ToolDefinition> {
        self.specs.clone()
    }

    pub async fn preview(&self, tool_name: &str, arguments: Value) -> Result<ToolPreview> {
        let tool = self
            .tools
            .get(tool_name)
            .ok_or_else(|| AppError::UnknownTool(tool_name.to_owned()))?;
        tool.preview(arguments).await
    }

    pub async fn execute(&self, tool_name: &str, arguments: Value) -> Result<ToolExecutionOutcome> {
        let tool = self
            .tools
            .get(tool_name)
            .ok_or_else(|| AppError::UnknownTool(tool_name.to_owned()))?;
        tool.execute(arguments).await
    }
}

pub fn docker_remove_all_tool_definition() -> ToolDefinition {
    ToolDefinition::new(
        "docker.remove_all_running_containers",
        "Stop and remove every currently running Docker container. Use this only when the user wants running containers deleted or removed, not when they only want containers stopped.",
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "stop_timeout_secs": {
                    "type": "integer",
                    "description": "Grace period in seconds before Docker force-stops a container. Default to 10 unless the user asks for a different timeout."
                },
                "remove_anonymous_volumes": {
                    "type": "boolean",
                    "description": "Set true only if the user explicitly asks to remove anonymous volumes too. Default false."
                }
            },
            "required": ["stop_timeout_secs", "remove_anonymous_volumes"]
        }),
    )
}

pub fn rust_count_lines_of_code_tool_definition() -> ToolDefinition {
    ToolDefinition::new(
        "rust.count_lines_of_code",
        "Count Rust source files and Rust lines of code in a repository or directory using the embedded tokei library. Use this for requests about lines of code, code size, Rust file counts, comments, blanks, or total Rust LOC.",
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Repository or directory path to analyze. Use '.' for the current working directory when the user refers to this repo or does not specify a path."
                }
            }
        }),
    )
}

pub fn rust_run_coverage_tool_definition() -> ToolDefinition {
    ToolDefinition::new(
        "rust.run_coverage",
        "Run Rust test coverage for a repository or crate using the embedded Tarpaulin library with the ptrace engine on Linux x86_64. Use this for requests about coverage, line coverage, uncovered code, or running Tarpaulin.",
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Repository directory or Cargo.toml path to analyze. Use '.' for the current working directory when the user refers to this repo or does not specify a path."
                }
            }
        }),
    )
}
