use retina_traits::McpRuntime;
use retina_types::*;
use rmcp::{
    ServiceExt,
    model::{CallToolRequestParams, RawContent, ReadResourceRequestParams, ResourceContents},
    transport::TokioChildProcess,
};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct ConfiguredMcpRuntime {
    config_path: PathBuf,
}

impl ConfiguredMcpRuntime {
    pub fn new(config_path: PathBuf) -> Self {
        Self { config_path }
    }

    fn load_config(&self) -> Result<McpConfigFile> {
        let path = &self.config_path;
        if !path.exists() {
            return Ok(McpConfigFile::default());
        }
        let raw = std::fs::read_to_string(path)
            .map_err(|error| KernelError::Configuration(format!("read MCP config: {error}")))?;
        toml::from_str(&raw)
            .map_err(|error| KernelError::Configuration(format!("parse MCP config: {error}")))
    }

    fn enabled_servers(&self) -> Result<Vec<(String, StdioServerConfig)>> {
        Ok(self
            .load_config()?
            .servers
            .into_iter()
            .filter(|(_, cfg)| cfg.enabled.unwrap_or(true))
            .collect())
    }

    fn with_client<T>(
        &self,
        config: &StdioServerConfig,
        op: impl FnOnce(
            &tokio::runtime::Runtime,
            rmcp::service::RunningService<rmcp::RoleClient, ()>,
        ) -> Result<T>,
    ) -> Result<T> {
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|error| KernelError::Execution(format!("start tokio runtime: {error}")))?;
        let mut command = tokio::process::Command::new(&config.command);
        command.args(&config.args);
        if let Some(cwd) = &config.cwd {
            command.current_dir(cwd);
        }
        for (key, value) in &config.env {
            command.env(key, value);
        }
        let transport = TokioChildProcess::new(command)
            .map_err(|error| KernelError::Execution(format!("spawn MCP server: {error}")))?;
        let client = runtime
            .block_on(async { ().serve(transport).await })
            .map_err(|error| KernelError::Execution(format!("connect MCP server: {error}")))?;
        op(&runtime, client)
    }

    fn get_server_config(&self, server: &str) -> Result<StdioServerConfig> {
        self.load_config()?
            .servers
            .get(server)
            .cloned()
            .filter(|cfg| cfg.enabled.unwrap_or(true))
            .ok_or_else(|| {
                KernelError::Execution(format!("MCP server '{server}' is not configured"))
            })
    }
}

impl McpRuntime for ConfiguredMcpRuntime {
    fn snapshot(&self) -> Result<McpRegistrySnapshot> {
        let mut servers = Vec::new();
        for (name, config) in self.enabled_servers()? {
            let result = self.with_client(&config, |runtime, client| {
                let tools = runtime
                    .block_on(async { client.peer().list_all_tools().await })
                    .map_err(|error| {
                        KernelError::Execution(format!("list MCP tools for {name}: {error}"))
                    })?;
                let resources = runtime
                    .block_on(async { client.peer().list_all_resources().await })
                    .map_err(|error| {
                        KernelError::Execution(format!("list MCP resources for {name}: {error}"))
                    })?;
                Ok(McpServerSnapshot {
                    name: name.clone(),
                    tools: tools
                        .into_iter()
                        .map(|tool| summarize_tool(&name, tool))
                        .collect(),
                    resources: resources
                        .into_iter()
                        .map(|resource| summarize_resource(&name, resource))
                        .collect(),
                    error: None,
                })
            });

            match result {
                Ok(server) => servers.push(server),
                Err(error) => servers.push(McpServerSnapshot {
                    name,
                    tools: Vec::new(),
                    resources: Vec::new(),
                    error: Some(error.to_string()),
                }),
            }
        }
        Ok(McpRegistrySnapshot { servers })
    }

    fn list_resources(&self, server: Option<&str>) -> Result<Vec<McpResourceSummary>> {
        let snapshot = self.snapshot()?;
        Ok(snapshot
            .servers
            .into_iter()
            .filter(|entry| server.map(|name| name == entry.name).unwrap_or(true))
            .flat_map(|entry| entry.resources.into_iter())
            .collect())
    }

