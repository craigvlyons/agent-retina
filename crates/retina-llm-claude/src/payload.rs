use crate::config::{
    ClaudeContextManagement, ClaudePromptCaching, model_supports_server_compaction,
};
use retina_types::ReasonRequest;
use serde_json::json;

pub(crate) fn build_payload(
    model_id: &str,
    request: &ReasonRequest,
    reflection: bool,
    prompt_caching: &ClaudePromptCaching,
    context_management: &ClaudeContextManagement,
) -> serde_json::Value {
    let system_blocks = build_system_blocks(reflection, prompt_caching);
    let user_content = build_user_content_blocks(request);
    let mut payload = json!({
        "model": model_id,
        "max_tokens": request.max_tokens.unwrap_or(if reflection { 256 } else { 512 }),
        "system": system_blocks,
        "messages": [
            {
                "role": "user",
                "content": user_content
            }
        ]
    });

    if let Some(edits) =
        build_context_management_edits(model_id, request, reflection, context_management)
    {
        payload["context_management"] = json!({ "edits": edits });
    }

    payload
}

fn build_system_blocks(
    reflection: bool,
    prompt_caching: &ClaudePromptCaching,
) -> Vec<serde_json::Value> {
    let stable_instructions = build_stable_instructions(reflection);
    let mut blocks = vec![json!({
        "type": "text",
        "text": "Return JSON only. Do not wrap the response in markdown fences."
    })];

    let mut stable_block = json!({
        "type": "text",
        "text": stable_instructions
    });
    if prompt_caching.enabled {
        stable_block["cache_control"] = prompt_caching.cache_control_json();
    }
    blocks.push(stable_block);
    blocks
}

fn build_user_content_blocks(request: &ReasonRequest) -> Vec<serde_json::Value> {
    vec![
        json!({
            "type": "text",
            "text": build_dynamic_context_block(request)
        }),
        json!({
            "type": "text",
            "text": request.context.render()
        }),
    ]
}

pub(crate) fn build_stable_instructions(reflection: bool) -> String {
    format!(
        "You are the Retina agent reasoner.\n\
Reflection mode: {reflection}.\n\
Choose exactly one action and return strict JSON with these fields:\n\
- type\n\
- command\n\
- path\n\
- root\n\
- pattern\n\
- query\n\
- content\n\
- include_content\n\
- recursive\n\
- max_entries\n\
- max_results\n\
- max_bytes\n\
- max_rows\n\
- max_chars\n\
- page_start\n\
- page_end\n\
- overwrite\n\
- require_approval\n\
- expect_change\n\
- note\n\
- message\n\
- task_complete\n\
- intent_kind\n\
- deliverable\n\
- completion_basis\n\
- reasoning\n\
\n\
Supported action types:\n\
- run_command\n\
- inspect_path\n\
- list_directory\n\
- find_files\n\
- search_text\n\
- read_file\n\
- ingest_structured_data\n\
- extract_document_text\n\
- write_file\n\
- append_file\n\
- record_note\n\
- respond\n\
\n\
Planning rules:\n\
- The harness is your body. Explore through shell actions instead of guessing.\n\
- Choose one concrete next step that advances the task or reduces uncertainty.\n\
- If you use run_command to create or modify a specific artifact, set path to the target file so the harness can verify the change.\n\
- Use ingest_structured_data for CSV/TSV-style local data when headers, rows, or sample records matter more than plain prose.\n\
- Use extract_document_text for PDFs and other document formats when reading raw bytes would be unhelpful.\n\
- If the task asks for specific PDF pages, set page_start and page_end so the shell extracts only that page range.\n\
- For find_files, keep root as a real directory path and keep pattern limited to a filename or glob; do not pack path fragments into pattern.\n\
- For search_text, keep root as the directory scope and keep query limited to search terms; do not combine them into one field.\n\
- Set task_complete=true only when the requested work is actually complete, not when you have only found a path or partial evidence.\n\
- Discovery-only steps such as inspect_path, list_directory, find_files, and search_text are intermediate progress when the request still asks you to read, answer, summarize, extract, or create output.\n\
- In general, intermediate shell steps should not be marked task_complete=true. Use task_complete=true when you are returning a grounded final response or when a verified output/state change satisfies the task.\n\
- If task_state shows an explicit output artifact that still needs verification, do not mark task_complete=true until that artifact is verified, unless you are surfacing a grounded blocker.\n\
- In reflection mode, do something materially different or report the grounded blocker/current status.\n\
- For terminal or system-control tasks, use command output as evidence, but do not assume the harness can fully verify world state for you.\n\
- Prefer hard verification for concrete artifact changes. For process, service, or environment checks, reason from the observed command evidence.\n\
- When repeated command checks no longer change the picture, stop varying the same check and either take a materially different action or respond with the current grounded status.\n\
- `intent_kind`, `deliverable`, and `completion_basis` are optional continuity metadata. When useful, keep `intent_kind` to: answer, output, or unknown.\n\
\n\
Set require_approval=true only for delete-like or kill-like commands that need explicit operator approval.\n\
You are allowed to explore the workspace in bounded steps.\n\
Path hints like Desktop, Documents, Downloads, and ~/ are user-facing aliases; rely on shell verification rather than assuming a fixed underlying path."
    )
}

