use std::{
    collections::HashMap,
    fmt,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use anyhow::{Context, Result as AnyResult, anyhow, bail};
use bon::Builder;
use clap::ValueEnum;
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{ChildStdin, ChildStdout, Command},
    sync::{Mutex, broadcast, oneshot},
};

use crate::{
    error::Result,
    planner::{
        ChatResponse, Message, PlannerBackend, SYSTEM_PROMPT, ToolDefinition, parse_chat_response,
    },
};

const CODEX_CLIENT_NAME: &str = "bbot";
const CODEX_CLIENT_TITLE: &str = "B-Bot Codex Planner";
const CODEX_SERVICE_NAME: &str = "bee_bot_planner";
const PREFERRED_MODELS: &[&str] = &["gpt-5.4", "gpt-5.4-mini", "gpt-5.1", "gpt-5.1-mini"];

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq, ValueEnum)]
pub enum ReasoningEffort {
    Minimal,
    Low,
    #[default]
    Medium,
    High,
}

impl fmt::Display for ReasoningEffort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        })
    }
}

#[derive(Debug, Clone, Builder)]
pub struct CodexPlannerConfig {
    #[builder(into)]
    pub binary: String,
    pub cwd: PathBuf,
    pub model: Option<String>,
    #[builder(default)]
    pub effort: ReasoningEffort,
    #[builder(default)]
    pub network_access: bool,
}

pub struct CodexPlannerBackend {
    config: CodexPlannerConfig,
}

impl CodexPlannerBackend {
    pub fn new(config: CodexPlannerConfig) -> Self {
        Self { config }
    }
}

#[async_trait::async_trait]
impl PlannerBackend for CodexPlannerBackend {
    async fn plan(&self, prompt: &str, tools: &[ToolDefinition]) -> Result<ChatResponse> {
        let prompt = build_planner_prompt(prompt, tools)?;
        let client = AppServerClient::spawn(&self.config.binary).await?;
        let mut notifications = client.subscribe();

        client
            .initialize(
                CODEX_CLIENT_NAME,
                CODEX_CLIENT_TITLE,
                env!("CARGO_PKG_VERSION"),
            )
            .await?;

        ensure_logged_in(&client, &mut notifications).await?;

        let runtime = resolve_runtime_defaults(&client).await;
        let model = resolve_model(&client, self.config.model.as_deref()).await?;
        let thread_id = start_thread(&client, &model, &self.config.cwd, &runtime).await?;
        let turn_id =
            start_turn(&client, &thread_id, &model, &prompt, &self.config, &runtime).await?;
        let output = collect_turn_output(&mut notifications, &turn_id).await?;

        Ok(parse_codex_output(&output)?)
    }
}

fn build_planner_prompt(prompt: &str, tools: &[ToolDefinition]) -> Result<String> {
    let tools_json = serde_json::to_string_pretty(tools)?;
    Ok(format!(
        "{SYSTEM_PROMPT}\n\n\
Additional Codex planner rules:\n\
- Do not run shell commands.\n\
- Do not edit files.\n\
- Do not inspect the repository.\n\
- Do not browse the web.\n\
- Decide only from the user request and the provided tool definitions.\n\
- Return exactly one JSON object and nothing else.\n\
- The JSON must have the shape {{\"message\": {{\"content\": string | null, \"tool_calls\": array | null}}}}.\n\
- Set exactly one of `message.content` or `message.tool_calls`.\n\
- If `message.tool_calls` is set, it must contain exactly one tool call with the shape {{\"type\": \"function\", \"function\": {{\"name\": string, \"arguments\": object}}}}.\n\
- Do not wrap the JSON in markdown fences.\n\n\
Available tools:\n{tools_json}\n\n\
User request:\n{prompt}"
    ))
}

fn parse_codex_output(output: &str) -> Result<ChatResponse> {
    let normalized = strip_markdown_fences(output).trim().to_owned();
    if normalized.is_empty() {
        return parse_chat_response("Codex", &normalized);
    }

    match parse_chat_response("Codex", &normalized) {
        Ok(response) => Ok(response),
        Err(_) => Ok(ChatResponse {
            message: Message::assistant(normalized),
        }),
    }
}