    fn read_resource(&self, server: &str, uri: &str) -> Result<McpResourceReadResult> {
        let config = self.get_server_config(server)?;
        self.with_client(&config, |runtime, client| {
            let result = runtime
                .block_on(async {
                    client
                        .peer()
                        .read_resource(ReadResourceRequestParams::new(uri.to_string()))
                        .await
                })
                .map_err(|error| {
                    KernelError::Execution(format!(
                        "read MCP resource '{uri}' from '{server}': {error}"
                    ))
                })?;
            Ok(McpResourceReadResult {
                server: server.to_string(),
                uri: uri.to_string(),
                contents: result
                    .contents
                    .into_iter()
                    .map(resource_content_item)
                    .collect(),
            })
        })
    }

    fn call_tool(
        &self,
        server: &str,
        tool: &str,
        input_json: &serde_json::Value,
    ) -> Result<McpToolCallResult> {
        let config = self.get_server_config(server)?;
        self.with_client(&config, |runtime, client| {
            let params =
                CallToolRequestParams::new(tool.to_string()).with_arguments(match input_json {
                    serde_json::Value::Object(map) => map.clone(),
                    _ => serde_json::Map::new(),
                });
            let result = runtime
                .block_on(async { client.peer().call_tool(params).await })
                .map_err(|error| {
                    KernelError::Execution(format!("call MCP tool '{tool}' on '{server}': {error}"))
                })?;
            Ok(McpToolCallResult {
                server: server.to_string(),
                tool: tool.to_string(),
                content_preview: summarize_tool_content(&result.content),
                structured_content: result.structured_content,
                is_error: result.is_error.unwrap_or(false),
            })
        })
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
struct McpConfigFile {
    #[serde(default)]
    servers: BTreeMap<String, StdioServerConfig>,
}

#[derive(Clone, Debug, Deserialize)]
struct StdioServerConfig {
    command: String,
    #[serde(default)]
    args: Vec<String>,
    cwd: Option<PathBuf>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    enabled: Option<bool>,
}

fn summarize_tool(server: &str, tool: rmcp::model::Tool) -> McpToolSummary {
    let annotations = tool.annotations.as_ref();
    McpToolSummary {
        server: server.to_string(),
        name: tool.name.to_string(),
        description: tool.description.map(|value| value.to_string()),
        read_only: annotations
            .and_then(|value| value.read_only_hint)
            .unwrap_or(false),
        destructive: annotations
            .and_then(|value| value.destructive_hint)
            .unwrap_or(true),
        open_world: annotations
            .and_then(|value| value.open_world_hint)
            .unwrap_or(true),
    }
}

fn summarize_resource(server: &str, resource: rmcp::model::Resource) -> McpResourceSummary {
    McpResourceSummary {
        server: server.to_string(),
        uri: resource.uri.clone(),
        name: resource.name.clone(),
        description: resource.description.clone(),
        mime_type: resource.mime_type.clone(),
    }
}

fn resource_content_item(content: ResourceContents) -> McpResourceContentItem {
    match content {
        ResourceContents::TextResourceContents {
            uri,
            mime_type,
            text,
            ..
        } => McpResourceContentItem {
            uri,
            mime_type,
            text: Some(text),
            blob_base64: None,
        },
        ResourceContents::BlobResourceContents {
            uri,
            mime_type,
            blob,
            ..
        } => McpResourceContentItem {
            uri,
            mime_type,
            text: None,
            blob_base64: Some(blob),
        },
    }
}

fn summarize_tool_content(content: &[rmcp::model::Content]) -> String {
    let mut parts = Vec::new();
    for item in content {
        match &**item {
            RawContent::Text(text) => parts.push(text.text.clone()),
            RawContent::Resource(resource) => match &resource.resource {
                ResourceContents::TextResourceContents { text, .. } => parts.push(text.clone()),
                ResourceContents::BlobResourceContents { uri, .. } => {
                    parts.push(format!("binary resource {uri}"))
                }
            },
            RawContent::Image(image) => parts.push(format!("image {}", image.mime_type)),
            RawContent::Audio(audio) => parts.push(format!("audio {}", audio.mime_type)),
            RawContent::ResourceLink(link) => parts.push(format!("resource link {}", link.uri)),
        }
    }
    if parts.is_empty() {
        "(no content returned)".to_string()
    } else {
        parts.join("\n")
    }
}

pub fn default_config_path(retina_home: &Path) -> PathBuf {
    retina_home.join("mcp").join("servers.toml")
}
