use retina_types::{
    McpRegistrySnapshot, ShellCapabilities, ToolApprovalPolicy, ToolConcurrencyClass,
    ToolDescriptor, ToolSourceKind, build_mcp_tool_name,
};
use serde_json::json;

pub fn shell_builtin_tools(
    capabilities: ShellCapabilities,
    supports_local_agents: bool,
) -> Vec<ToolDescriptor> {
    let mut tools = Vec::new();

    if capabilities.can_respond_text {
        tools.push(builtin_tool(
            "respond",
            "Answer operator questions directly when no shell action is needed.",
            ToolConcurrencyClass::ReadOnly,
            vec!["text_response"],
            json!({
                "type": "object",
                "properties": {
                    "message": { "type": "string" }
                },
                "required": ["message"]
            }),
        ));
    }
    if capabilities.can_read_files {
        tools.push(builtin_tool(
            "inspect_path",
            "Inspect one path for existence, metadata, and optional content hash.",
            ToolConcurrencyClass::ReadOnly,
            vec!["file_read"],
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "include_content": { "type": "boolean" }
                },
                "required": ["path"]
            }),
        ));
    }
    if capabilities.can_search_files {
        tools.push(builtin_tool(
            "list_directory",
            "List files and directories in a target directory.",
            ToolConcurrencyClass::ReadOnly,
            vec!["file_search"],
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "recursive": { "type": "boolean" },
                    "max_entries": { "type": "integer" }
                },
                "required": ["path"]
            }),
        ));
        tools.push(builtin_tool(
            "find_files",
            "Find files by filename or path fragment.",
            ToolConcurrencyClass::ReadOnly,
            vec!["file_search"],
            json!({
                "type": "object",
                "properties": {
                    "root": { "type": "string" },
                    "pattern": { "type": "string" },
                    "recursive": { "type": "boolean" },
                    "max_results": { "type": "integer" }
                },
                "required": ["root", "pattern"]
            }),
        ));
        tools.push(builtin_tool(
            "search_text",
            "Search text content across files in the current workspace.",
            ToolConcurrencyClass::ReadOnly,
            vec!["file_search"],
            json!({
                "type": "object",
                "properties": {
                    "root": { "type": "string" },
                    "query": { "type": "string" },
                    "max_results": { "type": "integer" }
                },
                "required": ["root", "query"]
            }),
        ));
    }

    if capabilities.can_read_files {
        tools.push(builtin_tool(
            "read_file",
            "Read text-like files such as markdown, code, config, and plaintext with truncation protection.",
            ToolConcurrencyClass::ReadOnly,
            vec!["file_read"],
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "max_bytes": { "type": "integer" }
                },
                "required": ["path"]
            }),
        ));
        tools.push(builtin_tool(
            "ingest_structured_data",
            "Inspect structured local data such as CSV or TSV files by headers and sample rows.",
            ToolConcurrencyClass::ReadOnly,
            vec!["file_read"],
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "max_rows": { "type": "integer" }
                },
                "required": ["path"]
            }),
        ));
    }
    if capabilities.can_extract_documents {
        tools.push(builtin_tool(
            "extract_document_text",
            "Extract readable text from documents such as PDFs when raw file reads would be binary or unhelpful.",
            ToolConcurrencyClass::ReadOnly,
            vec!["document_extract"],
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "max_chars": { "type": "integer" },
                    "page_start": { "type": "integer" },
                    "page_end": { "type": "integer" }
                },
                "required": ["path"]
            }),
        ));
    }
    if capabilities.can_write_files {
        tools.push(builtin_tool(
            "edit_file",
            "Prefer this for modifying existing text files. Edit by exact old-string replacement after reading the file first; ambiguous matches are rejected unless replace_all=true.",
            ToolConcurrencyClass::Mutation,
            vec!["file_write"],
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "old_string": { "type": "string" },
                    "new_string": { "type": "string" },
                    "replace_all": { "type": "boolean" }
                },
                "required": ["path", "old_string", "new_string"]
            }),
        ));
        tools.push(builtin_tool(
            "write_file",
            "Use this for creating new text files or complete rewrites. If the file already exists, read it first.",
            ToolConcurrencyClass::Mutation,
            vec!["file_write"],
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "content": { "type": "string" },
                    "overwrite": { "type": "boolean" }
                },
                "required": ["path", "content"]
            }),
        ));
        tools.push(builtin_tool(
            "append_file",
            "Append content to a text file. Existing files should be read first.",
            ToolConcurrencyClass::Mutation,
            vec!["file_write"],
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "content": { "type": "string" }
                },
                "required": ["path", "content"]
            }),
        ));
        tools.push(builtin_tool(
            "edit_notebook",
            "Modify a .ipynb notebook by replacing, inserting, or deleting a specific cell after reading the notebook first.",
            ToolConcurrencyClass::Mutation,
            vec!["file_write"],
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "cell_id": { "type": "string" },
                    "new_source": { "type": "string" },
                    "cell_type": { "type": "string" },
                    "edit_mode": { "type": "string" }
                },
                "required": ["path", "new_source"]
            }),
        ));
    }
    if capabilities.can_write_notes {
        tools.push(builtin_tool(
            "record_note",
            "Store a compact note in local memory when it helps preserve useful context.",
            ToolConcurrencyClass::Mutation,
            vec!["note_write"],
            json!({
                "type": "object",
                "properties": {
                    "note": { "type": "string" }
                },
                "required": ["note"]
            }),
        ));
    }
    if supports_local_agents {
        tools.push(builtin_tool(
            "agent_spawn",
            "Delegate a bounded local subtask to a child Retina worker and integrate its result.",
            ToolConcurrencyClass::LongRunning,
            vec!["agent_delegation"],
            json!({
                "type": "object",
                "properties": {
                    "prompt": { "type": "string" },
                    "allowed_tools": {
                        "type": "array",
                        "items": { "type": "string" }
                    },
                    "denied_tools": {
                        "type": "array",
                        "items": { "type": "string" }
                    }
                },
                "required": ["prompt"]
            }),
        ));
    }
    if capabilities.can_execute_commands {
        let mut command = builtin_tool(
            "run_command",
            "Run shell commands, pipelines, or local scripts when they best advance the task.",
            ToolConcurrencyClass::LongRunning,
            vec!["command_execution"],
            json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string" },
                    "path": { "type": "string" },
                    "require_approval": { "type": "boolean" },
                    "expect_change": { "type": "boolean" }
                },
                "required": ["command"]
            }),
        );
        command.approval = ToolApprovalPolicy::ExplicitOperatorApproval;
        command.streaming = true;
        tools.push(command);
    }

    tools
}