fn strip_markdown_fences(output: &str) -> &str {
    let trimmed = output.trim();
    if !trimmed.starts_with("```") {
        return trimmed;
    }

    let without_prefix = trimmed
        .trim_start_matches("```json")
        .trim_start_matches("```JSON")
        .trim_start_matches("```");
    without_prefix
        .trim()
        .strip_suffix("```")
        .map(str::trim)
        .unwrap_or(trimmed)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ClientMethod {
    Initialize,
    Initialized,
    AccountRead,
    AccountLoginStart,
    ConfigRequirementsRead,
    ModelList,
    ThreadStart,
    TurnStart,
}

impl fmt::Display for ClientMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Initialize => "initialize",
            Self::Initialized => "initialized",
            Self::AccountRead => "account/read",
            Self::AccountLoginStart => "account/login/start",
            Self::ConfigRequirementsRead => "configRequirements/read",
            Self::ModelList => "model/list",
            Self::ThreadStart => "thread/start",
            Self::TurnStart => "turn/start",
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ServerMethod {
    AccountLoginCompleted,
    ItemAgentMessageDelta,
    ItemCompleted,
    TurnCompleted,
    Error,
    Unknown(String),
}

impl ServerMethod {
    fn from_wire(value: &str) -> Self {
        match value {
            "account/login/completed" => Self::AccountLoginCompleted,
            "item/agentMessage/delta" => Self::ItemAgentMessageDelta,
            "item/completed" => Self::ItemCompleted,
            "turn/completed" => Self::TurnCompleted,
            "error" => Self::Error,
            other => Self::Unknown(other.to_owned()),
        }
    }
}

impl fmt::Display for ServerMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AccountLoginCompleted => f.write_str("account/login/completed"),
            Self::ItemAgentMessageDelta => f.write_str("item/agentMessage/delta"),
            Self::ItemCompleted => f.write_str("item/completed"),
            Self::TurnCompleted => f.write_str("turn/completed"),
            Self::Error => f.write_str("error"),
            Self::Unknown(other) => f.write_str(other),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum ApprovalPolicy {
    #[default]
    Never,
    OnFailure,
    Untrusted,
    OnRequest,
}

impl ApprovalPolicy {
    const PREFERRED: [Self; 4] = [
        Self::Never,
        Self::OnFailure,
        Self::Untrusted,
        Self::OnRequest,
    ];

    fn from_wire(value: &str) -> Option<Self> {
        match value {
            "never" => Some(Self::Never),
            "on-failure" => Some(Self::OnFailure),
            "untrusted" => Some(Self::Untrusted),
            "on-request" => Some(Self::OnRequest),
            _ => None,
        }
    }
}

impl fmt::Display for ApprovalPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Never => "never",
            Self::OnFailure => "on-failure",
            Self::Untrusted => "untrusted",
            Self::OnRequest => "on-request",
        })
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum ThreadSandboxMode {
    WorkspaceWrite,
    #[default]
    ReadOnly,
    DangerFullAccess,
}

impl ThreadSandboxMode {
    const PREFERRED: [Self; 3] = [Self::ReadOnly, Self::WorkspaceWrite, Self::DangerFullAccess];

    fn from_wire(value: &str) -> Option<Self> {
        match value {
            "workspace-write" => Some(Self::WorkspaceWrite),
            "read-only" => Some(Self::ReadOnly),
            "danger-full-access" => Some(Self::DangerFullAccess),
            _ => None,
        }
    }

    fn turn_policy_json(self, cwd: &Path, network_access: bool) -> Value {
        match self {
            Self::ReadOnly => json!({
                "type": "readOnly",
                "access": { "type": "fullAccess" },
            }),
            Self::DangerFullAccess => json!({
                "type": "dangerFullAccess",
            }),
            Self::WorkspaceWrite => json!({
                "type": "workspaceWrite",
                "writableRoots": [cwd.display().to_string()],
                "networkAccess": network_access,
            }),
        }
    }
}

