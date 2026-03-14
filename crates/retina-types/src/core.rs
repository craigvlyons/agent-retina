use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt::{Display, Formatter};
use thiserror::Error;
use uuid::Uuid;

pub type Result<T> = std::result::Result<T, KernelError>;
pub type EventPayload = Value;

macro_rules! id_type {
    ($name:ident) => {
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        pub struct $name(pub String);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4().to_string())
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl Display for $name {
            fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

id_type!(AgentId);
id_type!(TaskId);
id_type!(SessionId);
id_type!(IntentId);
id_type!(ActionId);
id_type!(EventId);
id_type!(ExperienceId);
id_type!(KnowledgeId);
id_type!(RuleId);
id_type!(ToolId);

#[derive(Debug, Error, Clone, Serialize, Deserialize)]
pub enum KernelError {
    #[error("configuration error: {0}")]
    Configuration(String),
    #[error("validation error: {0}")]
    Validation(String),
    #[error("shell execution error: {0}")]
    Execution(String),
    #[error("reasoning error: {0}")]
    Reasoning(String),
    #[error("storage error: {0}")]
    Storage(String),
    #[error("approval denied: {0}")]
    ApprovalDenied(String),
    #[error("unsupported operation: {0}")]
    Unsupported(String),
}

impl From<std::io::Error> for KernelError {
    fn from(value: std::io::Error) -> Self {
        Self::Execution(value.to_string())
    }
}
