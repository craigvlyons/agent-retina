use super::*;

const MAX_EDIT_FILE_SIZE: u64 = 1024 * 1024 * 1024;

pub(crate) struct MutationOutcome {
    pub(crate) path: PathBuf,
    pub(crate) mutation_kind: FileMutationKind,
    pub(crate) bytes_written: usize,
    pub(crate) created: bool,
    pub(crate) overwritten: bool,
    pub(crate) appended: bool,
    pub(crate) original_hash: Option<String>,
    pub(crate) updated_hash: String,
    pub(crate) changed_line_count: usize,
    pub(crate) patch_summary: Option<PatchSummary>,
    pub(crate) preview_excerpt: Option<String>,
    pub(crate) artifact: FileArtifactPayload,
}

impl CliShell {
    pub(crate) fn write_file(
        &self,
        path: &Path,
        content: &str,
        overwrite: bool,
    ) -> Result<MutationOutcome> {
        let path = Self::resolve_path(path)?;
        Self::reject_notebook_text_mutation(&path, "write_file")?;
        let existed_before = path.exists();
        if existed_before && !overwrite {
            return Err(KernelError::Validation(format!(
                "refusing to overwrite existing file {} without overwrite=true",
                path.display()
            )));
        }

        let prior_read = self.require_full_read_for_existing(&path)?;
        let original_content = prior_read
            .as_ref()
            .map(|state| state.content.clone())
            .unwrap_or_default();
        let original_hash = prior_read
            .as_ref()
            .and_then(|state| state.version.content_hash.clone());
        if let Some(state) = prior_read.as_ref() {
            let _ = self.ensure_read_state_is_fresh(&path, state)?;
        }

        let (bytes_written, updated_version, final_content) =
            self.write_text_atomically(&path, content, prior_read.as_ref())?;
        self.maybe_remember_text_read(&path, &final_content, false)?;
        let updated_hash = Self::required_content_hash(
            &path,
            &updated_version,
            "write_file completed but the refreshed file hash was missing",
        )?;

        Ok(MutationOutcome {
            path,
            mutation_kind: if existed_before {
                FileMutationKind::Overwrite
            } else {
                FileMutationKind::Create
            },
            bytes_written,
            created: !existed_before,
            overwritten: existed_before,
            appended: false,
            original_hash,
            updated_hash,
            changed_line_count: Self::changed_line_count(&original_content, &final_content),
            patch_summary: None,
            preview_excerpt: Some(Self::preview_text_fragment(&final_content)),
            artifact: FileArtifactPayload {
                original_content: existed_before.then_some(original_content),
                final_content,
            },
        })
    }

    pub(crate) fn append_file(&self, path: &Path, content: &str) -> Result<MutationOutcome> {
        let path = Self::resolve_path(path)?;
        Self::reject_notebook_text_mutation(&path, "append_file")?;
        let existed_before = path.exists();
        let prior_read = self.require_full_read_for_existing(&path)?;
        let original_content = prior_read
            .as_ref()
            .map(|state| state.content.clone())
            .unwrap_or_default();
        let original_hash = prior_read
            .as_ref()
            .and_then(|state| state.version.content_hash.clone());
        if let Some(state) = prior_read.as_ref() {
            let _ = self.ensure_read_state_is_fresh(&path, state)?;
        }

        let next_content = format!("{original_content}{content}");
        let (bytes_written, updated_version, final_content) =
            self.write_text_atomically(&path, &next_content, prior_read.as_ref())?;
        self.maybe_remember_text_read(&path, &final_content, false)?;
        let updated_hash = Self::required_content_hash(
            &path,
            &updated_version,
            "append_file completed but the refreshed file hash was missing",
        )?;

        Ok(MutationOutcome {
            path,
            mutation_kind: FileMutationKind::Append,
            bytes_written,
            created: !existed_before,
            overwritten: false,
            appended: true,
            original_hash,
            updated_hash,
            changed_line_count: Self::changed_line_count(&original_content, &final_content),
            patch_summary: None,
            preview_excerpt: Some(Self::preview_text_fragment(&final_content)),
            artifact: FileArtifactPayload {
                original_content: existed_before.then_some(original_content),
                final_content,
            },
        })
    }

