use super::TaskLoopState;

pub(crate) fn describe_task_phase(
    state: &TaskLoopState,
    current_step: usize,
    max_steps: usize,
) -> String {
    if state.step_index == 0 {
        "starting".to_string()
    } else if current_step >= max_steps {
        "final step".to_string()
    } else {
        format!("working through step {} of {}", current_step, max_steps)
    }
}

pub(crate) fn build_task_frontier(state: &TaskLoopState) -> Vec<String> {
    state
        .avoid_rules
        .iter()
        .map(|avoid| format!("avoid repeating {} because {}", avoid.label, avoid.reason))
        .collect()
}
