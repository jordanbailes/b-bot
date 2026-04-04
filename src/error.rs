use thiserror::Error;

pub type Result<T> = std::result::Result<T, AppError>;

#[derive(Debug, Error)]
pub enum AppError {
    #[error(transparent)]
    Other(#[from] anyhow::Error),

    #[error("failed to read input: {0}")]
    Io(#[from] std::io::Error),

    #[error("failed to call Ollama: {0}")]
    Http(#[from] reqwest::Error),

    #[error("failed to parse JSON: {0}")]
    Json(#[from] serde_json::Error),

    #[error("failed to parse {backend} response: {source}. response body: {body}")]
    PlannerParse {
        backend: &'static str,
        #[source]
        source: serde_json::Error,
        body: String,
    },

    #[error("docker error: {0}")]
    Docker(#[from] bollard::errors::Error),

    #[error("{backend} returned an invalid response")]
    InvalidPlannerResponse { backend: &'static str },

    #[error("unknown tool requested by the model: {0}")]
    UnknownTool(String),

    #[error("model returned invalid arguments for {tool}: {source}")]
    InvalidToolArguments {
        tool: String,
        #[source]
        source: serde_json::Error,
    },

    #[error("execution cancelled by user")]
    UserDeclined,

    #[error("{0}")]
    Message(String),
}
