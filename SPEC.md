# B-Bot Specification

## 1. Overview

B-Bot is a personal AI command-line assistant written in Rust.

B-Bot translates natural-language requests into calls to a fixed set of typed Rust tools. It is local-first and uses local inference through Ollama in v1. B-Bot is intended for a single user and is optimized for control, predictability, and auditability rather than broad autonomy.

B-Bot is not primarily a shell command generator. The model chooses from capabilities that the Rust application explicitly exposes. The Rust application validates and executes those capabilities.

## 2. Product Definition

B-Bot is a natural-language interface to a controlled Rust tool system.

The model is responsible for:

1. Understanding the user request
2. Selecting one or more known tools
3. Filling typed arguments
4. Revising a plan based on tool output when needed

The Rust application is responsible for:

1. Defining the tool registry
2. Enforcing capability boundaries
3. Validating arguments and targets
4. Applying risk policy
5. Executing tool logic
6. Rendering results in the terminal

## 3. Goals

### 3.1 Primary Goals

1. Accept natural-language instructions in a terminal.
2. Convert those instructions into typed tool invocation plans.
3. Execute only capabilities explicitly implemented in Rust.
4. Preserve complete developer control over what B-Bot can do.
5. Run with local inference by default.
6. Keep behavior auditable and predictable.
7. Avoid automatic execution when user intent or tool fit is too ambiguous.

### 3.2 Secondary Goals

1. Support multi-step workflows.
2. Support replanning after observing tool output.
3. Support backend replacement without rewriting core logic.
4. Allow future expansion into memory or retrieval without loosening execution control.
5. Capture tooling gaps locally so future B-Bot versions are easier to iterate on.

## 4. Non-Goals

B-Bot v1 will not:

1. Execute arbitrary shell text produced by the model.
2. Act as a general autonomous agent.
3. Depend on cloud inference.
4. Support multi-user operation.
5. Support dynamic user-defined tools at runtime.
6. Target macOS or Windows.
7. Build cross-platform abstractions for future portability.
8. Require the user to remember or reconstruct multi-step CLI sequences to accomplish supported goals.

## 5. Platform Assumption

Linux is the only supported platform for B-Bot.

This is an explicit product decision, not a temporary implementation shortcut. B-Bot may use Linux-specific assumptions, conventions, and APIs where useful. Portability to macOS or Windows is a non-goal.

## 6. Product Principles

### 6.1 Tool-First

B-Bot should invoke Rust tools, not generate shell by default.

### 6.2 Typed Over Free-Form

The planner should return structured tool calls with typed arguments.

### 6.3 Capabilities Are Code

If a behavior is not implemented and registered in Rust, B-Bot cannot do it.

### 6.4 Validate Before Execute

Every planned tool call must pass deterministic validation before execution.

### 6.5 Local-First

Inference should run locally through Ollama in v1.

### 6.6 User Visibility

B-Bot should show which tools it plans to invoke, with which arguments, and why.

### 6.7 Goal-Oriented Abstraction

B-Bot should hide low-level CLI choreography from the user.

If a user expresses a high-level goal, B-Bot should prefer a single goal-oriented tool that encapsulates the required internal operations rather than exposing command pipelines, shell composition, or command-by-command sequencing.

## 7. Core Use Cases

B-Bot v1 should focus on a narrow, high-confidence set of developer workflows:

1. Filesystem inspection
   Example: "Find the ten largest files in this directory."

2. Repository inspection
   Example: "List branches merged into main."

3. Rust project workflows
   Example: "Run tests for the current crate."

4. Search and diagnostics
   Example: "Search this repo for TODOs related to auth."

5. Process inspection
   Example: "Show me what is listening on port 3000."

6. Docker container lifecycle management
   Example: "Delete all running Docker containers."
   This use case exists specifically to hide Docker CLI details, flags, and lifecycle sequencing from the user. B-Bot should let the user express the goal in natural language without needing to remember whether containers must be listed first, stopped first, removed afterward, or handled through a per-repo `docker compose down`.

7. Controlled local actions
   Example: "Clean the target directory for this project."

## 8. High-Level Architecture

B-Bot v1 consists of:

