use retina_types::{
    McpRegistrySnapshot, ShellCapabilities, ToolApprovalPolicy, ToolConcurrencyClass,
    ToolDescriptor, ToolSourceKind,
};

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
        ));
    }
    if capabilities.can_read_files {
        tools.push(builtin_tool(
            "inspect_path",
            "Inspect one path for existence, metadata, and optional content hash.",
            ToolConcurrencyClass::ReadOnly,
            vec!["file_read"],
        ));
    }
    if capabilities.can_search_files {
        tools.push(builtin_tool(
            "list_directory",
            "List files and directories in a target directory.",
            ToolConcurrencyClass::ReadOnly,
            vec!["file_search"],
        ));
        tools.push(builtin_tool(
            "find_files",
            "Find files by filename or path fragment.",
            ToolConcurrencyClass::ReadOnly,
            vec!["file_search"],
        ));
        tools.push(builtin_tool(
            "search_text",
            "Search text content across files in the current workspace.",
            ToolConcurrencyClass::ReadOnly,
            vec!["file_search"],
        ));
    }

    if capabilities.can_read_files {
        tools.push(builtin_tool(
            "read_file",
            "Read text-like files such as markdown, code, config, and plaintext with truncation protection.",
            ToolConcurrencyClass::ReadOnly,
            vec!["file_read"],
        ));
        tools.push(builtin_tool(
            "ingest_structured_data",
            "Inspect structured local data such as CSV or TSV files by headers and sample rows.",
            ToolConcurrencyClass::ReadOnly,
            vec!["file_read"],
        ));
    }
    if capabilities.can_extract_documents {
        tools.push(builtin_tool(
            "extract_document_text",
            "Extract readable text from documents such as PDFs when raw file reads would be binary or unhelpful.",
            ToolConcurrencyClass::ReadOnly,
            vec!["document_extract"],
        ));
    }
    if capabilities.can_write_files {
        tools.push(builtin_tool(
            "edit_file",
            "Prefer this for modifying existing text files. Edit by exact old-string replacement after reading the file first; ambiguous matches are rejected unless replace_all=true.",
            ToolConcurrencyClass::Mutation,
            vec!["file_write"],
        ));
        tools.push(builtin_tool(
            "write_file",
            "Use this for creating new text files or complete rewrites. If the file already exists, read it first.",
            ToolConcurrencyClass::Mutation,
            vec!["file_write"],
        ));
        tools.push(builtin_tool(
            "append_file",
            "Append content to a text file. Existing files should be read first.",
            ToolConcurrencyClass::Mutation,
            vec!["file_write"],
        ));
        tools.push(builtin_tool(
            "edit_notebook",
            "Modify a .ipynb notebook by replacing, inserting, or deleting a specific cell after reading the notebook first.",
            ToolConcurrencyClass::Mutation,
            vec!["file_write"],
        ));
    }
    if capabilities.can_write_notes {
        tools.push(builtin_tool(
            "record_note",
            "Store a compact note in local memory when it helps preserve useful context.",
            ToolConcurrencyClass::Mutation,
            vec!["note_write"],
        ));
    }
    if supports_local_agents {
        tools.push(builtin_tool(
            "agent_spawn",
            "Delegate a bounded local subtask to a child Retina worker and integrate its result.",
            ToolConcurrencyClass::LongRunning,
            vec!["agent_delegation"],
        ));
    }
    if capabilities.can_execute_commands {
        let mut command = builtin_tool(
            "run_command",
            "Run shell commands, pipelines, or local scripts when they best advance the task.",
            ToolConcurrencyClass::LongRunning,
            vec!["command_execution"],
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

    let tool_preview = connected_servers
        .iter()
        .flat_map(|server| {
            server.tools.iter().map(|tool| {
                format!(
                    "{}/{}{}",
                    server.name,
                    tool.name,
                    tool.description
                        .as_deref()
                        .map(|desc| format!(" - {}", trim_description(desc, 90)))
                        .unwrap_or_default()
                )
            })
        })
        .take(10)
        .collect::<Vec<_>>()
        .join("; ");

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
    });
    tools.push(ToolDescriptor {
        name: "mcp_call".to_string(),
        description: format!(
            "Call a configured MCP tool by server and tool name. Available tools: {}.",
            if tool_preview.is_empty() {
                "none discovered".to_string()
            } else {
                tool_preview
            }
        ),
        source: ToolSourceKind::McpServer,
        concurrency: ToolConcurrencyClass::Streaming,
        approval: ToolApprovalPolicy::None,
        required_authority: vec!["mcp".to_string()],
        streaming: true,
    });

    tools
}

fn builtin_tool(
    name: &str,
    description: &str,
    concurrency: ToolConcurrencyClass,
    required_authority: Vec<&str>,
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
    }
}

fn trim_description(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let trimmed = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{trimmed}...")
    } else {
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(names.contains(&"mcp_call"));
    }
}