pub fn mcp_client_tools(snapshot: &McpRegistrySnapshot) -> Vec<ToolDescriptor> {
    let mut tools = Vec::new();
    let connected_servers = snapshot
        .servers
        .iter()
        .filter(|server| server.error.is_none())
        .collect::<Vec<_>>();

    if connected_servers.is_empty() {
        return tools;
    }

    let resource_preview = connected_servers
        .iter()
        .flat_map(|server| {
            server
                .resources
                .iter()
                .map(|resource| format!("{}/{} ({})", server.name, resource.name, resource.uri))
        })
        .take(8)
        .collect::<Vec<_>>()
        .join("; ");
    let has_resources = connected_servers
        .iter()
        .any(|server| !server.resources.is_empty());

    for server in &connected_servers {
        for tool in &server.tools {
            tools.push(ToolDescriptor {
                name: build_mcp_tool_name(&server.name, &tool.name),
                description: format!(
                    "{}{}{}",
                    search_tool_prefix(&tool.name),
                    tool.description
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .unwrap_or("Call this MCP tool."),
                    render_input_schema_hint(&tool.input_schema)
                ),
                source: ToolSourceKind::McpServer,
                concurrency: ToolConcurrencyClass::Streaming,
                approval: ToolApprovalPolicy::None,
                required_authority: vec!["mcp".to_string()],
                streaming: true,
                input_schema: serde_json::Value::Object(
                    tool.input_schema.as_object().cloned().unwrap_or_default(),
                ),
            });
        }
    }

    if has_resources {
        tools.push(ToolDescriptor {
            name: "list_mcp_resources".to_string(),
            description: format!(
                "List readable resources exposed by configured MCP servers. Connected servers: {}.",
                connected_servers
                    .iter()
                    .map(|server| server.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            source: ToolSourceKind::McpServer,
            concurrency: ToolConcurrencyClass::ReadOnly,
            approval: ToolApprovalPolicy::None,
            required_authority: vec!["mcp".to_string()],
            streaming: false,
            input_schema: json!({
                "type": "object",
                "properties": {
                    "server": { "type": "string" }
                },
                "required": []
            }),
        });
        tools.push(ToolDescriptor {
            name: "read_mcp_resource".to_string(),
            description: format!(
                "Read a specific resource from a configured MCP server. Recent resources: {}.",
                if resource_preview.is_empty() {
                    "none discovered".to_string()
                } else {
                    resource_preview
                }
            ),
            source: ToolSourceKind::McpServer,
            concurrency: ToolConcurrencyClass::ReadOnly,
            approval: ToolApprovalPolicy::None,
            required_authority: vec!["mcp".to_string()],
            streaming: false,
            input_schema: json!({
                "type": "object",
                "properties": {
                    "server": { "type": "string" },
                    "uri": { "type": "string" }
                },
                "required": ["server", "uri"]
            }),
        });
    }
    tools
}

fn search_tool_prefix(tool_name: &str) -> &'static str {
    if tool_name.contains("local_search") {
        "Use this for place-aware local venue, business, or nearby lookup tasks. "
    } else if tool_name.contains("news_search") {
        "Use this for recent news, current-events coverage, and roundup-style reporting. "
    } else if tool_name.contains("web_search") || tool_name.ends_with("search") {
        "Use this for broad web discovery, official pages, and general internet research. "
    } else {
        ""
    }
}

fn builtin_tool(
    name: &str,
    description: &str,
    concurrency: ToolConcurrencyClass,
    required_authority: Vec<&str>,
    input_schema: serde_json::Value,
) -> ToolDescriptor {
    ToolDescriptor {
        name: name.to_string(),
        description: description.to_string(),
        source: ToolSourceKind::BuiltinShell,
        concurrency,
        approval: ToolApprovalPolicy::None,
        required_authority: required_authority
            .into_iter()
            .map(ToString::to_string)
            .collect(),
        streaming: false,
        input_schema,
    }
}

fn render_input_schema_hint(schema: &serde_json::Value) -> String {
    let Some(properties) = schema
        .get("properties")
        .and_then(serde_json::Value::as_object)
    else {
        return String::new();
    };
    if properties.is_empty() {
        return String::new();
    }

    let required = schema
        .get("required")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .collect::<std::collections::BTreeSet<_>>()
        })
        .unwrap_or_default();

    let fields = properties
        .iter()
        .take(6)
        .map(|(name, value)| {
            let field_type = value
                .get("type")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("value");
            if required.contains(name.as_str()) {
                format!("{name} ({field_type}, required)")
            } else {
                format!("{name} ({field_type})")
            }
        })
        .collect::<Vec<_>>();
    if fields.is_empty() {
        String::new()
    } else {
        format!(" Arguments: {}.", fields.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn shell_builtins_follow_capabilities_and_include_mutation_tools() {
        let tools = shell_builtin_tools(
            ShellCapabilities {
                can_execute_commands: true,
                can_read_files: true,
                can_write_files: true,
                can_search_files: true,
                can_extract_documents: true,
                can_write_notes: true,
                can_respond_text: true,
            },
            true,
        );

        let names = tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();
        assert!(names.contains(&"write_file"));
        assert!(names.contains(&"append_file"));
        assert!(names.contains(&"record_note"));
        assert!(names.contains(&"agent_spawn"));
        assert!(names.contains(&"run_command"));
    }

    #[test]
    fn mcp_client_tools_surface_generic_actionable_tools() {
        let tools = mcp_client_tools(&McpRegistrySnapshot {
            servers: vec![retina_types::McpServerSnapshot {
                name: "docs".to_string(),
                tools: vec![retina_types::McpToolSummary {
                    server: "docs".to_string(),
                    name: "search".to_string(),
                    description: Some("Search docs".to_string()),
                    read_only: true,
                    destructive: false,
                    open_world: false,
                    input_schema: json!({
                        "type": "object",
                        "properties": {
                            "query": { "type": "string" }
                        },
                        "required": ["query"]
                    }),
                }],
                resources: vec![retina_types::McpResourceSummary {
                    server: "docs".to_string(),
                    uri: "docs://guide".to_string(),
                    name: "guide".to_string(),
                    description: None,
                    mime_type: Some("text/plain".to_string()),
                }],
                error: None,
            }],
        });
        let names = tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();
        assert!(names.contains(&"list_mcp_resources"));
        assert!(names.contains(&"read_mcp_resource"));
        assert!(names.contains(&"mcp__docs__search"));
        assert!(!names.contains(&"mcp_call"));
    }

    #[test]
    fn mcp_client_tools_hide_resource_actions_when_no_resources_exist() {
        let tools = mcp_client_tools(&McpRegistrySnapshot {
            servers: vec![retina_types::McpServerSnapshot {
                name: "brave".to_string(),
                tools: vec![retina_types::McpToolSummary {
                    server: "brave".to_string(),
                    name: "brave_web_search".to_string(),
                    description: Some("Search the web".to_string()),
                    read_only: true,
                    destructive: false,
                    open_world: true,
                    input_schema: json!({
                        "type": "object",
                        "properties": {
                            "query": { "type": "string" }
                        },
                        "required": ["query"]
                    }),
                }],
                resources: Vec::new(),
                error: None,
            }],
        });
        let names = tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();
        assert!(!names.contains(&"list_mcp_resources"));
        assert!(!names.contains(&"read_mcp_resource"));
        assert!(names.contains(&"mcp__brave__brave_web_search"));
        assert!(!names.contains(&"mcp_call"));
    }

    #[test]
    fn mcp_search_tools_surface_family_specific_descriptions() {
        let tools = mcp_client_tools(&McpRegistrySnapshot {
            servers: vec![retina_types::McpServerSnapshot {
                name: "brave".to_string(),
                tools: vec![
                    retina_types::McpToolSummary {
                        server: "brave".to_string(),
                        name: "brave_web_search".to_string(),
                        description: Some("Search the web".to_string()),
                        read_only: true,
                        destructive: false,
                        open_world: true,
                        input_schema: json!({
                            "type": "object",
                            "properties": { "query": { "type": "string" } },
                            "required": ["query"]
                        }),
                    },
                    retina_types::McpToolSummary {
                        server: "brave".to_string(),
                        name: "brave_news_search".to_string(),
                        description: Some("Search recent news".to_string()),
                        read_only: true,
                        destructive: false,
                        open_world: true,
                        input_schema: json!({
                            "type": "object",
                            "properties": { "query": { "type": "string" } },
                            "required": ["query"]
                        }),
                    },
                ],
                resources: Vec::new(),
                error: None,
            }],
        });

        let web = tools
            .iter()
            .find(|tool| tool.name == "mcp__brave__brave_web_search")
            .expect("expected web search tool");
        let news = tools
            .iter()
            .find(|tool| tool.name == "mcp__brave__brave_news_search")
            .expect("expected news search tool");

        assert!(web.description.contains("broad web discovery"));
        assert!(news.description.contains("recent news"));
    }
}
