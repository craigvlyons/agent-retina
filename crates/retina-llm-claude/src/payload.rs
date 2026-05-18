use crate::config::{
    ClaudeContextManagement, ClaudePromptCaching, model_supports_server_compaction,
};
use chrono::Local;
use retina_types::ReasonRequest;
use serde_json::json;

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn build_payload(
    model_id: &str,
    request: &ReasonRequest,
    reflection: bool,
    prompt_caching: &ClaudePromptCaching,
    context_management: &ClaudeContextManagement,
) -> serde_json::Value {
    build_payload_with_max_tokens(
        model_id,
        request,
        reflection,
        prompt_caching,
        context_management,
        None,
    )
}

pub(crate) fn build_payload_with_max_tokens(
    model_id: &str,
    request: &ReasonRequest,
    reflection: bool,
    prompt_caching: &ClaudePromptCaching,
    context_management: &ClaudeContextManagement,
    max_tokens_override: Option<u32>,
) -> serde_json::Value {
    let system_blocks = build_system_blocks(request, reflection, prompt_caching);
    let user_content = build_user_content_blocks(request);
    let mut payload = json!({
        "model": model_id,
        "max_tokens": resolved_max_tokens(request.max_tokens, reflection, max_tokens_override),
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

pub(crate) fn resolved_max_tokens(
    request_max_tokens: Option<u32>,
    reflection: bool,
    max_tokens_override: Option<u32>,
) -> u32 {
    max_tokens_override
        .or(request_max_tokens)
        .unwrap_or(if reflection { 256 } else { 512 })
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
- Search family semantics:\n\
  - use web search for broad discovery, official pages, and general internet research\n\
  - use news search for current-event coverage, roundup articles, and recent reporting\n\
  - use local search for place-aware venue or nearby business lookup when location resolution matters\n\
- If continuation_window next_step_guidance includes `preferred_search_family` or `suggested_query`, treat that as the preferred reformulation path unless the latest observed evidence clearly supports a better grounded alternative.\n\
- If recent_context is present, treat it as active follow-up continuity rather than background chatter. Reuse previously validated tools, library paths, working sources, saved artifacts, and successful execution paths before introducing a different stack.\n\
- If recent_context sticky_constraints mention reusing a library, toolchain, path, or prior execution path, treat that as binding unless the observed evidence shows it cannot satisfy the new task.\n\
- For time-sensitive web tasks such as today, tonight, tomorrow, this weekend, current, or latest, use the current local date/time from context, prefer event-specific/date-specific search results over evergreen attraction pages, and do one more targeted search before answering if the first result is too generic.\n\
- For time-sensitive event searches, do not answer with generic city attractions, venue lists, or “types of things to do” unless the search results actually contain concrete event names or date-specific details. If the first result is only an events portal or broad listing page, do one more targeted search for specific events before responding. If you still cannot verify concrete events, say that clearly instead of inventing a lineup.\n\
- For time-sensitive event searches, do not fill gaps with general knowledge about the city. Do not mention attractions, neighborhoods, sports teams, venues, or activity categories unless they appear in the observed search results or snippets. If the results only prove that an events portal exists, say that and ask whether the user wants a narrower follow-up search.\n\
- If repeated MCP searches return the same top hit or the same generic portal page, do not run the same search again. Either materially reformulate the query toward specific event details, switch to the preferred search family from continuation_window guidance, or respond with the grounded limitation.\n\
- MCP tool identifiers and MCP locators are not files. Do not use read_file on values like `server/tool`, `mcp__server__tool`, or `mcp-tool://...`; continue with another MCP tool step or respond from the MCP result instead.\n\
- If an MCP tool call fails with an input validation error, retry at most once using only the required schema fields and a simpler argument payload before switching tools or responding with the grounded limitation.\n\
- Prefer edit_file for modifying existing text files when you know the exact old_string to replace.\n\
- Use write_file mainly for new files or complete rewrites. If the file already exists, read it first.\n\
- Use append_file only when adding content to the end is truly the intended edit.\n\
- If the requested artifact is long enough that a full write_file payload might exceed one model response, create the file with an initial grounded section and then use append_file for additional grounded sections. Do not emit one oversized write_file JSON action that risks truncation.\n\
- Use edit_notebook for `.ipynb` files; do not use text mutation tools for notebooks.\n\
- Do not create or modify files unless the user asked for a saved artifact or a file change is actually required to complete the task. For answer-only tasks, prefer respond.\n\
- For respond, put the operator-facing answer in `input.message`.\n\
- For read_file, use `input.start_line` and `input.limit_lines` when you only need a specific region of a large file. If you expect to edit an existing file, prefer a full read before the mutation step instead of relying on a partial excerpt.\n\
- When a file mutation succeeds, treat the saved artifact path and the verified artifact/tool-result evidence in continuation_window as the grounded source of truth for what was actually written.\n\
- For multi-file batch tasks, do not mark task_complete=true immediately after writing one per-source artifact unless the user explicitly asked for separate output files. If the request implies one combined summary, report, or deliverable over many inputs, keep going until that combined artifact exists or you are surfacing a grounded blocker.\n\
- If the task asks for specific PDF pages, set `input.page_start` and `input.page_end` so the shell extracts only that page range.\n\
- For find_files, keep `input.root` as a real directory path and keep `input.pattern` limited to a filename or glob; do not pack path fragments into pattern. Use `input.recursive=false` for top-level file requests on or in a folder unless the user asked for nested contents.\n\
- For find_files, if a previous result was truncated and you need more of the same match set, continue with `input.offset` instead of restarting the same search from the beginning.\n\
- For search_text, keep `input.root` as the directory scope and keep `input.query` limited to search terms; do not combine them into one field.\n\
- For search_text, choose `input.output_mode=content` when you need exact matching lines, `files_with_matches` when you are deciding which files to read next, and `count` when you need prevalence or spread rather than raw excerpts.\n\
- For search_text, if a previous result was truncated and you still need more of the same result set, continue with `input.offset` instead of rerunning the same search from the beginning.\n\
- Use agent_spawn only for a bounded delegated subtask whose result you will integrate back into the current task.\n\
- For agent_spawn, set `input.prompt` to the delegated objective. Use `input.allowed_tools` or `input.denied_tools` only when narrowing the child worker's tool pool is clearly helpful.\n\
- Set task_complete=true only when the requested work is actually complete, not when you have only found a path or partial evidence.\n\
- Discovery-only steps such as inspect_path, list_directory, find_files, and search_text are intermediate progress when the request still asks you to read, answer, summarize, extract, or create output.\n\
- In general, intermediate shell steps should not be marked task_complete=true. Use task_complete=true when you are returning a grounded final response or when a verified output/state change satisfies the task.\n\
- If a directory listing already gives the evidence needed to answer an inventory or summary question, respond from that grounded listing instead of repeating the same listing.\n\
- Directory listings include compact summary facts such as counts and sample names. For simple inventory tasks, prefer answering from that grounded listing instead of reopening files or re-listing the same path.\n\
- Treat requests for files on or in a folder as top-level scope by default. Use recursive listing or nested search only when the user says under, recursively, across subfolders, or otherwise clearly asks for nested contents.\n\
- For simple inventory or file-summary tasks, keep the response short and factual. Counts, notable items, and brief content summaries are usually enough unless the user asked for detailed writeups.\n\
- If continuation_window still shows an explicit output artifact that needs verification, do not mark task_complete=true until that artifact is verified, unless you are surfacing a grounded blocker.\n\
- For output-file tasks, keep the final response narrow and artifact-driven: report the saved path and summarize only what is supported by the saved artifact result or exact evidence.\n\
- For summaries or reports derived from local evidence, prefer extractive facts over interpretation. Do not invent action items, causes, themes, or explanations unless the user asked for analysis or the source states them directly.\n\
- For batch reports over a folder or file set, account for every discovered requested input before finishing. If a directory listing or match set established 4 candidate PDFs, the final report should cover all 4 or explicitly state which items were excluded and why.\n\
- For mixed local-plus-web reports, keep web enrichment attached to the exact local entity. Omit same-name people, adjacent companies, directory noise, or broad industry filler unless the result snippet or title clearly disambiguates the exact person, company, or document named in the local source.\n\
- For mixed local-plus-web reports about people or companies, do not rely on people-search aggregators, generic directories, or broad location listings as if they were direct evidence about the exact entity. Treat sources like Spokeo, generic people finders, or unrelated regional directories as weak unless the result clearly and directly matches the exact entity in the local document.\n\
- For generated reports, do not add recommendation sections, action items, strategy advice, or market-context filler unless the user explicitly asked for recommendations, implications, or analysis.\n\
- Keep document-analysis sections limited to facts observed in the local source. If a fact came only from web research, put it in the web-research section instead of mixing it into the source-document profile.\n\
- Treat a web result as a direct current update only when the exact entity is clearly named or otherwise strongly disambiguated in the title/snippet/result itself. Generic earnings reports, sector roundups, location listings, or broad public-health pages are not direct updates about the target entity.\n\
- If current web research does not verify anything directly attributable to the exact entity, say that no direct current development was verified. Do not fill the gap with generic industry context just to make the section look complete.\n\
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
            let enum_suffix = value
                .get("enum")
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    let values = items
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .collect::<Vec<_>>();
                    if values.is_empty() {
                        String::new()
                    } else {
                        format!("({})", values.join("|"))
                    }
                })
                .unwrap_or_default();
            if required.contains(name.as_str()) {
                format!("{name}: {field_type}{enum_suffix}*")
            } else {
                format!("{name}: {field_type}{enum_suffix}")
            }
        })
        .collect::<Vec<_>>();

    format!("{{ {} }}", fields.join(", "))
}

