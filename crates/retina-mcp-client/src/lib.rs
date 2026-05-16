use retina_traits::McpRuntime;
use retina_types::*;
use rmcp::{
    ServiceExt,
    model::{CallToolRequestParams, RawContent, ReadResourceRequestParams, ResourceContents},
    transport::TokioChildProcess,
};
use serde::Deserialize;
use serde_json::Value;
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
        let client = runtime.block_on(async {
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
            ().serve(transport)
                .await
                .map_err(|error| KernelError::Execution(format!("connect MCP server: {error}")))
        })?;
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
                let resources =
                    match runtime.block_on(async { client.peer().list_all_resources().await }) {
                        Ok(resources) => resources,
                        Err(error) if is_method_not_found_error(&error.to_string()) => Vec::new(),
                        Err(error) => {
                            return Err(KernelError::Execution(format!(
                                "list MCP resources for {name}: {error}"
                            )));
                        }
                    };
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
            let content_preview = summarize_tool_content(&result.content);
            let search_hits =
                extract_search_hits(result.structured_content.as_ref(), &content_preview);
            let search_outcome_kind = classify_search_outcome(
                tool,
                result.is_error.unwrap_or(false),
                &content_preview,
                result.structured_content.as_ref(),
                &search_hits,
            );
            let evidence_identities = extract_evidence_identities(
                &content_preview,
                result.structured_content.as_ref(),
                &search_hits,
            );
            let primary_locator = extract_primary_locator(
                &content_preview,
                result.structured_content.as_ref(),
                &search_hits,
            );
            Ok(McpToolCallResult {
                server: server.to_string(),
                tool: tool.to_string(),
                content_preview,
                structured_content: result.structured_content,
                is_error: result.is_error.unwrap_or(false),
                search_outcome_kind,
                evidence_identities,
                search_hits: search_hits.clone(),
                primary_locator,
                evidence_summary: summarize_search_evidence(&search_hits),
            })
        })
    }
}

fn classify_search_outcome(
    tool_name: &str,
    is_error: bool,
    content_preview: &str,
    structured_content: Option<&Value>,
    search_hits: &[McpSearchHitSummary],
) -> Option<McpSearchOutcomeKind> {
    if !tool_name.contains("search") {
        return Some(McpSearchOutcomeKind::NonSearchResult);
    }
    if is_error {
        if content_preview.contains("Input validation error") {
            return Some(McpSearchOutcomeKind::ValidationError);
        }
        return Some(McpSearchOutcomeKind::ToolError);
    }
    if tool_name.contains("local_search") && content_preview.contains("No location data") {
        return Some(McpSearchOutcomeKind::NoLocalSignal);
    }
    if tool_name.contains("news_search") {
        return Some(McpSearchOutcomeKind::NewsRoundup);
    }
    if looks_like_generic_portal(content_preview, structured_content, search_hits) {
        return Some(McpSearchOutcomeKind::GenericPortal);
    }
    if looks_like_single_event(search_hits) {
        return Some(McpSearchOutcomeKind::SingleEvent);
    }
    Some(McpSearchOutcomeKind::SpecificListing)
}

fn extract_evidence_identities(
    content_preview: &str,
    structured_content: Option<&Value>,
    search_hits: &[McpSearchHitSummary],
) -> Vec<String> {
    let mut tokens = Vec::new();
    for hit in search_hits.iter().take(4) {
        push_unique(&mut tokens, hit.url.clone());
        if let Some(title) = &hit.title {
            push_unique(&mut tokens, title.clone());
        }
    }
    collect_identity_tokens_from_json(structured_content, &mut tokens);
    if tokens.is_empty() {
        if let Some(url) = extract_url_like_token(content_preview) {
            tokens.push(url);
        }
    }
    tokens.truncate(6);
    tokens
}

fn extract_primary_locator(
    content_preview: &str,
    structured_content: Option<&Value>,
    search_hits: &[McpSearchHitSummary],
) -> Option<String> {
    if let Some(url) = search_hits.first().map(|hit| hit.url.clone()) {
        return Some(url);
    }
    extract_evidence_identities(content_preview, structured_content, search_hits)
        .into_iter()
        .find(|value| value.starts_with("http://") || value.starts_with("https://"))
}