impl fmt::Display for ThreadSandboxMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::WorkspaceWrite => "workspace-write",
            Self::ReadOnly => "read-only",
            Self::DangerFullAccess => "danger-full-access",
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LoginType {
    ChatGpt,
}

impl fmt::Display for LoginType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::ChatGpt => "chatgpt",
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum AccountType {
    ChatGpt,
    ApiKey,
    Unknown(String),
}

impl AccountType {
    fn from_wire(value: &str) -> Self {
        match value {
            "chatgpt" => Self::ChatGpt,
            "apiKey" => Self::ApiKey,
            other => Self::Unknown(other.to_owned()),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InputItemType {
    Text,
}

impl fmt::Display for InputItemType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("text")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SummaryMode {
    Concise,
}

impl fmt::Display for SummaryMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("concise")
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ThreadItemType {
    AgentMessage,
    CommandExecution,
    FileChange,
    Unknown(String),
}

impl ThreadItemType {
    fn from_wire(value: &str) -> Self {
        match value {
            "agentMessage" => Self::AgentMessage,
            "commandExecution" => Self::CommandExecution,
            "fileChange" => Self::FileChange,
            other => Self::Unknown(other.to_owned()),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ItemStatus {
    InProgress,
    Completed,
    Failed,
    Declined,
    Interrupted,
    Unknown(String),
}

impl ItemStatus {
    fn from_wire(value: &str) -> Self {
        match value {
            "inProgress" => Self::InProgress,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "declined" => Self::Declined,
            "interrupted" => Self::Interrupted,
            other => Self::Unknown(other.to_owned()),
        }
    }
}

impl fmt::Display for ItemStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InProgress => f.write_str("inProgress"),
            Self::Completed => f.write_str("completed"),
            Self::Failed => f.write_str("failed"),
            Self::Declined => f.write_str("declined"),
            Self::Interrupted => f.write_str("interrupted"),
            Self::Unknown(other) => f.write_str(other),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum TurnStatus {
    InProgress,
    Completed,
    Interrupted,
    Failed,
    Unknown(String),
}

impl TurnStatus {
    fn from_wire(value: &str) -> Self {
        match value {
            "inProgress" => Self::InProgress,
            "completed" => Self::Completed,
            "interrupted" => Self::Interrupted,
            "failed" => Self::Failed,
            other => Self::Unknown(other.to_owned()),
        }
    }
}

impl fmt::Display for TurnStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InProgress => f.write_str("inProgress"),
            Self::Completed => f.write_str("completed"),
            Self::Interrupted => f.write_str("interrupted"),
            Self::Failed => f.write_str("failed"),
            Self::Unknown(other) => f.write_str(other),
        }
    }
}

#[derive(Debug, Default)]
struct RuntimeDefaults {
    approval_policy: ApprovalPolicy,
    sandbox_mode: ThreadSandboxMode,
}

type PendingMap = Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>;

struct AppServerClient {
    stdin: Arc<Mutex<ChildStdin>>,
    pending: PendingMap,
    notifications: broadcast::Sender<Value>,
    next_id: AtomicU64,
}

impl AppServerClient {
    async fn spawn(program: &str) -> AnyResult<Self> {
        let mut child = Command::new(program)
            .args(["app-server"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .with_context(|| format!("failed to start `{program} app-server`"))?;

        let stdin = child
            .stdin
            .take()
            .context("failed to capture app-server stdin")?;
        let stdout = child
            .stdout
            .take()
            .context("failed to capture app-server stdout")?;

        let pending = Arc::new(Mutex::new(HashMap::new()));
        let (notifications, _) = broadcast::channel(512);
        let stdin = Arc::new(Mutex::new(stdin));

        tokio::spawn(read_loop(
            stdout,
            pending.clone(),
            notifications.clone(),
            stdin.clone(),
        ));

        tokio::spawn(async move {
            match child.wait().await {
                Ok(status) => eprintln!("codex app-server exited with status {status}"),
                Err(error) => eprintln!("failed to wait on codex app-server: {error}"),
            }
        });

        Ok(Self {
            stdin,
            pending,
            notifications,
            next_id: AtomicU64::new(1),
        })
    }

    fn subscribe(&self) -> broadcast::Receiver<Value> {
        self.notifications.subscribe()
    }

    async fn initialize(&self, name: &str, title: &str, version: &str) -> AnyResult<()> {
        self.request(
            ClientMethod::Initialize,
            json!({
                "clientInfo": {
                    "name": name,
                    "title": title,
                    "version": version,
                }
            }),
        )
        .await?;

        self.notify(ClientMethod::Initialized, json!({})).await
    }

    async fn request(&self, method: ClientMethod, params: Value) -> AnyResult<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();

        self.pending.lock().await.insert(id, tx);

        self.send_json(&json!({
            "method": method.to_string(),
            "id": id,
            "params": params,
        }))
        .await?;

        let response = rx
            .await
            .with_context(|| format!("server closed before replying to `{method}`"))?;

        if let Some(error) = response.get("error") {
            let code = error
                .get("code")
                .and_then(Value::as_i64)
                .unwrap_or_default();
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown JSON-RPC error");
            bail!("`{method}` failed with JSON-RPC error {code}: {message}");
        }

        response
            .get("result")
            .cloned()
            .ok_or_else(|| anyhow!("`{method}` returned no result payload"))
    }

    async fn notify(&self, method: ClientMethod, params: Value) -> AnyResult<()> {
        self.send_json(&json!({
            "method": method.to_string(),
            "params": params,
        }))
        .await
    }

    async fn send_json(&self, message: &Value) -> AnyResult<()> {
        let mut stdin = self.stdin.lock().await;
        let encoded =
            serde_json::to_string(message).context("failed to encode JSON-RPC message")?;
        stdin
            .write_all(encoded.as_bytes())
            .await
            .context("failed to write JSON-RPC message")?;
        stdin
            .write_all(b"\n")
            .await
            .context("failed to terminate JSON-RPC message")?;
        stdin
            .flush()
            .await
            .context("failed to flush app-server stdin")
    }
}

async fn read_loop(
    stdout: ChildStdout,
    pending: PendingMap,
    notifications: broadcast::Sender<Value>,
    stdin: Arc<Mutex<ChildStdin>>,
) {
    let mut lines = BufReader::new(stdout).lines();

    loop {
        let line = match lines.next_line().await {
            Ok(Some(line)) => line,
            Ok(None) => break,
            Err(error) => {
                eprintln!("failed to read app-server stdout: {error}");
                break;
            }
        };

        if line.trim().is_empty() {
            continue;
        }

        let message: Value = match serde_json::from_str(&line) {
            Ok(message) => message,
            Err(error) => {
                eprintln!("skipping invalid JSON-RPC line: {error}: {line}");
                continue;
            }
        };

        if let Some(id) = message.get("id").and_then(Value::as_u64) {
            if message.get("result").is_some() || message.get("error").is_some() {
                if let Some(tx) = pending.lock().await.remove(&id) {
                    let _ = tx.send(message);
                } else {
                    eprintln!("dropping response for unknown request id {id}");
                }
                continue;
            }

            if message.get("method").is_some() {
                if let Err(error) = reject_server_request(&stdin, message).await {
                    eprintln!("failed to reply to server request: {error:#}");
                }
                continue;
            }
        }

        if message.get("method").is_some() {
            let _ = notifications.send(message);
            continue;
        }

        eprintln!("ignoring unexpected app-server payload: {line}");
    }

    let mut pending = pending.lock().await;
    for (_, tx) in pending.drain() {
        let _ = tx.send(json!({
            "error": {
                "code": -32000,
                "message": "codex app-server closed",
            }
        }));
    }
}

async fn reject_server_request(stdin: &Arc<Mutex<ChildStdin>>, message: Value) -> AnyResult<()> {
    let id = message
        .get("id")
        .cloned()
        .context("server request did not include an id")?;
    let method =
        server_method(&message).unwrap_or_else(|| ServerMethod::Unknown(String::from("<unknown>")));

    let response = json!({
        "id": id,
        "error": {
            "code": -32601,
            "message": format!("bbot does not implement server request `{method}`"),
        }
    });

    let encoded =
        serde_json::to_string(&response).context("failed to encode server-request rejection")?;
    let mut stdin = stdin.lock().await;
    stdin
        .write_all(encoded.as_bytes())
        .await
        .context("failed to write server-request rejection")?;
    stdin
        .write_all(b"\n")
        .await
        .context("failed to terminate server-request rejection")?;
    stdin
        .flush()
        .await
        .context("failed to flush server-request rejection")
}

async fn ensure_logged_in(
    client: &AppServerClient,
    notifications: &mut broadcast::Receiver<Value>,
) -> AnyResult<()> {
    let account = client
        .request(ClientMethod::AccountRead, json!({ "refreshToken": false }))
        .await?;

    if let Some(summary) = describe_account(account.get("account").unwrap_or(&Value::Null)) {
        eprintln!("authenticated: {summary}");
        return Ok(());
    }

    let requires_openai_auth = account
        .get("requiresOpenaiAuth")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    if !requires_openai_auth {
        eprintln!("app-server does not require OpenAI authentication for this environment");
        return Ok(());
    }

    let login = client
        .request(
            ClientMethod::AccountLoginStart,
            json!({ "type": LoginType::ChatGpt.to_string() }),
        )
        .await?;

    let auth_url = login
        .get("authUrl")
        .and_then(Value::as_str)
        .context("ChatGPT login did not return `authUrl`")?;
    eprintln!("open this ChatGPT login URL in a browser, then return here:");
    eprintln!("{auth_url}");

    loop {
        match notifications.recv().await {
            Ok(message) => {
                if server_method(&message) != Some(ServerMethod::AccountLoginCompleted) {
                    continue;
                }

                let params = message.get("params").unwrap_or(&Value::Null);
                if params
                    .get("success")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                {
                    break;
                }

                let error = params
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown login error");
                bail!("ChatGPT login failed: {error}");
            }
            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                eprintln!("warning: skipped {skipped} notifications while waiting for login");
            }
            Err(broadcast::error::RecvError::Closed) => {
                bail!("notification stream closed while waiting for login");
            }
        }
    }

    let account = client
        .request(ClientMethod::AccountRead, json!({ "refreshToken": false }))
        .await?;
    let summary = describe_account(account.get("account").unwrap_or(&Value::Null))
        .context("login completed but account/read still returned no account")?;
    eprintln!("authenticated: {summary}");
    Ok(())
}

fn describe_account(account: &Value) -> Option<String> {
    if account.is_null() {
        return None;
    }

    let kind = account
        .get("type")
        .and_then(Value::as_str)
        .map(AccountType::from_wire)
        .unwrap_or_else(|| AccountType::Unknown(String::from("unknown")));
    match kind {
        AccountType::ChatGpt => {
            let email = account
                .get("email")
                .and_then(Value::as_str)
                .unwrap_or("unknown-email");
            let plan = account
                .get("planType")
                .and_then(Value::as_str)
                .unwrap_or("unknown-plan");
            Some(format!("chatgpt {email} ({plan})"))
        }
        AccountType::ApiKey => Some(String::from("OpenAI API key")),
        AccountType::Unknown(other) => Some(other),
    }
}

async fn resolve_runtime_defaults(client: &AppServerClient) -> RuntimeDefaults {
    let response = match client
        .request(ClientMethod::ConfigRequirementsRead, json!({}))
        .await
    {
        Ok(response) => response,
        Err(error) => {
            eprintln!("warning: failed to read config requirements, using defaults: {error:#}");
            return RuntimeDefaults::default();
        }
    };

    let requirements = response.get("requirements").unwrap_or(&Value::Null);
    if requirements.is_null() {
        return RuntimeDefaults::default();
    }

    let allowed_approval_policies = requirements
        .get("allowedApprovalPolicies")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .filter_map(ApprovalPolicy::from_wire)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let approval_policy = ApprovalPolicy::PREFERRED
        .into_iter()
        .find(|candidate| allowed_approval_policies.contains(candidate))
        .unwrap_or_default();

    let allowed_sandbox_modes = requirements
        .get("allowedSandboxModes")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .filter_map(ThreadSandboxMode::from_wire)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let sandbox_mode = ThreadSandboxMode::PREFERRED
        .into_iter()
        .find(|candidate| allowed_sandbox_modes.contains(candidate))
        .unwrap_or_default();

    RuntimeDefaults {
        approval_policy,
        sandbox_mode,
    }
}

async fn resolve_model(client: &AppServerClient, requested: Option<&str>) -> AnyResult<String> {
    if let Some(model) = requested {
        return Ok(model.to_owned());
    }

    let response = client
        .request(
            ClientMethod::ModelList,
            json!({
                "limit": 20,
                "includeHidden": false,
            }),
        )
        .await?;

    let available = response
        .get("data")
        .and_then(Value::as_array)
        .context("model/list returned no `data` array")?
        .iter()
        .filter_map(|entry| {
            entry
                .get("model")
                .or_else(|| entry.get("id"))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .collect::<Vec<_>>();

    for candidate in PREFERRED_MODELS {
        if available.iter().any(|model| model == candidate) {
            return Ok((*candidate).to_owned());
        }
    }

    available
        .into_iter()
        .next()
        .context("model/list returned zero visible models")
}

async fn start_thread(
    client: &AppServerClient,
    model: &str,
    cwd: &Path,
    runtime: &RuntimeDefaults,
) -> AnyResult<String> {
    let response = client
        .request(
            ClientMethod::ThreadStart,
            json!({
                "model": model,
                "cwd": cwd.display().to_string(),
                "approvalPolicy": runtime.approval_policy.to_string(),
                "sandbox": runtime.sandbox_mode.to_string(),
                "serviceName": CODEX_SERVICE_NAME,
            }),
        )
        .await?;

    response
        .get("thread")
        .and_then(|thread| thread.get("id"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .context("thread/start returned no thread id")
}

async fn start_turn(
    client: &AppServerClient,
    thread_id: &str,
    model: &str,
    prompt: &str,
    config: &CodexPlannerConfig,
    runtime: &RuntimeDefaults,
) -> AnyResult<String> {
    let sandbox_policy = runtime
        .sandbox_mode
        .turn_policy_json(&config.cwd, config.network_access);

    let response = client
        .request(
            ClientMethod::TurnStart,
            json!({
                "threadId": thread_id,
                "input": [
                    {
                        "type": InputItemType::Text.to_string(),
                        "text": prompt,
                    }
                ],
                "cwd": config.cwd.display().to_string(),
                "approvalPolicy": runtime.approval_policy.to_string(),
                "sandboxPolicy": sandbox_policy,
                "model": model,
                "effort": config.effort.to_string(),
                "summary": SummaryMode::Concise.to_string(),
            }),
        )
        .await?;

    response
        .get("turn")
        .and_then(|turn| turn.get("id"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .context("turn/start returned no turn id")
}

async fn collect_turn_output(
    notifications: &mut broadcast::Receiver<Value>,
    turn_id: &str,
) -> AnyResult<String> {
    let mut output = String::new();

    loop {
        let message = match notifications.recv().await {
            Ok(message) => message,
            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                eprintln!("warning: skipped {skipped} notifications during turn");
                continue;
            }
            Err(broadcast::error::RecvError::Closed) => {
                bail!("notification stream closed while waiting for turn completion");
            }
        };

        let Some(method) = server_method(&message) else {
            continue;
        };
        let params = message.get("params").unwrap_or(&Value::Null);

        match method {
            ServerMethod::ItemAgentMessageDelta if params_matches_turn(params, turn_id) => {
                let delta = params
                    .get("delta")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                output.push_str(delta);
            }
            ServerMethod::ItemCompleted if params_matches_turn(params, turn_id) => {
                let item = params.get("item").unwrap_or(&Value::Null);
                let item_type = item
                    .get("type")
                    .and_then(Value::as_str)
                    .map(ThreadItemType::from_wire)
                    .unwrap_or_else(|| ThreadItemType::Unknown(String::from("unknown")));

                match item_type {
                    ThreadItemType::AgentMessage if output.is_empty() => {
                        let text = item.get("text").and_then(Value::as_str).unwrap_or_default();
                        output.push_str(text);
                    }
                    ThreadItemType::CommandExecution => {
                        let command = item
                            .get("command")
                            .and_then(Value::as_array)
                            .map(|parts| {
                                parts
                                    .iter()
                                    .filter_map(Value::as_str)
                                    .collect::<Vec<_>>()
                                    .join(" ")
                            })
                            .unwrap_or_else(|| String::from("<command>"));
                        let status = item
                            .get("status")
                            .and_then(Value::as_str)
                            .map(ItemStatus::from_wire)
                            .unwrap_or_else(|| ItemStatus::Unknown(String::from("unknown")));
                        bail!(
                            "codex planner attempted command execution while planning: [{status}] {command}"
                        );
                    }
                    ThreadItemType::FileChange => {
                        bail!("codex planner attempted file changes while planning");
                    }
                    _ => {}
                }
            }
            ServerMethod::TurnCompleted => {
                let completed_turn = params.get("turn").unwrap_or(&Value::Null);
                if completed_turn.get("id").and_then(Value::as_str) != Some(turn_id) {
                    continue;
                }

                let status = completed_turn
                    .get("status")
                    .and_then(Value::as_str)
                    .map(TurnStatus::from_wire)
                    .unwrap_or_else(|| TurnStatus::Unknown(String::from("unknown")));

                if status == TurnStatus::Completed {
                    return Ok(output);
                }

                let error_message = completed_turn
                    .get("error")
                    .and_then(|error| error.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or("unknown turn failure");
                bail!("turn finished with status `{status}`: {error_message}");
            }
            ServerMethod::Error if params_matches_turn(params, turn_id) => {
                let error_message = params
                    .get("error")
                    .and_then(|error| error.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or("unknown app-server error");
                bail!("codex planner turn failed: {error_message}");
            }
            _ => {}
        }
    }
}

fn server_method(message: &Value) -> Option<ServerMethod> {
    message
        .get("method")
        .and_then(Value::as_str)
        .map(ServerMethod::from_wire)
}

fn params_matches_turn(params: &Value, turn_id: &str) -> bool {
    params.get("turnId").and_then(Value::as_str) == Some(turn_id)
}

#[cfg(test)]
mod tests {
    use super::{parse_codex_output, strip_markdown_fences};

    #[test]
    fn strips_json_fences() {
        let fenced = "```json\n{\"message\":{\"content\":\"hello\"}}\n```";
        assert_eq!(
            strip_markdown_fences(fenced),
            "{\"message\":{\"content\":\"hello\"}}"
        );
    }

    #[test]
    fn wraps_plain_text_as_assistant_message() {
        let parsed = parse_codex_output("unsupported request").expect("plain text should parse");
        assert_eq!(
            parsed.message.content.as_deref(),
            Some("unsupported request")
        );
        assert!(parsed.message.tool_calls.is_none());
    }

    #[test]
    fn parses_tool_calls_without_role_field() {
        let parsed = parse_codex_output(
            r#"{"message":{"content":null,"tool_calls":[{"type":"function","function":{"name":"rust.count_lines_of_code","arguments":{"path":"."}}}]}}"#,
        )
        .expect("tool call JSON should parse");

        let tool_calls = parsed.message.tool_calls.expect("tool calls should exist");
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].function.name, "rust.count_lines_of_code");
        assert_eq!(tool_calls[0].function.arguments["path"], ".");
        assert_eq!(parsed.message.role, "assistant");
    }
}