1. CLI frontend
2. Session manager
3. Context collector
4. Planner
5. Tool registry
6. Validator and policy engine
7. Tool executor
8. Model backend abstraction
9. Logging and history store

```text
User Request
    ->
CLI Frontend
    ->
Context Collector
    ->
Planner
    ->
Structured ToolPlan
    ->
Validator / Policy Engine
    ->
Approved Tool Calls
    ->
Tool Executor
    ->
Tool Output
    ->
Session History / Optional Replan
```

## 9. Tool System

### 9.1 Tool Registry

B-Bot shall maintain a registry of tools explicitly implemented in Rust.

Each tool shall define:

1. A stable name
2. A typed input schema
3. A typed output schema
4. A risk classification function
5. Validation rules
6. Execution logic

The registry is B-Bot’s capability boundary.

### 9.2 Tool Interface

The internal shape should be roughly:

```rust
pub trait Tool: Send + Sync {
    type Input;
    type Output;

    fn name(&self) -> &'static str;
    fn risk(&self, input: &Self::Input) -> RiskLevel;
    fn validate(&self, input: &Self::Input, ctx: &ValidationContext) -> Result<(), ToolError>;
    async fn execute(&self, input: Self::Input, ctx: &ExecutionContext) -> Result<Self::Output, ToolError>;
}
```

Type erasure is acceptable for registry dispatch, but tool implementations should remain typed.

### 9.3 Initial Tool Domains

The initial tool set should come from a small number of domains:

1. `fs.*`
2. `git.*`
3. `cargo.*`
4. `search.*`
5. `process.*`
6. `docker.*`

### 9.4 Example Tool Calls

```rust
pub enum ToolCall {
    FsList {
        path: PathBuf,
        limit: Option<usize>,
    },
    FsFindLargeFiles {
        path: PathBuf,
        limit: usize,
    },
    GitStatus {
        repo: PathBuf,
    },
    GitMergedBranches {
        repo: PathBuf,
        base: String,
    },
    CargoTest {
        path: PathBuf,
        package: Option<String>,
        test_name: Option<String>,
    },
    SearchRipgrep {
        path: PathBuf,
        pattern: String,
        glob: Option<String>,
    },
    ProcessListPort {
        port: u16,
    },
    DockerRemoveAllRunningContainers {
        stop_timeout_secs: u64,
        remove_anonymous_volumes: bool,
    },
}
```

These examples define the style B-Bot should aim for: high-level enough to be useful, narrow enough to validate.

For Docker in particular, the tool layer should encapsulate Docker’s container lifecycle rules rather than expose raw command syntax. The user should be able to express goals such as "delete all running Docker containers" without having to remember the exact combination of listing, stopping, removing, force flags, volume-removal flags, or compose-related commands.

Goal-oriented tools are allowed to perform multiple internal operations to satisfy a single user-visible action. For example, `docker.remove_all_running_containers` may internally list running containers, stop them gracefully, and then remove them, while still presenting a single capability to the user.

## 10. Planning Contract

The model must return a structured `ToolPlan`. B-Bot must not interpret unconstrained natural-language output as executable instructions.

### 10.1 Plan Structure

```rust
pub struct ToolPlan {
    pub intent: String,
    pub needs_confirmation: bool,
    pub needs_clarification: bool,
    pub clarification_question: Option<String>,
    pub intent_confidence: f32,
    pub capability_confidence: f32,
    pub planner_confidence: f32,
    pub ambiguity_summary: Option<String>,
    pub capability_gap: Option<CapabilityGap>,
    pub steps: Vec<PlannedToolStep>,
}

pub struct PlannedToolStep {
    pub id: String,
    pub tool_name: String,
    pub arguments: serde_json::Value,
    pub reason: String,
    pub risk: RiskLevel,
    pub requires_confirmation: bool,
}

pub struct CapabilityGap {
    pub missing_capability_summary: String,
    pub suggested_tools: Vec<String>,
    pub suggested_shell_equivalent: Option<String>,
}
```

### 10.2 Planning Rules

1. The planner may only choose known tools.
2. The planner must provide arguments that conform to the tool schema.
3. The planner should ask for clarification instead of guessing destructive targets.
4. The planner should prefer a small number of concrete tool calls over vague multi-step behavior.
5. If the planner is not highly confident that the selected tools will accomplish the user’s goal, it must ask for confirmation or clarification before execution.
6. High ambiguity in user intent, target resolution, or tool fit must prevent auto-execution.
7. If the current tool set cannot likely satisfy the request, the planner should return a capability gap instead of forcing a weak tool match.