    pub(crate) fn edit_file(
        &self,
        path: &Path,
        old_string: &str,
        new_string: &str,
        replace_all: bool,
    ) -> Result<MutationOutcome> {
        let path = Self::resolve_path(path)?;
        Self::reject_notebook_text_mutation(&path, "edit_file")?;
        if old_string.is_empty() {
            return Err(KernelError::Validation(
                "edit_file requires a non-empty old_string".to_string(),
            ));
        }
        if old_string == new_string {
            return Err(KernelError::Validation(
                "edit_file has nothing to change because old_string and new_string are identical"
                    .to_string(),
            ));
        }
        if let Ok(metadata) = fs::metadata(&path) {
            if metadata.len() > MAX_EDIT_FILE_SIZE {
                return Err(KernelError::Validation(format!(
                    "edit_file refuses to edit {} because it is too large ({} bytes)",
                    path.display(),
                    metadata.len()
                )));
            }
        }
        let Some(prior_read) = self.require_full_read_for_existing(&path)? else {
            let suggestion = Self::suggest_similar_path(&path);
            return Err(KernelError::Validation(format!(
                "edit_file requires an existing file: {}{}",
                path.display(),
                suggestion
                    .map(|candidate| format!("; did you mean {}?", candidate.display()))
                    .unwrap_or_default()
            )));
        };
        let _ = self.ensure_read_state_is_fresh(&path, &prior_read)?;

        let current_content = prior_read.content.clone();
        let resolved_old_string = Self::resolve_edit_old_string(&current_content, old_string);
        let exact_old_string = resolved_old_string.as_deref().unwrap_or(old_string);
        let matches = current_content.matches(exact_old_string).count();
        if matches == 0 {
            if prior_read.normalized_content.matches(old_string).count() > 0 {
                return Err(KernelError::Validation(format!(
                    "edit_file found the target only after line-ending normalization in {}; read the exact file contents and retry with an exact old_string",
                    path.display()
                )));
            }
            return Err(KernelError::Validation(format!(
                "edit_file could not find the requested old_string in {}",
                path.display()
            )));
        }
        if matches > 1 && !replace_all {
            return Err(KernelError::Validation(format!(
                "edit_file found {} matches for old_string in {}; set replace_all=true or make the match more specific",
                matches,
                path.display()
            )));
        }

        let updated_content = if replace_all {
            current_content.replace(exact_old_string, new_string)
        } else {
            current_content.replacen(exact_old_string, new_string, 1)
        };
        let replaced_occurrences = if replace_all { matches } else { 1 };
        let (bytes_written, updated_version, final_content) =
            self.write_text_atomically(&path, &updated_content, Some(&prior_read))?;
        self.maybe_remember_text_read(&path, &final_content, false)?;
        let updated_hash = Self::required_content_hash(
            &path,
            &updated_version,
            "edit_file completed but the refreshed file hash was missing",
        )?;

        Ok(MutationOutcome {
            path,
            mutation_kind: FileMutationKind::ExactEdit,
            bytes_written,
            created: false,
            overwritten: true,
            appended: false,
            original_hash: prior_read.version.content_hash.clone(),
            updated_hash,
            changed_line_count: Self::changed_line_count(&current_content, &final_content),
            patch_summary: Some(PatchSummary {
                matched_occurrences: matches,
                replaced_occurrences,
                old_preview: Self::preview_text_fragment(exact_old_string),
                new_preview: Self::preview_text_fragment(new_string),
            }),
            preview_excerpt: Some(Self::preview_text_fragment(&final_content)),
            artifact: FileArtifactPayload {
                original_content: Some(current_content),
                final_content,
            },
        })
    }

