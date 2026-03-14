// File boundary: keep lib.rs limited to module declarations and re-exports.
// New type families and helper logic should live in focused sibling modules.
mod actions;
mod agents;
mod core;
mod execution;
mod fabrication;
mod memory;
mod reasoning;
mod task_state;
mod tasking;

pub use actions::*;
pub use agents::*;
pub use core::*;
pub use execution::*;
pub use fabrication::*;
pub use memory::*;
pub use reasoning::*;
pub use task_state::*;
pub use tasking::*;

#[cfg(test)]
mod tests {
    use crate::{Action, ActionId, HashScope, PrivilegedCommandKind, classify_privileged_command};
    use std::path::PathBuf;

    #[test]
    fn privileged_command_classifier_only_flags_delete_and_kill_commands() {
        assert_eq!(
            classify_privileged_command("rm tmp/test.txt"),
            Some(PrivilegedCommandKind::Delete)
        );
        assert_eq!(
            classify_privileged_command("find . -name '*.tmp' -delete"),
            Some(PrivilegedCommandKind::Delete)
        );
        assert_eq!(
            classify_privileged_command("pkill retina"),
            Some(PrivilegedCommandKind::Kill)
        );
        assert_eq!(classify_privileged_command("mv a b"), None);
        assert_eq!(classify_privileged_command("chmod +x script.sh"), None);
        assert_eq!(classify_privileged_command("curl --version"), None);
    }

    #[test]
    fn only_delete_or_kill_commands_require_approval_by_policy() {
        let delete = Action::RunCommand {
            id: ActionId::new(),
            command: "rm tmp/test.txt".to_string(),
            cwd: None,
            require_approval: false,
            expect_change: true,
            state_scope: HashScope::default(),
        };
        let write = Action::WriteFile {
            id: ActionId::new(),
            path: PathBuf::from("tmp/test.txt"),
            content: "hello".to_string(),
            overwrite: true,
        };

        assert!(delete.approval_required_by_policy());
        assert!(!write.approval_required_by_policy());
    }
}