fn build_dynamic_context_block(request: &ReasonRequest) -> String {
    let constraints = if request.constraints.is_empty() {
        "none".to_string()
    } else {
        request
            .constraints
            .iter()
            .map(|constraint| format!("- {constraint}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    format!(
        "Constraints:\n{}\n\nObserved state snapshot:\n- output_written: {}\n- output_verified: {}\n\nDynamic task context follows in the next block. Treat task_state as the canonical live thread. Observations and verified tool results are the source of truth for this step.",
        constraints,
        request.context.task_state.progress.output_written,
        request.context.task_state.progress.output_verified
    )
}

fn build_context_management_edits(
    model_id: &str,
    request: &ReasonRequest,
    reflection: bool,
    context_management: &ClaudeContextManagement,
) -> Option<Vec<serde_json::Value>> {
    let mut edits = Vec::new();

    if context_management.tool_result_clearing_enabled {
        edits.push(json!({
            "type": "clear_tool_uses_20250919",
            "trigger": {
                "type": "input_tokens",
                "value": context_management.tool_result_trigger_tokens
            },
            "clear_tool_inputs": false
        }));
    }

    if context_management.server_side_compaction_enabled
        && model_supports_server_compaction(model_id)
    {
        edits.push(json!({
            "type": "compact_20260112",
            "trigger": {
                "type": "input_tokens",
                "value": context_management.compaction_trigger_tokens
            },
            "pause_after_compaction": false,
            "instructions": build_compaction_instructions(request, reflection)
        }));
    }

    if edits.is_empty() { None } else { Some(edits) }
}

fn build_compaction_instructions(request: &ReasonRequest, reflection: bool) -> String {
    format!(
        "Write a compact continuation artifact for this Retina task. Preserve the task goal, progress, working sources, artifact references, blockers, failed paths, and next frontier. Prefer exact file paths, IDs, and evidence references over vague prose. Keep it concise and continuation-oriented. Reflection mode: {reflection}. Task: {}. Wrap the result in <summary></summary>.",
        request.context.task
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use retina_types::*;

    fn sample_request() -> ReasonRequest {
        ReasonRequest {
            context: AssembledContext {
                identity: "Retina/root".to_string(),
                task: "what is Watcher.txt about?".to_string(),
                task_state: TaskState {
                    goal: TaskGoal {
                        objective: "what is Watcher.txt about?".to_string(),
                        constraints: vec![],
                    },
                    progress: TaskProgress {
                        current_phase: "working".to_string(),
                        current_step: 1,
                        max_steps: 50,
                        completed_checkpoints: vec![],
                        verified_facts: vec![],
                        output_written: false,
                        output_verified: false,
                    },
                    frontier: TaskFrontier::default(),
                    recent_actions: vec![],
                    working_sources: vec![],
                    artifact_references: vec![],
                    avoid: vec![],
                    compaction: None,
                },
                recent_context: Some(RecentContext {
                    prior_objective: "list files in texts".to_string(),
                    prior_answer_summary: Some("Watcher.txt is a meeting notes file.".to_string()),
                    sources: vec![WorkingSource {
                        kind: "file".to_string(),
                        locator: "C:/texts/Watcher.txt".to_string(),
                        role: "authoritative".to_string(),
                        status: "read".to_string(),
                        why_it_matters: "recent file".to_string(),
                        last_used_step: 2,
                        evidence_refs: vec![],
                        page_reference: None,
                        extraction_method: Some("text_read".to_string()),
                        structured_summary: None,
                        preview_excerpt: Some("watcher notes".to_string()),
                    }],
                    artifacts: vec![ArtifactReference {
                        kind: "file".to_string(),
                        locator: "C:/texts/Watcher.txt".to_string(),
                        status: "read".to_string(),
                    }],
                }),
                tools: vec![],
                memory_slice: vec![],
                last_result: None,
                last_result_summary: None,
                recent_steps: vec![],
                operator_guidance: None,
                current_step: 1,
                max_steps: 50,
            },
            tools: vec![],
            constraints: vec![],
            max_tokens: None,
        }
    }

    #[test]
    fn dynamic_context_block_stays_compact_and_task_state_canonical() {
        let block = build_dynamic_context_block(&sample_request());
        assert!(block.contains("Observed state snapshot"));
        assert!(block.contains("Treat task_state as the canonical live thread"));
        assert!(!block.contains("Recent conversational context"));
        assert!(!block.contains("compact source set"));
    }

    #[test]
    fn assembled_context_render_stays_minimal_and_does_not_replay_sections() {
        let rendered = sample_request().context.render();
        assert!(rendered.contains("Task state:"));
        assert!(rendered.contains("Tools:"));
        assert!(!rendered.contains("Recent conversational context:"));
        assert!(!rendered.contains("Memory:"));
        assert!(!rendered.contains("Recent steps:"));
        assert!(!rendered.contains("Last result summary:"));
        assert!(!rendered.contains("Last result:"));
    }
}
