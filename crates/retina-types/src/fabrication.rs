use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum SourceLanguage {
    Rust,
    Other(String),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Dependency {
    pub name: String,
    pub version: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolSource {
    pub language: SourceLanguage,
    pub code: String,
    pub dependencies: Vec<Dependency>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompiledTool {
    pub binary: Vec<u8>,
    pub source_hash: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolTest {
    pub name: String,
    pub input: Value,
    pub expected: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TestReport {
    pub passed: bool,
    pub executed: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FabricatorCapabilities {
    pub allows_filesystem: bool,
    pub allows_network: bool,
    pub memory_limit_bytes: u64,
    pub timeout_ms: u64,
}