### 10.3 Ambiguity and Confidence Threshold

B-Bot must treat ambiguity as a first-class execution constraint.

This includes:

1. Ambiguity in the user’s intent
2. Ambiguity in the target path, repository, process, or workspace
3. Uncertainty about whether available tools can actually accomplish the request
4. Partial matches where a tool could do something related, but not reliably what was asked

The planner should estimate its confidence that the proposed tool plan is both:

1. Interpreting the user request correctly
2. Likely to accomplish the user’s goal with the currently available tool set

If that confidence is below a configured threshold, B-Bot must not silently execute the plan. It must either:

1. Ask a clarification question
2. Ask for explicit confirmation
3. Report that the request is not well covered by current tools

### 10.4 Operational Confidence Rules

For implementation purposes, "highly confident" means B-Bot has both:

1. High confidence that it understood the user’s intent correctly
2. High confidence that the selected tools can likely achieve that intent

B-Bot should represent these as normalized scores from `0.0` to `1.0`:

1. `intent_confidence`
2. `capability_confidence`

`planner_confidence` may be stored as a convenience aggregate, but auto-execution must consider the two underlying dimensions separately.

The default interpretation of those scores should be:

1. `>= 0.85`
   High confidence. The plan may proceed under normal risk-based policy.

2. `0.70` to `< 0.85`
   Borderline confidence. B-Bot should ask for explicit confirmation before execution, even for nominally low-risk steps.

3. `< 0.70`
   Insufficient confidence. B-Bot should ask a clarification question or report a capability gap instead of executing.

Auto-execution is only allowed when all of the following are true:

1. `intent_confidence >= min_planner_confidence`
2. `capability_confidence >= min_planner_confidence`
3. `needs_clarification == false`
4. `capability_gap.is_none()`
5. `ambiguity_summary` is empty or absent
6. All planned steps pass validation and policy checks
7. The resulting risk policy allows auto-execution

If B-Bot is unsure what the user means, it should ask a clarification question.

If B-Bot understands what the user means but is unsure the available tools can achieve it, it should ask for confirmation or report a tooling gap rather than pretending it can fully satisfy the request.

## 11. Policy and Risk Model

The policy engine sits between planning and execution.

It shall:

1. Check whether a tool is enabled
2. Validate argument schema and semantic constraints
3. Enforce allowed roots and target boundaries
4. Apply confirmation policy by risk level
5. Block disallowed tools or argument patterns
6. Enforce a minimum planner-confidence threshold before auto-execution

### 11.1 Risk Levels

1. `Low`
   Read-only inspection with no external side effects.

2. `Medium`
   Local modifications inside approved roots, local builds, or notable resource consumption.

3. `High`
   Process termination, package-state changes, network access, or state changes outside approved roots.

4. `Critical`
   Broad destructive impact, privilege escalation, or irreversible operations.

### 11.2 Confirmation Rules

1. `Low` may auto-run when enabled by config.
2. `Medium` should default to confirmation.
3. `High` must require confirmation.
4. `Critical` must require confirmation and may be blocked entirely.
5. Any step planned under a low-confidence or high-ambiguity interpretation must require confirmation or clarification even if its nominal risk is `Low`.

### 11.3 Example Policy Constraints

1. Allow `cargo.*` only inside approved workspace roots.
2. Block deletion tools outside the current project.
3. Disable network-capable tools entirely in v1 unless explicitly enabled.
4. Require confirmation for any process-kill operation.
5. Block privilege escalation by default.
6. Require confirmation for global Docker cleanup operations such as removing all running containers.

## 12. Execution Model

### 12.1 Primary Execution Path

B-Bot shall execute capabilities through Rust tool implementations.

Those implementations may internally:

1. Use standard Rust APIs
2. Read or write the filesystem
3. Spawn child processes such as `git`, `cargo`, or `rg`
4. Query the local environment

The key constraint is that the model invokes a tool, not an arbitrary command string.

