use crate::config::{
    ClaudeContextManagement, ClaudePromptCaching, model_supports_server_compaction,
};
use chrono::Local;
use retina_types::ReasonRequest;
use serde_json::json;

pub(crate) fn build_payload(
    model_id: &str,
    request: &ReasonRequest,
    reflection: bool,
    prompt_caching: &ClaudePromptCaching,
    context_management: &ClaudeContextManagement,
) -> serde_json::Value {
    let system_blocks = build_system_blocks(request, reflection, prompt_caching);
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
    request: &ReasonRequest,
    reflection: bool,
    prompt_caching: &ClaudePromptCaching,
) -> Vec<serde_json::Value> {
    let stable_instructions = build_stable_instructions(reflection, &request.tools);
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

pub(crate) fn build_stable_instructions(
    reflection: bool,
    tools: &[retina_types::ToolDescriptor],
) -> String {
    let tool_catalog = tool_catalog_block(tools);
    let has_list_mcp_resources = tools.iter().any(|tool| tool.name == "list_mcp_resources");
    let has_read_mcp_resource = tools.iter().any(|tool| tool.name == "read_mcp_resource");
    let has_generic_mcp_call = tools.iter().any(|tool| tool.name == "mcp_call");
    let has_concrete_mcp_tools = tools
        .iter()
        .any(|tool| retina_types::parse_mcp_tool_name(&tool.name).is_some());
    let has_mcp_search_tools = tools.iter().any(|tool| {
        retina_types::parse_mcp_tool_name(&tool.name)
            .map(|(_, tool_name)| {
                tool_name.contains("web_search")
                    || tool_name.contains("local_search")
                    || tool_name.contains("news_search")
                    || tool_name.ends_with("search")
            })
            .unwrap_or(false)
    });
    let mcp_instruction_block = build_mcp_instruction_block(
        has_list_mcp_resources,
        has_read_mcp_resource,
        has_generic_mcp_call,
        has_concrete_mcp_tools,
        has_mcp_search_tools,
    );
    format!(
        "You are the Retina agent reasoner.\n\
Reflection mode: {reflection}.\n\
Choose exactly one tool from the catalog and return strict JSON with this top-level shape:\n\
{{\n\
  \"type\": \"<exact tool name from the catalog>\",\n\
  \"input\": {{ ... arguments for that tool ... }},\n\
  \"task_complete\": false,\n\
  \"intent_kind\": \"answer\" | \"output\" | \"unknown\",\n\
  \"deliverable\": \"optional short noun phrase\",\n\
  \"completion_basis\": \"optional short grounded basis\",\n\
  \"reasoning\": \"optional brief rationale\"\n\
}}\n\
\n\
Top-level fields are limited to: type, input, task_complete, intent_kind, deliverable, completion_basis, reasoning.\n\
Always put tool arguments inside `input`. Use an empty object only when the chosen tool truly takes no arguments.\n\
\n\
Available tools:\n\
{tool_catalog}\n\
\n\
Planning rules:\n\
- The harness is your body. Explore through shell actions instead of guessing.\n\
- Choose one concrete next step that advances the task or reduces uncertainty.\n\
- You may choose only from the exact tool names listed above. Do not invent tool names or request unavailable tools.\n\
- If you use run_command to create or modify a specific artifact, set `input.path` to the target file so the harness can verify the change.\n\
- Use ingest_structured_data for CSV/TSV-style local data when headers, rows, or sample records matter more than plain prose.\n\
- Use extract_document_text for PDFs and other document formats when reading raw bytes would be unhelpful.\n\
- {mcp_instruction_block}\n\
- When the task needs current web information, local recommendations, or internet research and MCP search tools are available, prefer MCP tools over ad-hoc shell web scraping.\n\
- If a successful MCP search already returned concrete titles, links, or snippets that answer a recommendation or summary request, respond from that grounded result instead of repeating the same search.\n\
- For time-sensitive web tasks such as today, tonight, tomorrow, this weekend, current, or latest, use the current local date/time from context, prefer event-specific/date-specific search results over evergreen attraction pages, and do one more targeted search before answering if the first result is too generic.\n\
- For time-sensitive event searches, do not answer with generic city attractions, venue lists, or “types of things to do” unless the search results actually contain concrete event names or date-specific details. If the first result is only an events portal or broad listing page, do one more targeted search for specific events before responding. If you still cannot verify concrete events, say that clearly instead of inventing a lineup.\n\
- For time-sensitive event searches, do not fill gaps with general knowledge about the city. Do not mention attractions, neighborhoods, sports teams, venues, or activity categories unless they appear in the observed search results or snippets. If the results only prove that an events portal exists, say that and ask whether the user wants a narrower follow-up search.\n\
- If repeated MCP searches return the same top hit or the same generic portal page, do not run the same search again. Either materially reformulate the query to target specific event details or respond with the grounded limitation.\n\
- MCP tool identifiers and MCP locators are not files. Do not use read_file on values like `server/tool`, `mcp__server__tool`, or `mcp-tool://...`; continue with another MCP tool step or respond from the MCP result instead.\n\
- If an MCP tool call fails with an input validation error, retry at most once using only the required schema fields and a simpler argument payload before switching tools or responding with the grounded limitation.\n\
- Prefer edit_file for modifying existing text files when you know the exact old_string to replace.\n\
- Use write_file mainly for new files or complete rewrites. If the file already exists, read it first.\n\
- Use append_file only when adding content to the end is truly the intended edit.\n\
- Use edit_notebook for `.ipynb` files; do not use text mutation tools for notebooks.\n\
- Do not create or modify files unless the user asked for a saved artifact or a file change is actually required to complete the task. For answer-only tasks, prefer respond.\n\
- For respond, put the operator-facing answer in `input.message`.\n\
- When a file mutation succeeds, treat the saved artifact path and artifact result in task_state as the grounded source of truth for what was actually written.\n\
- If the task asks for specific PDF pages, set `input.page_start` and `input.page_end` so the shell extracts only that page range.\n\
- For find_files, keep `input.root` as a real directory path and keep `input.pattern` limited to a filename or glob; do not pack path fragments into pattern. Use `input.recursive=false` for top-level file requests on or in a folder unless the user asked for nested contents.\n\
- For search_text, keep `input.root` as the directory scope and keep `input.query` limited to search terms; do not combine them into one field.\n\
- Use agent_spawn only for a bounded delegated subtask whose result you will integrate back into the current task.\n\
- For agent_spawn, set `input.prompt` to the delegated objective. Use `input.allowed_tools` or `input.denied_tools` only when narrowing the child worker's tool pool is clearly helpful.\n\
- Set task_complete=true only when the requested work is actually complete, not when you have only found a path or partial evidence.\n\
- Discovery-only steps such as inspect_path, list_directory, find_files, and search_text are intermediate progress when the request still asks you to read, answer, summarize, extract, or create output.\n\
- In general, intermediate shell steps should not be marked task_complete=true. Use task_complete=true when you are returning a grounded final response or when a verified output/state change satisfies the task.\n\
- If a directory listing already gives the evidence needed to answer an inventory or summary question, respond from that grounded listing instead of repeating the same listing.\n\
- Directory listings include compact summary facts such as counts and sample names. For simple inventory tasks, prefer answering from that grounded listing instead of reopening files or re-listing the same path.\n\
- Treat requests for files on or in a folder as top-level scope by default. Use recursive listing or nested search only when the user says under, recursively, across subfolders, or otherwise clearly asks for nested contents.\n\
- For simple inventory or file-summary tasks, keep the response short and factual. Counts, notable items, and brief content summaries are usually enough unless the user asked for detailed writeups.\n\
- If task_state shows an explicit output artifact that still needs verification, do not mark task_complete=true until that artifact is verified, unless you are surfacing a grounded blocker.\n\
- For output-file tasks, keep the final response narrow and artifact-driven: report the saved path and summarize only what is supported by the saved artifact result or exact evidence.\n\
- For summaries or reports derived from local evidence, prefer extractive facts over interpretation. Do not invent action items, causes, themes, or explanations unless the user asked for analysis or the source states them directly.\n\
- If the source contains placeholders, tokens, or unknown markers such as `NO_SPEECH`, preserve them literally or mention them as observed text; do not explain what they mean unless the source explains them.\n\
- Do not add generated dates, labels, or metadata to an output artifact unless they were requested or are grounded in the observed evidence.\n\
- In reflection mode, do something materially different or report the grounded blocker/current status.\n\
- For terminal or system-control tasks, use command output as evidence, but do not assume the harness can fully verify world state for you.\n\
- Prefer hard verification for concrete artifact changes. For process, service, or environment checks, reason from the observed command evidence.\n\
- When repeated command checks no longer change the picture, stop varying the same check and either take a materially different action or respond with the current grounded status.\n\
- `intent_kind`, `deliverable`, and `completion_basis` are optional continuity metadata. When useful, keep `intent_kind` to: answer, output, or unknown.\n\
\n\
Set `input.require_approval=true` only for delete-like or kill-like run_command steps that need explicit operator approval.\n\
You are allowed to explore the workspace in bounded steps.\n\
Path hints like Desktop, Documents, Downloads, and ~/ are user-facing aliases that the file and directory tools can resolve directly. Prefer those tools first; do not spend a shell step on commands like `echo $HOME` unless direct path use actually failed."
    )
}

fn tool_catalog_block(tools: &[retina_types::ToolDescriptor]) -> String {
    if tools.is_empty() {
        return "- respond: Answer operator questions directly. Input: { message: string* }"
            .to_string();
    }

    tools
        .iter()
        .map(render_tool_catalog_entry)
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_tool_catalog_entry(tool: &retina_types::ToolDescriptor) -> String {
    let approval = match tool.approval {
        retina_types::ToolApprovalPolicy::None => None,
        retina_types::ToolApprovalPolicy::ExplicitOperatorApproval => Some("approval"),
        retina_types::ToolApprovalPolicy::ToolDefined => Some("conditional_approval"),
    };
    let mut traits = vec![tool.concurrency.label().to_string()];
    if tool.streaming {
        traits.push("streaming".to_string());
    }
    if let Some(flag) = approval {
        traits.push(flag.to_string());
    }
    if !tool.required_authority.is_empty() {
        traits.push(format!("requires {}", tool.required_authority.join(",")));
    }

    format!(
        "- {} [{}]: {} Input: {}",
        tool.name,
        traits.join(", "),
        tool.description.trim(),
        render_input_schema_for_prompt(&tool.input_schema)
    )
}

fn render_input_schema_for_prompt(schema: &serde_json::Value) -> String {
    let Some(properties) = schema
        .get("properties")
        .and_then(serde_json::Value::as_object)
    else {
        return "{}".to_string();
    };
    if properties.is_empty() {
        return "{}".to_string();
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
        .map(|(name, value)| {
            let field_type = value
                .get("type")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("value");
            if required.contains(name.as_str()) {
                format!("{name}: {field_type}*")
            } else {
                format!("{name}: {field_type}")
            }
        })
        .collect::<Vec<_>>();

    format!("{{ {} }}", fields.join(", "))
}

fn build_mcp_instruction_block(
    has_list_mcp_resources: bool,
    has_read_mcp_resource: bool,
    has_generic_mcp_call: bool,
    has_concrete_mcp_tools: bool,
    has_mcp_search_tools: bool,
) -> String {
    let mut lines = Vec::new();

    if has_concrete_mcp_tools {
        lines.push("Use the concrete MCP tool names from the tool catalog when those tools match the task.");
        lines.push("To call a concrete MCP tool, set `type` to the concrete tool name and put that tool's arguments in `input`. Example: `{ \"type\": \"mcp__brave__brave_web_search\", \"input\": { \"query\": \"colorado springs events this weekend\" } }`.");
        lines.push(
            "Treat a successful concrete MCP tool result as usable evidence for the next step.",
        );
    }
    if has_list_mcp_resources {
        lines.push(
            "Use list_mcp_resources to see current MCP resources when configured servers may contain the needed context.",
        );
    }
    if has_read_mcp_resource {
        lines
            .push("Use read_mcp_resource with `server` and `uri` to read a specific MCP resource.");
    }
    if has_generic_mcp_call {
        lines.push("Use mcp_call with `input.server`, `input.tool`, and `input.input_json` only when no concrete MCP tool name is available.");
    }
    if has_read_mcp_resource && (has_generic_mcp_call || has_concrete_mcp_tools) {
        lines.push("Do not follow a successful MCP tool call with read_mcp_resource unless you actually discovered a matching MCP resource URI.");
    }
    if has_mcp_search_tools {
        lines.push("When the user explicitly asks for web search, current information, local happenings, things to do this weekend, news, recommendations, or internet research, use the available MCP search tools first. Do not start with run_command, curl, or ad-hoc web scraping unless MCP search is unavailable or has already failed.");
    }

    if lines.is_empty() {
        "No MCP tools or MCP resources are available for this step.".to_string()
    } else {
        lines.join("\n- ")
    }
}

pub(crate) fn build_dynamic_context_block(request: &ReasonRequest) -> String {
    let current_local_time = Local::now().format("%A, %Y-%m-%d %H:%M %:z").to_string();
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
        "Constraints:\n{}\n\nCurrent local date/time:\n- {}\n\nObserved state snapshot:\n- output_written: {}\n- output_verified: {}\n\nDynamic task context follows in the next block. Treat task_state as the canonical live thread. Observations and verified tool results are the source of truth for this step.",
        constraints,
        current_local_time,
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
        "Write a compact continuation artifact for this Retina task. Preserve the task goal, progress, working sources, artifact references, failed attempts, and the clearest next unfinished obligation. Prefer exact file paths, IDs, and evidence references over vague prose. Keep it concise and continuation-oriented. Reflection mode: {reflection}. Task: {}. Wrap the result in <summary></summary>.",
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
                    recent_actions: vec![],
                    working_sources: vec![],
                    artifact_references: vec![],
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

    #[test]
    fn supported_action_types_follow_actual_tool_scope() {
        let instructions = build_stable_instructions(
            false,
            &[
                ToolDescriptor {
                    name: "respond".to_string(),
                    description: "Return a grounded answer.".to_string(),
                    source: ToolSourceKind::BuiltinShell,
                    concurrency: ToolConcurrencyClass::ReadOnly,
                    approval: ToolApprovalPolicy::None,
                    required_authority: Vec::new(),
                    streaming: false,
                    input_schema: serde_json::json!({
                        "type": "object",
                        "properties": { "message": { "type": "string" } },
                        "required": ["message"]
                    }),
                },
                ToolDescriptor {
                    name: "list_directory".to_string(),
                    description: "List a directory.".to_string(),
                    source: ToolSourceKind::BuiltinShell,
                    concurrency: ToolConcurrencyClass::ReadOnly,
                    approval: ToolApprovalPolicy::None,
                    required_authority: Vec::new(),
                    streaming: false,
                    input_schema: serde_json::json!({
                        "type": "object",
                        "properties": { "path": { "type": "string" } },
                        "required": ["path"]
                    }),
                },
            ],
        );
        assert!(instructions.contains("Available tools:"));
        assert!(instructions.contains("- respond [read_only]"));
        assert!(instructions.contains("Input: { message: string* }"));
        assert!(instructions.contains("- list_directory [read_only]"));
        assert!(!instructions.contains("- run_command"));
        assert!(instructions.contains("Do not invent tool names"));
    }
}
