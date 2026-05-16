use retina_types::{AgentAuthority, ToolDescriptor};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Default)]
pub struct ToolPolicy {
    allowed: Option<BTreeSet<String>>,
    denied: BTreeSet<String>,
}

impl ToolPolicy {
    pub fn allow_all() -> Self {
        Self::default()
    }

    pub fn from_authority(authority: &AgentAuthority) -> Self {
        let mut allowed = BTreeSet::new();
        if authority.allow_text_responses {
            allowed.insert("respond".to_string());
        }
        if authority.allow_file_reads {
            allowed.insert("inspect_path".to_string());
            allowed.insert("read_file".to_string());
            allowed.insert("ingest_structured_data".to_string());
            allowed.insert("extract_document_text".to_string());
        }
        if authority.allow_file_search {
            allowed.insert("list_directory".to_string());
            allowed.insert("find_files".to_string());
            allowed.insert("search_text".to_string());
        }
        if authority.allow_mcp {
            allowed.insert("mcp:*".to_string());
            allowed.insert("list_mcp_resources".to_string());
            allowed.insert("read_mcp_resource".to_string());
            allowed.insert("mcp_call".to_string());
        }
        if authority.allow_file_writes {
            allowed.insert("edit_file".to_string());
            allowed.insert("write_file".to_string());
            allowed.insert("append_file".to_string());
            allowed.insert("edit_notebook".to_string());
        }
        if authority.allow_agent_delegation {
            allowed.insert("agent_spawn".to_string());
        }
        if authority.allow_notes {
            allowed.insert("record_note".to_string());
        }
        if authority.allow_command_execution {
            allowed.insert("run_command".to_string());
        }
        Self {
            allowed: Some(allowed),
            denied: BTreeSet::new(),
        }
    }

    pub fn from_task_metadata(metadata: &BTreeMap<String, String>) -> Self {
        Self {
            allowed: parse_tool_list(metadata.get("allowed_tools")),
            denied: parse_tool_list(metadata.get("denied_tools")).unwrap_or_default(),
        }
    }

    pub fn with_task_metadata(mut self, metadata: &BTreeMap<String, String>) -> Self {
        self.apply_overlay(Self::from_task_metadata(metadata));
        self
    }

    pub fn permits(&self, tool: &ToolDescriptor) -> bool {
        let allowed = self
            .allowed
            .as_ref()
            .map(|set| {
                set.contains(&tool.name)
                    || (tool.source == retina_types::ToolSourceKind::McpServer
                        && set.contains("mcp:*"))
            })
            .unwrap_or(true);
        allowed && !self.denied.contains(&tool.name)
    }

    pub fn filter(&self, tools: impl IntoIterator<Item = ToolDescriptor>) -> Vec<ToolDescriptor> {
        tools
            .into_iter()
            .filter(|tool| self.permits(tool))
            .collect()
    }

    fn apply_overlay(&mut self, overlay: Self) {
        if let Some(overlay_allowed) = overlay.allowed {
            self.allowed = match self.allowed.take() {
                Some(base) => Some(base.intersection(&overlay_allowed).cloned().collect()),
                None => Some(overlay_allowed),
            };
        }
        self.denied.extend(overlay.denied);
    }
}