Tools may internally perform multiple sub-operations when that is the correct way to satisfy a single high-level user goal. This is expected behavior, not a fallback.

### 12.2 Process Spawning Inside Tools

Some tools will wrap existing CLI programs. That is acceptable and expected.

In those cases:

1. Process spawning is owned by Rust code.
2. Arguments are constructed by B-Bot code, not by raw model-authored shell text.
3. Validation occurs before process execution.
4. Multi-step workflows should be encapsulated inside the tool implementation rather than surfaced to the user as separate command steps.

### 12.3 Shell Execution

Arbitrary shell execution is not part of the B-Bot v1 design.

If a shell bridge ever exists later, it must be:

1. An explicit tool
2. Disabled by default
3. Strongly gated by policy
4. Clearly treated as lower-trust than typed tools

### 12.4 Timeouts and Output

Each tool should support:

1. Soft timeout for user feedback
2. Hard timeout for termination
3. Captured or streamed output as appropriate

### 12.5 Replanning

When a tool fails, B-Bot may:

1. Present the failure and stop
2. Ask the model for a revised tool plan based on observed output
3. Require confirmation again if the new step is not low risk

### 12.6 Unaccomplishable Requests

If B-Bot determines that the current tool set cannot likely accomplish the request, it shall:

1. Avoid executing speculative or weakly matched tools by default
2. Tell the user that the current capability set is insufficient
3. Suggest useful missing tool types when it has a credible idea
4. Optionally suggest a shell or CLI equivalent when that would help the user understand the missing capability
5. Persist a tooling-gap record locally for future iteration

## 13. Model Strategy

### 13.1 Default Backend

The default backend is Ollama running locally over HTTP.

Reasons:

1. Local inference is a core requirement.
2. Ollama handles model lifecycle and serving.
3. Structured outputs fit the typed tool-planning model well.

### 13.2 Backend Abstraction

B-Bot shall define an internal trait similar to:

```rust
pub trait InferenceBackend {
    async fn plan(&self, request: PlanRequest) -> Result<ToolPlan, BackendError>;
    async fn replan(&self, request: ReplanRequest) -> Result<ToolPlan, BackendError>;
    async fn health(&self) -> Result<BackendHealth, BackendError>;
}
```

The core system must depend on this trait, not on a framework-specific agent abstraction.

### 13.3 `rig` Position

`rig` may be useful later as an adapter or convenience layer, but it should not be the core runtime abstraction in v1.

The main reason is control: B-Bot’s value lies in its owned tool system and owned policy layer.

### 13.4 Candle Position

Candle is not the initial inference path.

It may become a future backend if B-Bot later needs:

1. In-process inference without Ollama
2. Custom model loading or sampling
3. Tighter packaging and runtime control

## 14. Context Collection

B-Bot may provide the planner with selected context such as:

1. Current working directory
2. Active project metadata
3. Presence of common tools like `git`, `cargo`, and `rg`
4. Enabled tool names and short descriptions
5. Recent tool outputs from the current session

Context collection should be explicit and minimal. B-Bot should avoid sending large file trees, full environment dumps, or sensitive data unless required.

## 15. CLI Behavior

B-Bot v1 should support:

The CLI binary name should be `bbot`.

1. Interactive mode
   `bbot`

2. One-shot mode
   `bbot "run tests for this crate"`

3. Dry-run mode
   `bbot --dry-run "find the largest files here"`

4. Explain mode
   `bbot --explain "show active network connections"`

CLI output should clearly separate:

1. User request
2. Generated tool plan
3. Risk assessment
4. Confirmation prompt
5. Tool output
6. Final summary

Example confirmation:

```text
Step 1 [Medium Risk]
Tool: cargo.test
Arguments: { "path": "/path/to/project", "package": null, "test_name": null }
Run this step? [y/N]
```

## 16. Configuration

B-Bot should load configuration from:

```text
~/.config/bbot/config.toml
```

Configuration areas:

1. Default model name
2. Ollama host URL
3. Auto-run policy by risk level
4. Enabled and disabled tools
5. Allowed workspace roots
6. Output verbosity
7. History retention
8. Timeout defaults
9. Minimum planner confidence for auto-execution
10. Optional confirmation band for borderline planner confidence
11. State directory and tooling-gap log path

