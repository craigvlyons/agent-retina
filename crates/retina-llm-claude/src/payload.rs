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
- max_chars\n\
- page_start\n\
- page_end\n\
- overwrite\n\
- require_approval\n\
- expect_change\n\
- note\n\
- message\n\
- task_complete\n\
- reasoning\n\
\n\
Supported action types:\n\
- run_command\n\
- inspect_path\n\
- list_directory\n\
- find_files\n\
- search_text\n\
- read_file\n\
- extract_document_text\n\
- write_file\n\
- append_file\n\
- record_note\n\
- respond\n\
\n\
Planning rules:\n\
- The harness is your body. Explore through shell actions instead of guessing.\n\
- Treat the task state artifact as the canonical compact continuity record for this task.\n\
- Respect the task shape in task_state. Discovery tasks, answer tasks, transform tasks, and output tasks should be handled differently.\n\
- Choose the best next verifiable step that most reduces uncertainty or advances the requested result.\n\
- If the task already names likely source files, directories, or output paths, target those artifacts directly instead of broad parent-directory listing unless location is genuinely uncertain.\n\
- Use whichever action gives the clearest verifiable progress: structured file actions, document extraction, or run_command for local shell pipelines, text processing, or small local scripts.\n\
- Prefer readable text sources such as .md, .txt, code, and config files when multiple candidates could answer the task.\n\
- Use extract_document_text for PDFs and other document formats when reading raw bytes would be unhelpful.\n\
- If the task asks for specific PDF pages, set page_start and page_end so the shell extracts only that page range.\n\
- When a prior result already includes likely candidate paths, choose the best next ingest, transform, or write step instead of searching again.\n\
- When the last result already contains enough evidence to answer the user, respond directly instead of repeating exploration.\n\
- If multiple files match, prefer the shallowest and most human-readable candidate unless the task explicitly asks for another one.\n\
- If the user asks a question about content, gather the evidence first and then finish with respond once you can answer directly.\n\
- If the last result already gave enough evidence, do not repeat the same exploratory step.\n\
- If a request genuinely needs discovery first, choose the exploratory action and set task_complete=false.\n\
- Set task_complete=true only when the requested work is actually complete, not when you have only found a path or partial evidence.\n\
- For transform or output tasks, do not mark task_complete=true while required inputs remain un-ingested.\n\
- For tasks with a requested output artifact in task_state, do not mark task_complete=true until that output exists and is verified, unless you are explicitly surfacing a real blocker.\n\
\n\
Set require_approval=true only for delete-like or kill-like commands that need explicit operator approval.\n\
You are allowed to explore the workspace in bounded steps.\n\
Do not confuse discovery with progress toward a requested output; move from locating sources to ingesting them, then to synthesis or output creation when the evidence is ready.\n\
Path hints like Desktop, Documents, Downloads, and ~/ refer to locations under the user's home directory."
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
    let required_inputs = if request.context.task_state.shape.required_inputs.is_empty() {
        "- none".to_string()
    } else {
        request
            .context
            .task_state
            .shape
            .required_inputs
            .iter()
            .map(|input| format!("- {} [{}]", input.locator_hint, input.status))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let requested_output = request
        .context
        .task_state
        .shape
        .requested_output
        .as_ref()
        .map(|output| {
            format!(
                "- {} [exists={}, verified={}]",
                output.locator_hint, output.exists, output.verified
            )
        })
        .unwrap_or_else(|| "- none".to_string());
    let blockers = if request.context.task_state.frontier.blockers.is_empty() {
        "- none".to_string()
    } else {
        request
            .context
            .task_state
            .frontier
            .blockers
            .iter()
            .map(|blocker| format!("- {blocker}"))
            .collect::<Vec<_>>()
            .join("\n")
    };

    format!(
        "Constraints:\n{}\n\nTask shape:\n- kind: {}\n- required inputs:\n{}\n- requested output:\n{}\n- blockers:\n{}\n\nDynamic task context follows in the next block. Use it as the mutable working set for this step.",
        constraints,
        request.context.task_state.shape.kind,
        required_inputs,
        requested_output,
        blockers
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