fn collect_identity_tokens_from_json(value: Option<&Value>, tokens: &mut Vec<String>) {
    let Some(value) = value else {
        return;
    };
    match value {
        Value::Object(map) => {
            if let Some(url) = map.get("url").and_then(Value::as_str) {
                push_unique(tokens, url.to_string());
            }
            if let Some(title) = map.get("title").and_then(Value::as_str) {
                push_unique(tokens, title.to_string());
            }
            for nested in map.values() {
                collect_identity_tokens_from_json(Some(nested), tokens);
                if tokens.len() >= 6 {
                    break;
                }
            }
        }
        Value::Array(items) => {
            for item in items.iter().take(6) {
                collect_identity_tokens_from_json(Some(item), tokens);
                if tokens.len() >= 6 {
                    break;
                }
            }
        }
        _ => {}
    }
}

fn extract_url_like_token(text: &str) -> Option<String> {
    text.split('"')
        .find(|part| part.starts_with("http://") || part.starts_with("https://"))
        .map(ToString::to_string)
}

fn looks_like_generic_portal(
    content_preview: &str,
    structured_content: Option<&Value>,
    search_hits: &[McpSearchHitSummary],
) -> bool {
    let identities = extract_evidence_identities(content_preview, structured_content, search_hits)
        .join(" ")
        .to_ascii_lowercase();
    let hit_titles = search_hits
        .iter()
        .filter_map(|hit| hit.title.as_deref())
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    let combined = format!(
        "{} {} {}",
        identities,
        hit_titles,
        content_preview.to_ascii_lowercase()
    );
    [
        "events",
        "calendar",
        "things to do",
        "discover",
        "weekend",
        "roundup",
        "guide",
        "what to do",
    ]
    .iter()
    .any(|needle| combined.contains(needle))
}

fn looks_like_single_event(search_hits: &[McpSearchHitSummary]) -> bool {
    let Some(hit) = search_hits.first() else {
        return false;
    };
    let combined = format!(
        "{} {} {}",
        hit.url,
        hit.title.as_deref().unwrap_or_default(),
        hit.snippet.as_deref().unwrap_or_default()
    )
    .to_ascii_lowercase();
    [
        "tickets",
        "doors open",
        "starts at",
        "pm",
        "am",
        "on sale",
        "lineup",
        "venue",
    ]
    .iter()
    .any(|needle| combined.contains(needle))
}

fn summarize_search_evidence(search_hits: &[McpSearchHitSummary]) -> Option<String> {
    let highlights = search_hits
        .iter()
        .take(3)
        .map(|hit| {
            let mut line = hit
                .title
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| hit.url.clone());
            if let Some(snippet) = &hit.snippet {
                line.push_str(": ");
                line.push_str(snippet.trim());
            }
            line
        })
        .collect::<Vec<_>>();
    if highlights.is_empty() {
        None
    } else {
        Some(highlights.join(" | "))
    }
}

fn extract_search_hits(
    structured_content: Option<&Value>,
    content_preview: &str,
) -> Vec<McpSearchHitSummary> {
    let mut hits = Vec::new();
    collect_search_hits_from_json(structured_content, &mut hits);
    if hits.is_empty() {
        if let Some(url) = extract_url_like_token(content_preview) {
            hits.push(McpSearchHitSummary {
                url,
                title: None,
                snippet: Some(content_preview.to_string()),
            });
        }
    }
    hits.truncate(6);
    hits
}

