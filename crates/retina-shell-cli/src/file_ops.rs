use super::*;
use crate::state_helpers::{
    looks_binary, normalize_requested_page_range, prefers_document_extraction,
};

const DEFAULT_MAX_STRUCTURED_ROWS: usize = 25;

pub(crate) struct StructuredDataIngestion {
    pub(crate) format: String,
    pub(crate) headers: Vec<String>,
    pub(crate) rows: Vec<StructuredDataRow>,
    pub(crate) total_rows: usize,
    pub(crate) truncated: bool,
    pub(crate) extraction_method: String,
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
        max_results: usize,
    ) -> Result<Vec<PathBuf>> {
        let root = Self::resolve_path(root)?;
        let mut matches = Vec::new();
        Self::collect_matching_files(
            &root,
            &pattern.to_lowercase(),
            max_results.max(1),
            &mut matches,
        )?;
        matches.sort();
        Ok(matches)
    }

    fn collect_matching_files(
        root: &Path,
        pattern: &str,
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

            if name.contains(pattern) || path_text.contains(pattern) {
                matches.push(path.clone());
                if matches.len() >= max_results {
                    break;
                }
            }

            if metadata.is_dir() {
                Self::collect_matching_files(&path, pattern, max_results, matches)?;
                continue;
            }
        }
        Ok(())
    }

    pub(crate) fn search_text(
        root: &Path,
        query: &str,
        max_results: usize,
    ) -> Result<Vec<SearchMatch>> {
        let root = Self::resolve_path(root)?;
        let mut matches = Vec::new();
        Self::collect_text_matches(
            &root,
            &query.to_lowercase(),
            max_results.max(1),
            &mut matches,
        )?;
        Ok(matches)
    }

    fn collect_text_matches(
        root: &Path,
        query: &str,
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
                Self::collect_text_matches(&path, query, max_results, matches)?;
                continue;
            }
            if metadata.len() > 512 * 1024 {
                continue;
            }
            let bytes = fs::read(&path)?;
            if bytes.contains(&0) {
                continue;
            }
            let content = String::from_utf8_lossy(&bytes);
            for (index, line) in content.lines().enumerate() {
                if line.to_lowercase().contains(query) {
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

    pub(crate) fn read_file(path: &Path, max_bytes: Option<usize>) -> Result<(String, bool)> {
        let path = Self::resolve_path(path)?;
        if prefers_document_extraction(&path) {
            return Err(KernelError::Unsupported(format!(
                "read_file is not suitable for {}; use extract_document_text instead",
                path.display()
            )));
        }
        let mut file = fs::File::open(&path)?;
        let limit = max_bytes.unwrap_or(DEFAULT_MAX_READ_BYTES);
        let mut buffer = Vec::new();
        Read::by_ref(&mut file)
            .take((limit + 1) as u64)
            .read_to_end(&mut buffer)?;
        if looks_binary(&buffer) {
            return Err(KernelError::Unsupported(format!(
                "read_file only supports text-like files; {} appears to be binary",
                path.display()
            )));
        }
        let truncated = buffer.len() > limit;
        if truncated {
            buffer.truncate(limit);
        }
        Ok((String::from_utf8_lossy(&buffer).to_string(), truncated))
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
                    let (content, _) = Self::read_file(&path, None)?;
                    (content, extension, "text_read".to_string(), None, false)
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

    pub(crate) fn write_file(path: &Path, content: &str, overwrite: bool) -> Result<(usize, bool, bool)> {
        let path = Self::resolve_path(path)?;
        let existed_before = path.exists();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        if existed_before && !overwrite {
            return Err(KernelError::Validation(format!(
                "refusing to overwrite existing file {} without overwrite=true",
                path.display()
            )));
        }
        fs::write(&path, content)?;
        Ok((content.len(), !existed_before, existed_before))
    }

    pub(crate) fn append_file(path: &Path, content: &str) -> Result<(usize, bool)> {
        let path = Self::resolve_path(path)?;
        let existed_before = path.exists();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
        file.write_all(content.as_bytes())?;
        Ok((content.len(), !existed_before))
    }
}
