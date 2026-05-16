use crate::{ToolPolicy, mcp_client_tools, shell_builtin_tools};
use retina_types::{McpRegistrySnapshot, ShellCapabilities, ToolDescriptor};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default)]
pub struct ToolRegistry {
    tools: BTreeMap<String, ToolDescriptor>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn for_shell_capabilities(
        capabilities: ShellCapabilities,
        supports_local_agents: bool,
    ) -> Self {
        let mut registry = Self::new();
        registry.extend(shell_builtin_tools(capabilities, supports_local_agents));
        registry
    }

    pub fn with_mcp_snapshot(mut self, snapshot: &McpRegistrySnapshot) -> Self {
        self.extend(mcp_client_tools(snapshot));
        self
    }

    pub fn register(&mut self, tool: ToolDescriptor) {
        self.tools.insert(tool.name.clone(), tool);
    }

    pub fn extend(&mut self, tools: impl IntoIterator<Item = ToolDescriptor>) {
        for tool in tools {
            self.register(tool);
        }
    }

    pub fn descriptors(&self) -> Vec<ToolDescriptor> {
        self.tools.values().cloned().collect()
    }

    pub fn filtered(&self, policy: &ToolPolicy) -> Vec<ToolDescriptor> {
        policy.filter(self.descriptors())
    }

    pub fn get(&self, name: &str) -> Option<&ToolDescriptor> {
        self.tools.get(name)
    }
}
