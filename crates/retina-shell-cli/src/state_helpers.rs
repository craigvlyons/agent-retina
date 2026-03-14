use super::*;

impl CliShell {
    pub(crate) fn inspect_path_state(path: &Path, include_content: bool) -> Result<PathState> {
        if !path.exists() {
            return Ok(PathState {
                path: path.to_path_buf(),
                exists: false,
                size: None,
                modified_at: None,
                content_hash: None,
            });
        }

        let metadata = fs::metadata(path)?;
        let modified_at = metadata.modified().ok().map(DateTime::<Utc>::from);
        let content_hash = if include_content && metadata.is_file() {
            let bytes = fs::read(path)?;
            Some(blake3::hash(&bytes).to_hex().to_string())
        } else {
            None
        };

        Ok(PathState {
            path: path.to_path_buf(),
            exists: true,
            size: Some(metadata.len()),
            modified_at,
            content_hash,
        })
    }

    pub(crate) fn cwd_hash(path: &Path) -> String {
        blake3::hash(path.display().to_string().as_bytes())
            .to_hex()
            .to_string()
    }

    pub(crate) fn resolve_path(path: &Path) -> Result<PathBuf> {
        if let Some(expanded) = expand_homeish_path(path) {
            return Ok(expanded);
        }
        if path.is_absolute() {
            Ok(path.to_path_buf())
        } else {
            Ok(std::env::current_dir()?.join(path))
        }
    }
}

pub(crate) fn path_fingerprint(path_state: &PathState) -> String {
    format!(
        "{}:{}:{:?}:{:?}",
        path_state.exists,
        path_state.size.unwrap_or_default(),
        path_state.modified_at,
        path_state.content_hash
    )
}

pub(crate) fn command_fingerprint(result: &CommandResult) -> String {
    let mut hasher = Hasher::new();
    hasher.update(result.command.as_bytes());
    hasher.update(result.stdout.as_bytes());
    hasher.update(result.stderr.as_bytes());
    hasher.update(&result.exit_code.unwrap_or_default().to_le_bytes());
    hasher.finalize().to_hex().to_string()
}

pub(crate) fn expand_homeish_path(path: &Path) -> Option<PathBuf> {
    let raw = path.to_str()?;
    if raw == "~" {
        return dirs::home_dir();
    }
    if let Some(stripped) = raw.strip_prefix("~/") {
        return dirs::home_dir().map(|home| home.join(stripped));
    }

    let first = path
        .components()
        .next()?
        .as_os_str()
        .to_str()?
        .to_lowercase();
    let base = match first.as_str() {
        "desktop" => dirs::home_dir().map(|home| home.join("Desktop")),
        "documents" => dirs::home_dir().map(|home| home.join("Documents")),
        "downloads" => dirs::home_dir().map(|home| home.join("Downloads")),
        _ => None,
    }?;

    let remainder = path.iter().skip(1).collect::<PathBuf>();
    Some(if remainder.as_os_str().is_empty() {
        base
    } else {
        base.join(remainder)
    })
}

pub(crate) fn prefers_document_extraction(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .map(|value| value.eq_ignore_ascii_case("pdf"))
        .unwrap_or(false)
}

pub(crate) fn normalize_requested_page_range(
    page_start: Option<usize>,
    page_end: Option<usize>,
) -> Result<Option<DocumentPageRange>> {
    match (page_start, page_end) {
        (None, None) => Ok(None),
        (Some(start_page), None) => {
            if start_page == 0 {
                return Err(KernelError::Validation(
                    "page numbers start at 1".to_string(),
                ));
            }
            Ok(Some(DocumentPageRange {
                start_page,
                end_page: start_page,
            }))
        }
        (None, Some(end_page)) => {
            if end_page == 0 {
                return Err(KernelError::Validation(
                    "page numbers start at 1".to_string(),
                ));
            }
            Ok(Some(DocumentPageRange {
                start_page: end_page,
                end_page,
            }))
        }
        (Some(start_page), Some(end_page)) => {
            if start_page == 0 || end_page == 0 {
                return Err(KernelError::Validation(
                    "page numbers start at 1".to_string(),
                ));
            }
            if end_page < start_page {
                return Err(KernelError::Validation(format!(
                    "invalid page range {}-{}",
                    start_page, end_page
                )));
            }
            Ok(Some(DocumentPageRange {
                start_page,
                end_page,
            }))
        }
    }
}

pub(crate) fn looks_binary(bytes: &[u8]) -> bool {
    bytes.contains(&0)
}
