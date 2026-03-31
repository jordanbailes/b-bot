use async_trait::async_trait;
use bollard::Docker;
use bollard::models::ContainerSummary;
use bollard::query_parameters::{
    ListContainersOptionsBuilder, RemoveContainerOptionsBuilder, StopContainerOptionsBuilder,
};
use serde::Deserialize;
use serde_json::Value;

use crate::error::{AppError, Result};
use crate::ollama::ToolDefinition;
use crate::tools::{Tool, ToolExecutionOutcome, ToolPreview, docker_remove_all_tool_definition};

#[derive(Debug, Deserialize)]
struct DockerRemoveAllRunningContainersArgs {
    #[serde(default = "default_stop_timeout_secs")]
    stop_timeout_secs: i32,
    #[serde(default)]
    remove_anonymous_volumes: bool,
}

fn default_stop_timeout_secs() -> i32 {
    10
}

pub struct DockerRemoveAllRunningContainersTool;

impl DockerRemoveAllRunningContainersTool {
    pub fn new() -> Self {
        Self
    }

    async fn running_containers(&self) -> Result<Vec<ContainerSummary>> {
        let docker = Docker::connect_with_socket_defaults()?;
        let options = ListContainersOptionsBuilder::default().all(false).build();
        Ok(docker.list_containers(Some(options)).await?)
    }
}

#[async_trait]
impl Tool for DockerRemoveAllRunningContainersTool {
    fn definition(&self) -> ToolDefinition {
        docker_remove_all_tool_definition()
    }

    async fn preview(&self, arguments: Value) -> Result<ToolPreview> {
        let args: DockerRemoveAllRunningContainersArgs = serde_json::from_value(arguments)
            .map_err(|source| AppError::InvalidToolArguments {
                tool: self.definition().function.name.to_owned(),
                source,
            })?;
        let containers = self.running_containers().await?;

        if containers.is_empty() {
            return Ok(ToolPreview {
                display: Some("No running Docker containers found.".to_owned()),
                is_noop: true,
                requires_confirmation: false,
            });
        }

        let mut lines = vec![format!(
            "This will stop and remove {} running container(s) with a {} second stop timeout{}:",
            containers.len(),
            args.stop_timeout_secs,
            if args.remove_anonymous_volumes {
                " and it will remove anonymous volumes"
            } else {
                ""
            }
        )];

        for container in &containers {
            lines.push(format!("- {}", container_label(container)));
        }

        Ok(ToolPreview {
            display: Some(lines.join("\n")),
            is_noop: false,
            requires_confirmation: true,
        })
    }

    async fn execute(&self, arguments: Value) -> Result<ToolExecutionOutcome> {
        let args: DockerRemoveAllRunningContainersArgs = serde_json::from_value(arguments)
            .map_err(|source| AppError::InvalidToolArguments {
                tool: self.definition().function.name.to_owned(),
                source,
            })?;
        let docker = Docker::connect_with_socket_defaults()?;
        let containers = self.running_containers().await?;

        if containers.is_empty() {
            return Ok(ToolExecutionOutcome {
                display: "No running Docker containers found.".to_owned(),
            });
        }

        let mut removed = Vec::new();
        let mut failures = Vec::new();

        for container in containers {
            let label = container_label(&container);
            let handle = match container_handle(&container) {
                Some(handle) => handle,
                None => {
                    failures.push(format!("{label}: missing Docker handle"));
                    continue;
                }
            };

            let stop_options = StopContainerOptionsBuilder::default()
                .t(args.stop_timeout_secs)
                .build();
            if let Err(err) = docker.stop_container(&handle, Some(stop_options)).await {
                failures.push(format!("{label}: failed to stop: {err}"));
                continue;
            }

            let remove_options = RemoveContainerOptionsBuilder::default()
                .force(false)
                .v(args.remove_anonymous_volumes)
                .build();
            if let Err(err) = docker.remove_container(&handle, Some(remove_options)).await {
                failures.push(format!("{label}: failed to remove: {err}"));
                continue;
            }

            removed.push(label);
        }

        let mut lines = Vec::new();
        lines.push(format!(
            "Removed {} running Docker container(s).",
            removed.len()
        ));

        if !removed.is_empty() {
            lines.push("Removed:".to_owned());
            for item in removed {
                lines.push(format!("- {item}"));
            }
        }

        if !failures.is_empty() {
            lines.push(String::new());
            lines.push("Failures:".to_owned());
            for item in failures {
                lines.push(format!("- {item}"));
            }
        }

        Ok(ToolExecutionOutcome {
            display: lines.join("\n"),
        })
    }
}

fn container_handle(container: &ContainerSummary) -> Option<String> {
    container.id.clone().or_else(|| {
        container
            .names
            .as_ref()
            .and_then(|names| names.first())
            .map(|name| name.trim_start_matches('/').to_owned())
    })
}

fn container_label(container: &ContainerSummary) -> String {
    let name = container
        .names
        .as_ref()
        .and_then(|names| names.first())
        .map(|name| name.trim_start_matches('/').to_owned())
        .unwrap_or_else(|| "<unnamed>".to_owned());
    let id = container
        .id
        .as_ref()
        .map(|id| short_id(id))
        .unwrap_or_else(|| "unknown".to_owned());
    format!("{name} ({id})")
}

fn short_id(id: &str) -> String {
    id.chars().take(12).collect()
}