fn collect_search_hits_from_json(value: Option<&Value>, hits: &mut Vec<McpSearchHitSummary>) {
    let Some(value) = value else {
        return;
    };
    match value {
        Value::Object(map) => {
            if let Some(url) = map.get("url").and_then(Value::as_str) {
                let title = first_string(map, &["title", "name", "headline"]);
                let snippet = first_string(map, &["description", "snippet", "summary"]);
                push_unique_hit(
                    hits,
                    McpSearchHitSummary {
                        url: url.to_string(),
                        title,
                        snippet,
                    },
                );
            }
            for nested in map.values() {
                collect_search_hits_from_json(Some(nested), hits);
                if hits.len() >= 6 {
                    break;
                }
            }
        }
        Value::Array(items) => {
            for item in items.iter().take(6) {
                collect_search_hits_from_json(Some(item), hits);
                if hits.len() >= 6 {
                    break;
                }
            }
        }
        _ => {}
    }
}

fn first_string(map: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| map.get(*key).and_then(Value::as_str))
        .map(str::to_string)
}

fn push_unique_hit(hits: &mut Vec<McpSearchHitSummary>, hit: McpSearchHitSummary) {
    if !hits.iter().any(|existing| existing.url == hit.url) {
        hits.push(hit);
    }
}

fn push_unique(tokens: &mut Vec<String>, value: String) {
    if !tokens.contains(&value) {
        tokens.push(value);
    }
}

fn is_method_not_found_error(message: &str) -> bool {
    message.contains("-32601") || message.to_ascii_lowercase().contains("method not found")
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
        input_schema: serde_json::Value::Object((*tool.input_schema).clone()),
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

pub fn default_config_template() -> String {
    r#"# Retina MCP server configuration
#
# Each server runs as a stdio MCP process.
# Enable the ones you actually want the agent to use.
#
# Example Brave Search server:
# [servers.brave]
# command = "npx"
# args = ["-y", "@modelcontextprotocol/server-brave-search"]
# enabled = false
# [servers.brave.env]
# BRAVE_API_KEY = "${BRAVE_API_KEY}"
"#
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_search_hits_and_evidence_from_structured_content() {
        let structured = serde_json::json!({
            "results": [
                {
                    "url": "https://visitdenver.com/blog/post/denver-events-this-weekend/",
                    "title": "Denver Events & Things to Do This Weekend",
                    "description": "Official weekend events guide"
                },
                {
                    "url": "https://www.westword.com/arts-culture/free-things-to-do-in-denver-20764019/",
                    "title": "Free Things to Do in Denver This Weekend",
                    "snippet": "Westword roundup of weekend activities"
                }
            ]
        });

        let hits = extract_search_hits(Some(&structured), "fallback");
        assert_eq!(hits.len(), 2);
        assert_eq!(
            hits[0].url,
            "https://visitdenver.com/blog/post/denver-events-this-weekend/"
        );
        assert_eq!(
            hits[0].title.as_deref(),
            Some("Denver Events & Things to Do This Weekend")
        );

        let identities = extract_evidence_identities("fallback", Some(&structured), &hits);
        assert!(
            identities
                .iter()
                .any(|value| value.contains("visitdenver.com"))
        );

        let summary = summarize_search_evidence(&hits).unwrap_or_default();
        assert!(summary.contains("Denver Events & Things to Do This Weekend"));
    }

    #[test]
    fn classifies_news_and_portal_search_outcomes() {
        let portal_hits = vec![McpSearchHitSummary {
            url: "https://visitdenver.com/blog/post/denver-events-this-weekend/".to_string(),
            title: Some("Denver Events & Things to Do This Weekend".to_string()),
            snippet: Some("Official weekend events guide".to_string()),
        }];
        assert_eq!(
            classify_search_outcome("brave_web_search", false, "preview", None, &portal_hits),
            Some(McpSearchOutcomeKind::GenericPortal)
        );

        let news_hits = vec![McpSearchHitSummary {
            url: "https://www.koaa.com/around-town/fun-filled-events-across-colorado-this-weekend"
                .to_string(),
            title: Some("Fun-filled events across Colorado this weekend".to_string()),
            snippet: Some("Weekend news roundup".to_string()),
        }];
        assert_eq!(
            classify_search_outcome("brave_news_search", false, "preview", None, &news_hits),
            Some(McpSearchOutcomeKind::NewsRoundup)
        );
    }
}