fn parse_tool_list(value: Option<&String>) -> Option<BTreeSet<String>> {
    let value = value?;
    if value.split(',').map(str::trim).any(|item| item == "*") {
        return None;
    }
    let parsed = value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToString::to_string)
        .collect::<BTreeSet<_>>();
    Some(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use retina_types::{ToolApprovalPolicy, ToolConcurrencyClass, ToolDescriptor, ToolSourceKind};

    fn tool_descriptor(name: &str, source: ToolSourceKind) -> ToolDescriptor {
        ToolDescriptor {
            name: name.to_string(),
            description: "test tool".to_string(),
            source,
            concurrency: ToolConcurrencyClass::ReadOnly,
            approval: ToolApprovalPolicy::None,
            required_authority: vec![],
            streaming: false,
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        }
    }

    #[test]
    fn task_metadata_can_allow_and_deny_tool_names() {
        let mut metadata = BTreeMap::new();
        metadata.insert(
            "allowed_tools".to_string(),
            "read_file, write_file, run_command".to_string(),
        );
        metadata.insert("denied_tools".to_string(), "run_command".to_string());
        let policy = ToolPolicy::from_task_metadata(&metadata);

        let allowed = tool_descriptor("read_file", ToolSourceKind::BuiltinShell);
        let denied = ToolDescriptor {
            name: "run_command".to_string(),
            ..allowed.clone()
        };

        assert!(policy.permits(&allowed));
        assert!(!policy.permits(&denied));
    }

    #[test]
    fn authority_drives_base_tool_allowlist() {
        let policy = ToolPolicy::from_authority(&AgentAuthority {
            allow_command_execution: false,
            allow_file_reads: true,
            allow_file_writes: false,
            allow_file_search: true,
            allow_mcp: false,
            allow_agent_delegation: false,
            allow_notes: false,
            allow_text_responses: true,
            accessible_roots: vec![],
        });

        let allowed = tool_descriptor("read_file", ToolSourceKind::BuiltinShell);
        let denied = ToolDescriptor {
            name: "run_command".to_string(),
            ..allowed.clone()
        };

        assert!(policy.permits(&allowed));
        assert!(!policy.permits(&denied));

        let mcp_tool = ToolDescriptor {
            concurrency: ToolConcurrencyClass::Streaming,
            required_authority: vec!["mcp".to_string()],
            streaming: true,
            ..tool_descriptor("mcp__brave__brave_web_search", ToolSourceKind::McpServer)
        };
        assert!(!policy.permits(&mcp_tool));
    }

    #[test]
    fn task_metadata_cannot_reenable_tool_blocked_by_authority() {
        let mut metadata = BTreeMap::new();
        metadata.insert(
            "allowed_tools".to_string(),
            "read_file,run_command".to_string(),
        );
        let policy = ToolPolicy::from_authority(&AgentAuthority {
            allow_command_execution: false,
            allow_file_reads: true,
            allow_file_writes: false,
            allow_file_search: false,
            allow_mcp: false,
            allow_agent_delegation: false,
            allow_notes: false,
            allow_text_responses: true,
            accessible_roots: vec![],
        })
        .with_task_metadata(&metadata);

        let allowed = tool_descriptor("read_file", ToolSourceKind::BuiltinShell);
        let denied = ToolDescriptor {
            name: "run_command".to_string(),
            ..allowed.clone()
        };

        assert!(policy.permits(&allowed));
        assert!(!policy.permits(&denied));
    }

    #[test]
    fn authority_mcp_permission_allows_concrete_mcp_tools() {
        let policy = ToolPolicy::from_authority(&AgentAuthority {
            allow_command_execution: false,
            allow_file_reads: false,
            allow_file_writes: false,
            allow_file_search: false,
            allow_mcp: true,
            allow_agent_delegation: false,
            allow_notes: false,
            allow_text_responses: true,
            accessible_roots: vec![],
        });

        let mcp_tool = ToolDescriptor {
            concurrency: ToolConcurrencyClass::Streaming,
            required_authority: vec!["mcp".to_string()],
            streaming: true,
            ..tool_descriptor("mcp__brave__brave_web_search", ToolSourceKind::McpServer)
        };

        assert!(policy.permits(&mcp_tool));
    }

    #[test]
    fn wildcard_allowed_tools_means_inherit_full_surface() {
        let mut metadata = BTreeMap::new();
        metadata.insert("allowed_tools".to_string(), "*,read_file".to_string());
        metadata.insert("denied_tools".to_string(), "run_command".to_string());
        let policy = ToolPolicy::from_authority(&AgentAuthority {
            allow_command_execution: true,
            allow_file_reads: true,
            allow_file_writes: false,
            allow_file_search: true,
            allow_mcp: false,
            allow_agent_delegation: false,
            allow_notes: false,
            allow_text_responses: true,
            accessible_roots: vec![],
        })
        .with_task_metadata(&metadata);

        let read_file = tool_descriptor("read_file", ToolSourceKind::BuiltinShell);
        let list_directory = ToolDescriptor {
            name: "list_directory".to_string(),
            ..read_file.clone()
        };
        let run_command = ToolDescriptor {
            name: "run_command".to_string(),
            ..read_file.clone()
        };

        assert!(policy.permits(&read_file));
        assert!(policy.permits(&list_directory));
        assert!(!policy.permits(&run_command));
    }
}