fn build_mcp_instruction_block(
    has_list_mcp_resources: bool,
    has_read_mcp_resource: bool,
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
    if has_read_mcp_resource && has_concrete_mcp_tools {
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
        "Constraints:\n{}\n\nCurrent local date/time:\n- {}\n\nDynamic task context follows in the next block. Treat continuation_window as the canonical live thread. Observations and verified tool results are the source of truth for this step.",
        constraints, current_local_time
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
                recent_context: Some(RecentContext {
                    prior_objective: "list files in texts".to_string(),
                    prior_answer_summary: Some("Watcher.txt is a meeting notes file.".to_string()),
                    sticky_constraints: vec![
                        "Reuse the validated local file path before searching elsewhere."
                            .to_string(),
                    ],
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
                continuation_window: ActiveContinuationWindow::default(),
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
    fn dynamic_context_block_stays_compact_and_continuation_canonical() {
        let block = build_dynamic_context_block(&sample_request());
        assert!(block.contains("Treat continuation_window as the canonical live thread"));
        assert!(!block.contains("Observed state snapshot"));
        assert!(!block.contains("Recent conversational context"));
        assert!(!block.contains("compact source set"));
    }

    #[test]
    fn assembled_context_render_includes_active_continuation_window_without_old_replay_sections() {
        let mut request = sample_request();
        request.context.continuation_window = ActiveContinuationWindow {
            objective: request.context.task.clone(),
            current_step: 1,
            max_steps: 50,
            reasoner_tokens_used: 0,
            max_output_tokens_recovery_count: 0,
            has_attempted_prompt_too_long_compaction: false,
            last_transition: None,
            read_state_cache: Vec::new(),
            search_state_cache: Vec::new(),
            transcript: TranscriptLedger::from_entries(vec![TranscriptUnit {
                ordinal: 1,
                step: 1,
                kind: TranscriptUnitKind::ToolResult,
                summary: "search hit".to_string(),
                result_ref_id: Some("result-1-1".to_string()),
                primary_locator: Some("https://example.com/weekend".to_string()),
                evidence_refs: vec!["https://example.com/weekend".to_string()],
                working_sources: Vec::new(),
                artifact_references: Vec::new(),
                next_step_guidance: None,
                repetition_signature: None,
                avoid_label: None,
                compaction_snapshot: None,
            }]),
            stored_results: StoredResultLedger::from_entries(vec![StoredResultReference {
                result_id: "result-1-1".to_string(),
                source_transcript_ordinal: 1,
                step: 1,
                result_type: "mcp_tool_call".to_string(),
                primary_locator: Some("https://example.com/weekend".to_string()),
                preview_excerpt: "generic_portal".to_string(),
                persisted_path: "/tmp/result-1-1.json".to_string(),
            }]),
            content_replacements: ContentReplacementState::from_continuation(
                &StoredResultLedger::from_entries(vec![StoredResultReference {
                    result_id: "result-1-1".to_string(),
                    source_transcript_ordinal: 1,
                    step: 1,
                    result_type: "mcp_tool_call".to_string(),
                    primary_locator: Some("https://example.com/weekend".to_string()),
                    preview_excerpt: "generic_portal".to_string(),
                    persisted_path: "/tmp/result-1-1.json".to_string(),
                }]),
                &[],
            ),
            reannounced_sources: Vec::new(),
            reannounced_artifacts: Vec::new(),
            next_step_guidance: None,
            compaction_boundaries: Vec::new(),
            reannounced_compacted_results: Vec::new(),
        };
        let rendered = request.context.render();
        assert!(rendered.contains("Active continuation window:"));
        assert!(rendered.contains("Recent context:"));
        assert!(rendered.contains("sticky_constraints:"));
        assert!(!rendered.contains("Derived task summary:"));
        assert!(rendered.contains("generic_portal"));
        assert!(rendered.contains("[stored-result result-1-1]"));
        assert!(!rendered.contains("carryover:"));
        assert!(!rendered.contains("content_replacements:"));
        assert!(!rendered.contains("stored_result_refs:"));
        assert!(!rendered.contains("reannounced_sources:"));
        assert!(!rendered.contains("reannounced_artifacts:"));
        assert!(!rendered.contains("next_step_guidance:"));
        assert!(!rendered.contains("compaction_boundaries:"));
        assert!(!rendered.contains("reannounced_compacted_results:"));
        assert!(rendered.contains("Tools:"));
        assert!(!rendered.contains("Operator guidance:"));
        assert!(!rendered.contains("Memory:"));
        assert!(!rendered.contains("Recent steps:"));
        assert!(!rendered.contains("Last result summary:"));
    }

    #[test]
    fn assembled_context_render_hides_compacted_result_ref_section_when_replacement_exists() {
        let request = ReasonRequest {
            context: AssembledContext {
                identity: "Retina/root".to_string(),
                task: "summarize compacted result".to_string(),
                continuation_window: ActiveContinuationWindow {
                    transcript: TranscriptLedger::from_entries(vec![TranscriptUnit {
                        ordinal: 1,
                        step: 1,
                        kind: TranscriptUnitKind::ToolResult,
                        summary: "large directory listing".to_string(),
                        result_ref_id: Some("boundary-3".to_string()),
                        primary_locator: Some("/tmp".to_string()),
                        evidence_refs: vec!["/tmp".to_string()],
                        working_sources: Vec::new(),
                        artifact_references: Vec::new(),
                        next_step_guidance: None,
                        repetition_signature: None,
                        avoid_label: None,
                        compaction_snapshot: None,
                    }]),
                    content_replacements: ContentReplacementState::from_continuation(
                        &StoredResultLedger::default(),
                        &[CompactedResultReference {
                            boundary_id: 3,
                            result_type: "directory_listing".to_string(),
                            locator: Some("/tmp".to_string()),
                            preview_excerpt: "preview".to_string(),
                            continuation: None,
                            persisted_path: Some("/tmp/boundary-3.json".to_string()),
                        }],
                    ),
                    reannounced_compacted_results: vec![CompactedResultReference {
                        boundary_id: 3,
                        result_type: "directory_listing".to_string(),
                        locator: Some("/tmp".to_string()),
                        preview_excerpt: "preview".to_string(),
                        continuation: None,
                        persisted_path: Some("/tmp/boundary-3.json".to_string()),
                    }],
                    ..ActiveContinuationWindow::default()
                },
                recent_context: None,
                tools: Vec::new(),
                memory_slice: Vec::new(),
                operator_guidance: None,
                current_step: 1,
                max_steps: 4,
            },
            tools: Vec::new(),
            constraints: Vec::new(),
            max_tokens: None,
        };

        let rendered = request.context.render();
        assert!(rendered.contains("[compacted-result boundary=3]"));
        assert!(!rendered.contains("carryover:"));
        assert!(!rendered.contains("content_replacements:"));
        assert!(!rendered.contains("reannounced_compacted_results:"));
        assert!(!rendered.contains("Recent context:"));
        assert!(!rendered.contains("Operator guidance:"));
    }

    #[test]
    fn assembled_context_render_uses_compaction_carryover_summary() {
        let request = ReasonRequest {
            context: AssembledContext {
                identity: "Retina/root".to_string(),
                task: "continue after compaction".to_string(),
                continuation_window: ActiveContinuationWindow {
                    transcript: TranscriptLedger::from_entries(vec![TranscriptUnit {
                        ordinal: 1,
                        step: 6,
                        kind: TranscriptUnitKind::CompactSummary,
                        summary: "compaction carried forward: reason=\"step threshold\" summary=\"Resume from the saved report context\" preserved_locator_count=1 continuation=\"Continue from the saved report\"".to_string(),
                        result_ref_id: None,
                        primary_locator: None,
                        evidence_refs: vec!["report.md".to_string()],
                        working_sources: Vec::new(),
                        artifact_references: Vec::new(),
                        next_step_guidance: None,
                        repetition_signature: None,
                        avoid_label: None,
                        compaction_snapshot: None,
                    }]),
                    compaction_boundaries: vec![CompactionSnapshot {
                        boundary_id: 5,
                        compacted_at_step: 6,
                        reason: "step threshold".to_string(),
                        score_explanations: vec![CompactionScoreExplanation {
                            item_kind: "source".to_string(),
                            locator: "report.md".to_string(),
                            decision: "keep".to_string(),
                            rationale: "authoritative".to_string(),
                        }],
                        preserved_locators: vec!["report.md".to_string()],
                        active_window_summary: "Resume from the saved report context".to_string(),
                        last_result_continuation: Some(
                            "Continue from the saved report".to_string(),
                        ),
                        compacted_results: Vec::new(),
                    }],
                    ..ActiveContinuationWindow::default()
                },
                recent_context: None,
                tools: Vec::new(),
                memory_slice: Vec::new(),
                operator_guidance: None,
                current_step: 1,
                max_steps: 4,
            },
            tools: Vec::new(),
            constraints: Vec::new(),
            max_tokens: None,
        };

        let rendered = request.context.render();
        assert!(rendered.contains("compaction carried forward:"));
        assert!(rendered.contains("[compact_summary]"));
        assert!(rendered.contains("summary=\"Resume from the saved report context\""));
        assert!(rendered.contains("preserved_locator_count=1"));
        assert!(!rendered.contains("carried forward context:"));
        assert!(!rendered.contains("- carryover:"));
        assert!(!rendered.contains("compaction_boundaries:"));
        assert!(!rendered.contains("boundary_id: 5"));
        assert!(!rendered.contains("ranking:"));
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

    #[test]
    fn stable_instructions_include_search_family_and_guidance_semantics() {
        let instructions = build_stable_instructions(
            false,
            &[ToolDescriptor {
                name: "mcp__brave__brave_web_search".to_string(),
                description: "Search the web".to_string(),
                source: ToolSourceKind::McpServer,
                concurrency: ToolConcurrencyClass::Streaming,
                approval: ToolApprovalPolicy::None,
                required_authority: vec!["mcp".to_string()],
                streaming: true,
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": { "query": { "type": "string" } },
                    "required": ["query"]
                }),
            }],
        );
        assert!(instructions.contains("Search family semantics"));
        assert!(instructions.contains("preferred_search_family"));
        assert!(instructions.contains("suggested_query"));
        assert!(instructions.contains("active follow-up continuity"));
        assert!(instructions.contains("sticky_constraints"));
        assert!(instructions.contains("output_mode=content"));
        assert!(instructions.contains("files_with_matches"));
        assert!(instructions.contains("count"));
        assert!(instructions.contains("start_line"));
        assert!(instructions.contains("limit_lines"));
        assert!(instructions.contains("input.offset"));
    }

    #[test]
    fn tool_catalog_renders_enum_values_for_search_modes() {
        let instructions = build_stable_instructions(
            false,
            &[ToolDescriptor {
                name: "search_text".to_string(),
                description: "Search local text.".to_string(),
                source: ToolSourceKind::BuiltinShell,
                concurrency: ToolConcurrencyClass::ReadOnly,
                approval: ToolApprovalPolicy::None,
                required_authority: Vec::new(),
                streaming: false,
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "root": { "type": "string" },
                        "query": { "type": "string" },
                        "output_mode": {
                            "type": "string",
                            "enum": ["content", "files_with_matches", "count"]
                        }
                    },
                    "required": ["root", "query"]
                }),
            }],
        );
        assert!(instructions.contains("output_mode: string(content|files_with_matches|count)"));
    }
}