Example:

```toml
[model]
backend = "ollama"
name = "qwen2.5-coder:14b"
base_url = "http://127.0.0.1:11434"

[policy]
auto_run_low_risk = true
enabled_tools = ["fs.list", "fs.find_large_files", "git.status", "cargo.test", "search.ripgrep"]
allowed_roots = ["/path/to/workspaces"]
min_planner_confidence = 0.85
confirmation_confidence_floor = 0.70

[execution]
default_timeout_secs = 30
stream_output = true

[state]
dir = "/path/to/state/bbot"
tooling_gap_log = "/path/to/state/bbot/tooling-gaps.jsonl"
```

## 17. State and Tooling-Gap Capture

B-Bot should persist local state under:

```text
~/.local/state/bbot/
```

At minimum, B-Bot should maintain a tooling-gap log for unmet or weakly covered requests.

When B-Bot cannot likely satisfy a request with the current tool set, it shall record a structured local entry containing:

1. Timestamp
2. Original user request
3. Planner intent summary
4. Ambiguity or confidence notes
5. Missing capability summary
6. Suggested future tool names or categories, if any
7. Suggested shell or CLI equivalent, if any
8. Current working directory or relevant execution context

The purpose of this store is product iteration, not automatic execution. B-Bot should use it to make future tool additions easier to prioritize.

## 18. Security Constraints

1. Tool output and file contents are untrusted model context.
2. Full environment variables should not be exposed to the model by default.
3. Paths should be normalized before validation and execution.
4. Privileged operations must never auto-run.
5. The model must not be able to create new tools or alter policy at runtime.

## 19. Suggested Rust Module Layout

```text
src/
  main.rs
  cli.rs
  app.rs
  session.rs
  config.rs
  planner/
    mod.rs
    prompt.rs
    schema.rs
  backend/
    mod.rs
    ollama.rs
  tools/
    mod.rs
    registry.rs
    fs.rs
    git.rs
    cargo.rs
    search.rs
    process.rs
  policy/
    mod.rs
    validator.rs
    risk.rs
  history/
    mod.rs
    store.rs
    gap_store.rs
  output/
    mod.rs
    render.rs
  error.rs
```

## 20. Dependencies

Likely crates for v1:

1. `clap`
2. `tokio`
3. `reqwest`
4. `serde`
5. `serde_json`
6. `toml`
7. `tracing`
8. `tracing-subscriber`
9. `thiserror` or `anyhow`
10. `schemars`

## 21. Delivery Phases

### Phase 1

1. CLI entrypoint
2. Ollama integration
3. Typed `ToolPlan`
4. Tool registry
5. Validation and risk policy
6. Dry-run mode
7. Execution of low-risk tools
8. Tooling-gap capture for unmet requests

### Phase 2

1. Confirmation prompts
2. Session history
3. Output streaming
4. Replanning on failure
5. Expanded tool catalog

### Phase 3

1. Richer context collection
2. More granular policy
3. Optional alternate backends
4. Optional Candle backend exploration

## 22. Acceptance Criteria for v1

B-Bot v1 is complete when:

1. A user can enter a natural-language request and receive a structured tool plan.
2. The plan only references tools explicitly registered in Rust.
3. The plan is validated before any execution occurs.
4. Low-risk tools can execute successfully through the CLI.
5. Medium-risk and above tools require confirmation.
6. Tool output is surfaced cleanly to the user.
7. Ollama is the default local inference backend.
8. Arbitrary shell execution is not part of the normal runtime path.
9. Ambiguous or low-confidence requests are not auto-executed.
10. Requests that current tools cannot satisfy produce useful gap suggestions and a persistent local tooling-gap record.
11. Auto-execution requires both intent confidence and capability confidence to meet the configured threshold.

## 23. Recommended Initial Position

For implementation start:

1. Use direct Ollama HTTP integration.
2. Keep `rig` out of the core runtime.
3. Reserve Candle for a future backend.
4. Make B-Bot a typed Rust tool orchestrator, not a command generator.
5. Start with a narrow tool set around filesystem inspection, git inspection, cargo workflows, search, and process inspection.
6. Include a small `docker.*` tool set early, because hiding Docker CLI complexity is a primary user value proposition.
