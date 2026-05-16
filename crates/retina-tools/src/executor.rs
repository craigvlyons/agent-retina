use crate::{ToolPolicy, ToolRegistry};
use retina_types::{Action, ToolDescriptor};

#[derive(Clone, Debug)]
pub struct ToolExecutor {
    registry: ToolRegistry,
    policy: ToolPolicy,
}

impl ToolExecutor {
    pub fn new(registry: ToolRegistry) -> Self {
        Self {
            registry,
            policy: ToolPolicy::allow_all(),
        }
    }

    pub fn with_policy(mut self, policy: ToolPolicy) -> Self {
        self.policy = policy;
        self
    }

    pub fn available_tools(&self) -> Vec<ToolDescriptor> {
        self.registry.filtered(&self.policy)
    }

    pub fn descriptor_for_action(&self, action: &Action) -> Option<&ToolDescriptor> {
        let name = tool_name_for_action(action);
        let tool = self.registry.get(&name)?;
        self.policy.permits(tool).then_some(tool)
    }
}

pub fn tool_name_for_action(action: &Action) -> String {
    match action {
        Action::RunCommand { .. } => "run_command".to_string(),
        Action::InspectPath { .. } => "inspect_path".to_string(),
        Action::InspectWorkingDirectory { .. } => "inspect_working_directory".to_string(),
        Action::ListDirectory { .. } => "list_directory".to_string(),
        Action::FindFiles { .. } => "find_files".to_string(),
        Action::SearchText { .. } => "search_text".to_string(),
        Action::ReadFile { .. } => "read_file".to_string(),
        Action::IngestStructuredData { .. } => "ingest_structured_data".to_string(),
        Action::ExtractDocumentText { .. } => "extract_document_text".to_string(),
        Action::ListMcpResources { .. } => "list_mcp_resources".to_string(),
        Action::ReadMcpResource { .. } => "read_mcp_resource".to_string(),
        Action::CallMcpTool {
            resolved_tool_name, ..
        } => resolved_tool_name
            .clone()
            .unwrap_or_else(|| "mcp_call".to_string()),
        Action::WriteFile { .. } => "write_file".to_string(),
        Action::EditFile { .. } => "edit_file".to_string(),
        Action::AppendFile { .. } => "append_file".to_string(),
        Action::EditNotebook { .. } => "edit_notebook".to_string(),
        Action::SpawnAgent { .. } => "agent_spawn".to_string(),
        Action::RecordNote { .. } => "record_note".to_string(),
        Action::Respond { .. } => "respond".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use retina_types::{ActionId, HashScope, ShellCapabilities};

    #[test]
    fn executor_maps_action_back_to_available_tool_descriptor() {
        let registry = ToolRegistry::for_shell_capabilities(
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
        let executor = ToolExecutor::new(registry);
        let action = Action::RunCommand {
            id: ActionId::new(),
            command: "pwd".to_string(),
            cwd: None,
            require_approval: false,
            expect_change: false,
            state_scope: HashScope::default(),
        };

        let descriptor = executor.descriptor_for_action(&action).unwrap();
        assert_eq!(descriptor.name, "run_command");
    }
}
