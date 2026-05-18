use super::*;
use crate::state_helpers::{
    looks_binary, normalize_requested_page_range, prefers_document_extraction,
};
use std::io::BufRead;

const DEFAULT_MAX_STRUCTURED_ROWS: usize = 25;

pub(crate) struct StructuredDataIngestion {
    pub(crate) format: String,
    pub(crate) headers: Vec<String>,
    pub(crate) rows: Vec<StructuredDataRow>,
    pub(crate) total_rows: usize,
    pub(crate) truncated: bool,
    pub(crate) extraction_method: String,
}

pub(crate) struct TextReadResult {
    pub(crate) content: String,
    pub(crate) truncated: bool,
    pub(crate) start_line: usize,
    pub(crate) line_count: usize,
    pub(crate) total_lines: usize,
    pub(crate) total_bytes: usize,
    pub(crate) read_bytes: usize,
    pub(crate) was_partial: bool,
}

pub(crate) struct FileMatchResult {
    pub(crate) matches: Vec<PathBuf>,
    pub(crate) truncated: bool,
    pub(crate) applied_offset: usize,
}

pub(crate) struct TextSearchResult {
    pub(crate) matches: Vec<SearchMatch>,
    pub(crate) content: Option<String>,
    pub(crate) filenames: Vec<PathBuf>,
    pub(crate) num_files: usize,
    pub(crate) num_matches: usize,
    pub(crate) truncated: bool,
    pub(crate) applied_offset: usize,
}

impl CliShell {
    pub(crate) fn list_directory(
        path: &Path,
        recursive: bool,
        max_entries: usize,
    ) -> Result<Vec<DirectoryEntry>> {
        let root = Self::resolve_path(path)?;
        let mut entries = Vec::new();
        Self::collect_directory_entries(&root, recursive, max_entries.max(1), &mut entries)?;
        entries.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(entries)
    }

    pub(crate) fn summarize_directory_entries(
        entries: &[DirectoryEntry],
    ) -> DirectoryListingSummary {
        let total_entries = entries.len();
        let file_count = entries.iter().filter(|entry| !entry.is_dir).count();
        let dir_count = entries.iter().filter(|entry| entry.is_dir).count();
        let hidden_count = entries
            .iter()
            .filter(|entry| {
                entry
                    .path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .map(|name| name.starts_with('.'))
                    .unwrap_or(false)
            })
            .count();
        let sample_names = entries
            .iter()
            .take(8)
            .filter_map(|entry| {
                entry
                    .path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .map(|name| name.to_string())
            })
            .collect::<Vec<_>>();

        DirectoryListingSummary {
            total_entries,
            file_count,
            dir_count,
            hidden_count,
            sample_names,
        }
    }

    fn collect_directory_entries(
        dir: &Path,
        recursive: bool,
        max_entries: usize,
        entries: &mut Vec<DirectoryEntry>,
    ) -> Result<()> {
        if entries.len() >= max_entries {
            return Ok(());
        }
        for item in fs::read_dir(dir)? {
            if entries.len() >= max_entries {
                break;
            }
            let item = item?;
            let path = item.path();
            let metadata = item.metadata()?;
            entries.push(DirectoryEntry {
                path: path.clone(),
                is_dir: metadata.is_dir(),
                size: if metadata.is_file() {
                    Some(metadata.len())
                } else {
                    None
                },
            });
            if recursive && metadata.is_dir() {
                Self::collect_directory_entries(&path, true, max_entries, entries)?;
            }
        }
        Ok(())
    }

    pub(crate) fn find_files(
        root: &Path,
        pattern: &str,
        recursive: bool,
        max_results: usize,
        offset: usize,
    ) -> Result<FileMatchResult> {
        let root = Self::resolve_path(root)?;
        let mut matches = Vec::new();
        Self::collect_matching_files(
            &root,
            &pattern.to_lowercase(),
            recursive,
            offset.saturating_add(max_results.max(1)).saturating_add(1),
            &mut matches,
        )?;
        matches.sort();
        let applied_offset = offset.min(matches.len());
        let limit = max_results.max(1);
        let remaining = &matches[applied_offset..];
        let truncated = remaining.len() > limit;
        let matches = remaining.iter().take(limit).cloned().collect::<Vec<_>>();
        Ok(FileMatchResult {
            matches,
            truncated,
            applied_offset,
        })
    }