    pub(crate) fn reject_notebook_text_mutation(path: &Path, action: &str) -> Result<()> {
        if path
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| value.eq_ignore_ascii_case("ipynb"))
            .unwrap_or(false)
        {
            return Err(KernelError::Unsupported(format!(
                "{action} does not support .ipynb files; use edit_notebook instead"
            )));
        }
        Ok(())
    }

    pub(crate) fn changed_line_count(before: &str, after: &str) -> usize {
        let left = before.lines().collect::<Vec<_>>();
        let right = after.lines().collect::<Vec<_>>();
        let max_len = left.len().max(right.len());
        (0..max_len)
            .filter(|index| left.get(*index) != right.get(*index))
            .count()
    }

    pub(crate) fn preview_text_fragment(value: &str) -> String {
        let preview = value.chars().take(120).collect::<String>();
        if value.chars().count() > 120 {
            format!("{preview}...")
        } else {
            preview
        }
    }

    fn resolve_edit_old_string<'a>(content: &'a str, requested: &str) -> Option<String> {
        if content.contains(requested) {
            return Some(requested.to_string());
        }

        let normalized_requested = Self::normalize_quotes(requested);
        let mut matches = Vec::new();
        let chars = content.char_indices().collect::<Vec<_>>();
        for start_index in 0..chars.len() {
            for end_index in start_index + 1..=chars.len() {
                let start = chars[start_index].0;
                let end = if end_index == chars.len() {
                    content.len()
                } else {
                    chars[end_index].0
                };
                let slice = &content[start..end];
                if Self::normalize_quotes(slice) == normalized_requested {
                    matches.push(slice.to_string());
                    if matches.len() > 1 {
                        return None;
                    }
                }
            }
        }
        matches.into_iter().next()
    }

    fn normalize_quotes(value: &str) -> String {
        value
            .chars()
            .map(|ch| match ch {
                '\u{2018}' | '\u{2019}' => '\'',
                '\u{201C}' | '\u{201D}' => '"',
                _ => ch,
            })
            .collect()
    }

    pub(crate) fn suggest_similar_path(path: &Path) -> Option<PathBuf> {
        let parent = path.parent()?;
        let requested_name = path.file_name()?.to_str()?.to_lowercase();
        let requested_stem = path.file_stem()?.to_str()?.to_lowercase();

        fs::read_dir(parent)
            .ok()?
            .filter_map(|entry| entry.ok().map(|item| item.path()))
            .find(|candidate| {
                let Some(name) = candidate.file_name().and_then(|value| value.to_str()) else {
                    return false;
                };
                let lower = name.to_lowercase();
                if lower == requested_name {
                    return true;
                }
                candidate
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .map(|stem| stem.eq_ignore_ascii_case(&requested_stem))
                    .unwrap_or(false)
                    || lower.contains(&requested_stem)
            })
    }

    pub(crate) fn required_content_hash(
        path: &Path,
        version: &crate::read_state::FileVersionSnapshot,
        context: &str,
    ) -> Result<String> {
        version
            .content_hash
            .clone()
            .ok_or_else(|| KernelError::Execution(format!("{} ({})", context, path.display())))
    }

    pub(crate) fn notebook_cell_index(
        cells: &[serde_json::Value],
        locator: Option<&str>,
    ) -> Result<usize> {
        match locator {
            Some(value) => {
                if let Ok(index) = value.parse::<usize>() {
                    if index < cells.len() {
                        return Ok(index);
                    }
                }
                cells
                    .iter()
                    .position(|cell| {
                        cell.get("id").and_then(serde_json::Value::as_str) == Some(value)
                    })
                    .ok_or_else(|| {
                        KernelError::Validation(format!(
                            "could not locate notebook cell '{}'",
                            value
                        ))
                    })
            }
            None if cells.len() == 1 => Ok(0),
            None => Err(KernelError::Validation(
                "notebook cell selection is ambiguous; provide cell_id".to_string(),
            )),
        }
    }
}
