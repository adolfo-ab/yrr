use thiserror::Error;

#[derive(Debug, Error)]
pub enum YrrError {
    #[error("failed to parse YAML: {0}")]
    YamlParse(#[from] serde_yaml_ng::Error),

    #[error("failed to parse JSON: {0}")]
    JsonParse(#[from] serde_json::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("agent not found: {0}")]
    AgentNotFound(String),

    #[error("swarm not found: {0}")]
    SwarmNotFound(String),

    #[error("invalid reference: {0}")]
    InvalidRef(String),

    #[error("validation error: {0}")]
    Validation(String),

    #[error("runtime error: {0}")]
    Runtime(String),

    #[error("bus error: {0}")]
    Bus(String),

    #[error("query error: {0}")]
    Query(String),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, YrrError>;