    fn collect_matching_files(
        root: &Path,
        pattern: &str,
        recursive: bool,
        max_results: usize,
        matches: &mut Vec<PathBuf>,
    ) -> Result<()> {
        if matches.len() >= max_results {
            return Ok(());
        }
        for item in fs::read_dir(root)? {
            if matches.len() >= max_results {
                break;
            }
            let item = item?;
            let path = item.path();
            let metadata = item.metadata()?;
            let name = path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_lowercase();
            let path_text = path.display().to_string().to_lowercase();

            if path_matches_pattern(&name, &path_text, pattern) {
                matches.push(path.clone());
                if matches.len() >= max_results {
                    break;
                }
            }

            if recursive && metadata.is_dir() {
                Self::collect_matching_files(&path, pattern, true, max_results, matches)?;
                continue;
            }
        }
        Ok(())
    }

    pub(crate) fn search_text(
        root: &Path,
        query: &str,
        max_results: usize,
        offset: usize,
        glob: Option<&str>,
        case_insensitive: bool,
        output_mode: &TextSearchOutputMode,
    ) -> Result<TextSearchResult> {
        let root = Self::resolve_path(root)?;
        if let Some(result) = Self::search_text_with_ripgrep(
            &root,
            query,
            max_results,
            offset,
            glob,
            case_insensitive,
            output_mode,
        )? {
            return Ok(result);
        }
        if !matches!(output_mode, TextSearchOutputMode::Content) {
            return Err(KernelError::Unsupported(
                "search_text files_with_matches/count fallback requires ripgrep".to_string(),
            ));
        }
        let mut matches = Vec::new();
        Self::collect_text_matches(
            &root,
            query,
            glob.map(|value| value.to_lowercase()),
            case_insensitive,
            offset.saturating_add(max_results.max(1)).saturating_add(1),
            &mut matches,
        )?;
        matches.sort_by(|left, right| {
            left.path
                .cmp(&right.path)
                .then(left.line_number.cmp(&right.line_number))
        });
        let applied_offset = offset.min(matches.len());
        let limit = max_results.max(1);
        let remaining = &matches[applied_offset..];
        let truncated = remaining.len() > limit;
        let matches = remaining.iter().take(limit).cloned().collect::<Vec<_>>();
        let content = if matches.is_empty() {
            None
        } else {
            Some(
                matches
                    .iter()
                    .map(|item| {
                        format!("{}:{}:{}", item.path.display(), item.line_number, item.line)
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
            )
        };
        Ok(TextSearchResult {
            num_matches: matches.len(),
            matches,
            content,
            filenames: Vec::new(),
            num_files: 0,
            truncated,
            applied_offset,
        })
    }

    fn collect_text_matches(
        root: &Path,
        query: &str,
        glob: Option<String>,
        case_insensitive: bool,
        max_results: usize,
        matches: &mut Vec<SearchMatch>,
    ) -> Result<()> {
        if matches.len() >= max_results {
            return Ok(());
        }
        for item in fs::read_dir(root)? {
            if matches.len() >= max_results {
                break;
            }
            let item = item?;
            let path = item.path();
            let metadata = item.metadata()?;
            if metadata.is_dir() {
                Self::collect_text_matches(
                    &path,
                    query,
                    glob.clone(),
                    case_insensitive,
                    max_results,
                    matches,
                )?;
                continue;
            }
            if metadata.len() > 512 * 1024 {
                continue;
            }
            if let Some(pattern) = glob.as_ref() {
                let name = path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or_default()
                    .to_lowercase();
                let path_text = path.display().to_string().to_lowercase();
                if !path_matches_pattern(&name, &path_text, pattern) {
                    continue;
                }
            }
            let bytes = fs::read(&path)?;
            if bytes.contains(&0) {
                continue;
            }
            let content = String::from_utf8_lossy(&bytes);
            let query_cmp = if case_insensitive {
                query.to_lowercase()
            } else {
                query.to_string()
            };
            for (index, line) in content.lines().enumerate() {
                let haystack = if case_insensitive {
                    line.to_lowercase()
                } else {
                    line.to_string()
                };
                if haystack.contains(&query_cmp) {
                    matches.push(SearchMatch {
                        path: path.clone(),
                        line_number: index + 1,
                        line: line.to_string(),
                    });
                    if matches.len() >= max_results {
                        break;
                    }
                }
            }
        }
        Ok(())
    }

    fn search_text_with_ripgrep(
        root: &Path,
        query: &str,
        max_results: usize,
        offset: usize,
        glob: Option<&str>,
        case_insensitive: bool,
        output_mode: &TextSearchOutputMode,
    ) -> Result<Option<TextSearchResult>> {
        let mut command = std::process::Command::new("rg");
        command.arg("--color").arg("never");
        match output_mode {
            TextSearchOutputMode::Content => {
                command.arg("-n").arg("--no-heading");
            }
            TextSearchOutputMode::FilesWithMatches => {
                command.arg("-l");
            }
            TextSearchOutputMode::Count => {
                command.arg("-c");
            }
        }
        if case_insensitive {
            command.arg("-i");
        }
        if let Some(pattern) = glob {
            command.arg("--glob").arg(pattern);
        }
        command.arg(query).arg(root);

        let output = match command.output() {
            Ok(output) => output,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(KernelError::Execution(format!(
                    "failed to invoke ripgrep: {error}"
                )));
            }
        };

        if !output.status.success() && output.status.code() != Some(1) {
            return Err(KernelError::Execution(format!(
                "ripgrep failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }

        let limit = max_results.max(1);
        let capture_limit = offset.saturating_add(limit).saturating_add(1);
        let stdout = String::from_utf8_lossy(&output.stdout);
        match output_mode {
            TextSearchOutputMode::Content => {
                let mut matches = Vec::new();
                for line in stdout.lines() {
                    if matches.len() >= capture_limit {
                        break;
                    }
                    let mut parts = line.splitn(3, ':');
                    let Some(path) = parts.next() else {
                        continue;
                    };
                    let Some(line_number) =
                        parts.next().and_then(|value| value.parse::<usize>().ok())
                    else {
                        continue;
                    };
                    let Some(content) = parts.next() else {
                        continue;
                    };
                    matches.push(SearchMatch {
                        path: PathBuf::from(path),
                        line_number,
                        line: content.to_string(),
                    });
                }

                let applied_offset = offset.min(matches.len());
                let remaining = &matches[applied_offset..];
                let truncated = remaining.len() > limit;
                let matches = remaining.iter().take(limit).cloned().collect::<Vec<_>>();
                let content = if matches.is_empty() {
                    None
                } else {
                    Some(
                        matches
                            .iter()
                            .map(|item| {
                                format!(
                                    "{}:{}:{}",
                                    item.path.display(),
                                    item.line_number,
                                    item.line
                                )
                            })
                            .collect::<Vec<_>>()
                            .join("\n"),
                    )
                };
                Ok(Some(TextSearchResult {
                    num_matches: matches.len(),
                    matches,
                    content,
                    filenames: Vec::new(),
                    num_files: 0,
                    truncated,
                    applied_offset,
                }))
            }
            TextSearchOutputMode::FilesWithMatches => {
                let mut filenames = Vec::new();
                for line in stdout.lines() {
                    if filenames.len() >= capture_limit {
                        break;
                    }
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    filenames.push(PathBuf::from(trimmed));
                }
                let applied_offset = offset.min(filenames.len());
                let remaining = &filenames[applied_offset..];
                let truncated = remaining.len() > limit;
                let filenames = remaining.iter().take(limit).cloned().collect::<Vec<_>>();
                let num_files = filenames.len();
                Ok(Some(TextSearchResult {
                    matches: Vec::new(),
                    content: None,
                    filenames,
                    num_files,
                    num_matches: 0,
                    truncated,
                    applied_offset,
                }))
            }
            TextSearchOutputMode::Count => {
                let mut lines = Vec::new();
                let mut num_files = 0usize;
                let mut num_matches = 0usize;
                for line in stdout.lines() {
                    if lines.len() >= capture_limit {
                        break;
                    }
                    let Some((path, count_text)) = line.rsplit_once(':') else {
                        continue;
                    };
                    let Ok(count) = count_text.parse::<usize>() else {
                        continue;
                    };
                    lines.push(format!("{path}:{count}"));
                    num_files += 1;
                    num_matches += count;
                }
                let applied_offset = offset.min(lines.len());
                let remaining = &lines[applied_offset..];
                let truncated = remaining.len() > limit;
                let selected = remaining.iter().take(limit).cloned().collect::<Vec<_>>();
                let content = if selected.is_empty() {
                    None
                } else {
                    Some(selected.join("\n"))
                };
                Ok(Some(TextSearchResult {
                    matches: Vec::new(),
                    content,
                    filenames: Vec::new(),
                    num_files,
                    num_matches,
                    truncated,
                    applied_offset,
                }))
            }
        }
    }

    pub(crate) fn read_file(
        path: &Path,
        start_line: Option<usize>,
        limit_lines: Option<usize>,
        max_bytes: Option<usize>,
    ) -> Result<TextReadResult> {
        let path = Self::resolve_path(path)?;
        if prefers_document_extraction(&path) {
            return Err(KernelError::Unsupported(format!(
                "read_file is not suitable for {}; use extract_document_text instead",
                path.display()
            )));
        }
        let metadata = fs::metadata(&path)?;
        if metadata.is_dir() {
            return Err(KernelError::Execution(format!(
                "EISDIR: illegal operation on a directory, read '{}'",
                path.display()
            )));
        }

        let file = fs::File::open(&path)?;
        let mut reader = io::BufReader::new(file);
        let requested_start_line = start_line.unwrap_or(1).max(1);
        let requested_limit_lines = limit_lines.filter(|value| *value > 0);
        let limit = max_bytes.unwrap_or(DEFAULT_MAX_READ_BYTES);
        let max_selected_line = requested_limit_lines
            .map(|count| requested_start_line.saturating_add(count).saturating_sub(1));

        let mut content = String::new();
        let mut line_count = 0usize;
        let mut total_lines = 0usize;
        let mut selected_bytes = 0usize;
        let mut saw_binary = false;
        let mut hit_byte_limit = false;
        let mut raw_line = Vec::new();

        loop {
            raw_line.clear();
            let bytes_read = reader.read_until(b'\n', &mut raw_line)?;
            if bytes_read == 0 {
                break;
            }
            total_lines += 1;

            if raw_line.contains(&0) {
                saw_binary = true;
                break;
            }

            if total_lines == 1 && raw_line.starts_with(&[0xEF, 0xBB, 0xBF]) {
                raw_line.drain(..3);
            }

            let had_line_terminator = if raw_line.ends_with(b"\n") {
                raw_line.pop();
                if raw_line.ends_with(b"\r") {
                    raw_line.pop();
                }
                true
            } else if raw_line.ends_with(b"\r") {
                raw_line.pop();
                true
            } else {
                false
            };

            let in_requested_range = total_lines >= requested_start_line
                && max_selected_line
                    .map(|end| total_lines <= end)
                    .unwrap_or(true);
            if !in_requested_range || hit_byte_limit {
                continue;
            }

            let line = String::from_utf8_lossy(&raw_line).to_string();
            let mut fragment = line;
            if had_line_terminator {
                fragment.push('\n');
            }
            let next_bytes = selected_bytes + fragment.len();
            if next_bytes > limit {
                hit_byte_limit = true;
                continue;
            }

            selected_bytes = next_bytes;
            line_count += 1;
            content.push_str(&fragment);
        }

        if saw_binary || (content.is_empty() && total_lines == 1 && looks_binary(&raw_line)) {
            return Err(KernelError::Unsupported(format!(
                "read_file only supports text-like files; {} appears to be binary",
                path.display()
            )));
        }

        let requested_end_line = max_selected_line.unwrap_or(usize::MAX);
        let range_truncated = total_lines > requested_end_line;
        let truncated = hit_byte_limit || range_truncated;
        let read_bytes = content.len();

        Ok(TextReadResult {
            content,
            truncated,
            start_line: requested_start_line,
            line_count,
            total_lines,
            total_bytes: metadata.len() as usize,
            read_bytes,
            was_partial: requested_start_line > 1 || requested_limit_lines.is_some() || truncated,
        })
    }

    pub(crate) fn ingest_structured_data(
        path: &Path,
        max_rows: Option<usize>,
    ) -> Result<StructuredDataIngestion> {
        let path = Self::resolve_path(path)?;
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_lowercase();
        let delimiter = match extension.as_str() {
            "csv" => b',',
            "tsv" => b'\t',
            _ => {
                return Err(KernelError::Unsupported(format!(
                    "structured data ingestion is only supported for csv/tsv right now: {}",
                    path.display()
                )));
            }
        };

        let mut reader = csv::ReaderBuilder::new()
            .delimiter(delimiter)
            .flexible(true)
            .from_path(&path)
            .map_err(|error| KernelError::Execution(error.to_string()))?;

        let headers = reader
            .headers()
            .map_err(|error| KernelError::Execution(error.to_string()))?
            .iter()
            .map(|value| value.trim().to_string())
            .collect::<Vec<_>>();

        let sample_limit = max_rows.unwrap_or(DEFAULT_MAX_STRUCTURED_ROWS).max(1);
        let mut rows = Vec::new();
        let mut total_rows = 0usize;

        for record in reader.records() {
            let record = record.map_err(|error| KernelError::Execution(error.to_string()))?;
            total_rows += 1;
            if rows.len() < sample_limit {
                rows.push(StructuredDataRow {
                    row_number: total_rows,
                    values: record
                        .iter()
                        .map(|value| value.trim().to_string())
                        .collect(),
                });
            }
        }

        Ok(StructuredDataIngestion {
            format: extension,
            headers,
            rows,
            total_rows,
            truncated: total_rows > sample_limit,
            extraction_method: "csv_reader".to_string(),
        })
    }

    pub(crate) fn extract_document_text(
        path: &Path,
        max_chars: Option<usize>,
        page_start: Option<usize>,
        page_end: Option<usize>,
    ) -> Result<(
        String,
        bool,
        String,
        String,
        Option<DocumentPageRange>,
        bool,
    )> {
        let path = Self::resolve_path(path)?;
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_lowercase();

        let requested_page_range = normalize_requested_page_range(page_start, page_end)?;
        let (content, format, extraction_method, page_range, structured_rows_detected) =
            match extension.as_str() {
                "pdf" => {
                    let pages = extract_text_by_pages(&path)
                        .map_err(|error| KernelError::Execution(error.to_string()))?;
                    if let Some(range) = requested_page_range {
                        let start_index = range.start_page.saturating_sub(1);
                        let end_page = range.end_page.min(pages.len());
                        if start_index >= pages.len() || range.start_page > end_page {
                            return Err(KernelError::Validation(format!(
                                "{} does not have {}",
                                path.display(),
                                range.render()
                            )));
                        }
                        (
                            pages[start_index..end_page].join("\n\n"),
                            "pdf".to_string(),
                            "pdf_extract_by_page".to_string(),
                            Some(DocumentPageRange {
                                start_page: range.start_page,
                                end_page,
                            }),
                            false,
                        )
                    } else {
                        (
                            extract_text(&path)
                                .map_err(|error| KernelError::Execution(error.to_string()))?,
                            "pdf".to_string(),
                            "pdf_extract_full".to_string(),
                            None,
                            false,
                        )
                    }
                }
                "md" | "txt" | "rs" | "toml" | "json" | "yaml" | "yml" => {
                    if requested_page_range.is_some() {
                        return Err(KernelError::Validation(format!(
                            "page-specific extraction is only supported for PDFs right now: {}",
                            path.display()
                        )));
                    }
                    let text_read = Self::read_file(&path, None, None, None)?;
                    (
                        text_read.content,
                        extension,
                        "text_read".to_string(),
                        None,
                        false,
                    )
                }
                _ => {
                    return Err(KernelError::Unsupported(format!(
                        "document extraction is not supported for {}",
                        path.display()
                    )));
                }
            };

        let limit = max_chars.unwrap_or(DEFAULT_MAX_READ_BYTES);
        let mut truncated = false;
        let content = if content.chars().count() > limit {
            truncated = true;
            content.chars().take(limit).collect::<String>()
        } else {
            content
        };
        Ok((
            content,
            truncated,
            format,
            extraction_method,
            page_range,
            structured_rows_detected,
        ))
    }
}

fn path_matches_pattern(name: &str, path_text: &str, pattern: &str) -> bool {
    let lowered = pattern.to_lowercase();
    if lowered.contains('*') || lowered.contains('?') {
        wildcard_matches(name, &lowered) || wildcard_matches(path_text, &lowered)
    } else {
        name.contains(&lowered) || path_text.contains(&lowered)
    }
}

fn wildcard_matches(text: &str, pattern: &str) -> bool {
    let text = text.as_bytes();
    let pattern = pattern.as_bytes();
    let (mut text_ix, mut pattern_ix) = (0usize, 0usize);
    let mut star_ix = None;
    let mut match_ix = 0usize;

    while text_ix < text.len() {
        if pattern_ix < pattern.len()
            && (pattern[pattern_ix] == b'?' || pattern[pattern_ix] == text[text_ix])
        {
            text_ix += 1;
            pattern_ix += 1;
        } else if pattern_ix < pattern.len() && pattern[pattern_ix] == b'*' {
            star_ix = Some(pattern_ix);
            match_ix = text_ix;
            pattern_ix += 1;
        } else if let Some(star_pos) = star_ix {
            pattern_ix = star_pos + 1;
            match_ix += 1;
            text_ix = match_ix;
        } else {
            return false;
        }
    }

    while pattern_ix < pattern.len() && pattern[pattern_ix] == b'*' {
        pattern_ix += 1;
    }

    pattern_ix == pattern.len()
}
