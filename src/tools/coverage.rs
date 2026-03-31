use std::path::{Path, PathBuf};

use async_trait::async_trait;
use cargo_tarpaulin::{
    config::{Config, TraceEngine},
    trace,
};
use serde::Deserialize;
use serde_json::Value;

use crate::error::{AppError, Result};
use crate::ollama::ToolDefinition;
use crate::tools::{Tool, ToolExecutionOutcome, ToolPreview, rust_run_coverage_tool_definition};

#[derive(Debug, Deserialize)]
struct RustRunCoverageArgs {
    path: Option<String>,
}

#[derive(Debug)]
struct CoverageTarget {
    display_path: PathBuf,
    manifest_path: PathBuf,
}

pub struct RustRunCoverageTool;

impl RustRunCoverageTool {
    pub fn new() -> Self {
        Self
    }

    fn parse_args(&self, arguments: Value) -> Result<CoverageTarget> {
        let args: RustRunCoverageArgs =
            serde_json::from_value(arguments).map_err(|source| AppError::InvalidToolArguments {
                tool: self.definition().function.name.to_owned(),
                source,
            })?;
        let raw_path = args.path.unwrap_or_else(|| ".".to_owned());
        let requested_path = PathBuf::from(raw_path);
        self.resolve_target(&requested_path)
    }

    fn resolve_target(&self, requested_path: &Path) -> Result<CoverageTarget> {
        if !requested_path.exists() {
            return Err(AppError::Message(format!(
                "path does not exist: {}",
                requested_path.display()
            )));
        }

        let (display_path, manifest_path) = if requested_path.is_dir() {
            let manifest_path = requested_path.join("Cargo.toml");
            (requested_path.to_path_buf(), manifest_path)
        } else if requested_path
            .file_name()
            .is_some_and(|name| name == "Cargo.toml")
        {
            let display_path = requested_path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from("."));
            (display_path, requested_path.to_path_buf())
        } else {
            return Err(AppError::Message(format!(
                "coverage path must be a repository directory or Cargo.toml file: {}",
                requested_path.display()
            )));
        };

        if !manifest_path.exists() {
            return Err(AppError::Message(format!(
                "no Cargo.toml found for coverage target: {}",
                manifest_path.display()
            )));
        }

        Ok(CoverageTarget {
            display_path: display_path
                .canonicalize()
                .unwrap_or_else(|_| display_path.to_path_buf()),
            manifest_path: manifest_path
                .canonicalize()
                .unwrap_or_else(|_| manifest_path.to_path_buf()),
        })
    }

    fn build_config(&self, target: &CoverageTarget) -> Config {
        let mut config = Config::default();
        config.set_manifest(target.manifest_path.clone());
        config.set_engine(TraceEngine::Ptrace);
        config
    }
}

#[async_trait]
impl Tool for RustRunCoverageTool {
    fn definition(&self) -> ToolDefinition {
        rust_run_coverage_tool_definition()
    }

    async fn preview(&self, arguments: Value) -> Result<ToolPreview> {
        let target = self.parse_args(arguments)?;
        let display = Some(format!(
            "Running Rust coverage under {} with Tarpaulin using the ptrace engine.",
            target.display_path.display()
        ));

        Ok(ToolPreview {
            display,
            is_noop: false,
            requires_confirmation: false,
        })
    }

    async fn execute(&self, arguments: Value) -> Result<ToolExecutionOutcome> {
        let target = self.parse_args(arguments)?;
        let config = self.build_config(&target);

        let join_result = tokio::task::spawn_blocking(move || trace(&[config])).await;
        let trace_result = join_result.map_err(|source| {
            AppError::Message(format!("coverage task failed to join: {source}"))
        })?;
        let (tracemap, _) = trace_result
            .map_err(|source| AppError::Message(format!("failed to run Tarpaulin: {source}")))?;

        let covered = tracemap.total_covered();
        let coverable = tracemap.total_coverable();
        let coverage_percent = tracemap.coverage_percentage() * 100.0;

        let display = format!(
            "Coverage for {} via Tarpaulin (ptrace):\n- covered lines: {}\n- coverable lines: {}\n- coverage: {:.2}%",
            target.display_path.display(),
            covered,
            coverable,
            coverage_percent
        );

        Ok(ToolExecutionOutcome { display })
    }
}
