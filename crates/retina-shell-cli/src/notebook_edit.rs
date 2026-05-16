use super::*;

impl CliShell {
    pub(crate) fn edit_notebook(
        &self,
        path: &Path,
        cell_id: Option<String>,
        new_source: &str,
        cell_type: Option<NotebookCellType>,
        edit_mode: NotebookEditMode,
    ) -> Result<crate::file_edit::MutationOutcome> {
        let path = Self::resolve_path(path)?;
        if !path
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| value.eq_ignore_ascii_case("ipynb"))
            .unwrap_or(false)
        {
            return Err(KernelError::Unsupported(format!(
                "edit_notebook requires a .ipynb file: {}",
                path.display()
            )));
        }
        if matches!(edit_mode, NotebookEditMode::Insert) && cell_type.is_none() {
            return Err(KernelError::Validation(
                "edit_notebook requires cell_type when edit_mode=insert".to_string(),
            ));
        }
        if !matches!(edit_mode, NotebookEditMode::Insert) && cell_id.is_none() {
            return Err(KernelError::Validation(
                "edit_notebook requires cell_id when edit_mode is replace or delete".to_string(),
            ));
        }

        let Some(prior_read) = self.require_full_read_for_existing(&path)? else {
            let suggestion = Self::suggest_similar_path(path.as_path());
            return Err(KernelError::Validation(format!(
                "edit_notebook requires an existing notebook: {}{}",
                path.display(),
                suggestion
                    .map(|candidate| format!("; did you mean {}?", candidate.display()))
                    .unwrap_or_default()
            )));
        };
        let _ = self.ensure_read_state_is_fresh(&path, &prior_read)?;

        let mut notebook: serde_json::Value = serde_json::from_str(&prior_read.content)
            .map_err(|error| KernelError::Execution(format!("invalid notebook JSON: {error}")))?;
        let cells = notebook
            .get_mut("cells")
            .and_then(serde_json::Value::as_array_mut)
            .ok_or_else(|| {
                KernelError::Execution("notebook is missing a cells array".to_string())
            })?;

        match edit_mode {
            NotebookEditMode::Replace => {
                let index = Self::notebook_cell_index(cells, cell_id.as_deref())?;
                let existing_type = cells[index]
                    .get("cell_type")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("code")
                    .to_string();
                let chosen_type = cell_type
                    .as_ref()
                    .map(Self::cell_type_name)
                    .unwrap_or(existing_type.as_str());
                if let Some(object) = cells[index].as_object_mut() {
                    object.insert(
                        "cell_type".to_string(),
                        serde_json::Value::String(chosen_type.to_string()),
                    );
                    object.insert("source".to_string(), Self::source_lines(new_source));
                    if chosen_type == "code" {
                        object
                            .entry("outputs".to_string())
                            .or_insert_with(|| serde_json::json!([]));
                        object
                            .entry("execution_count".to_string())
                            .or_insert(serde_json::Value::Null);
                    }
                }
            }
            NotebookEditMode::Insert => {
                let index = match cell_id.as_deref() {
                    Some(locator) => {
                        let resolved = Self::notebook_cell_index(cells, Some(locator))?;
                        (resolved + 1).min(cells.len())
                    }
                    None => cells.len(),
                };
                cells.insert(index, Self::new_notebook_cell(new_source, cell_type));
            }
            NotebookEditMode::Delete => {
                let index = Self::notebook_cell_index(cells, cell_id.as_deref())?;
                cells.remove(index);
            }
        }

        let updated_content = serde_json::to_string_pretty(&notebook)
            .map_err(|error| KernelError::Execution(error.to_string()))?;
        let (bytes_written, updated_version, final_content) =
            self.write_text_atomically(&path, &updated_content, Some(&prior_read))?;
        self.maybe_remember_text_read(&path, &final_content, false)?;
        let updated_hash = Self::required_content_hash(
            &path,
            &updated_version,
            "edit_notebook completed but the refreshed notebook hash was missing",
        )?;

        Ok(crate::file_edit::MutationOutcome {
            path,
            mutation_kind: match edit_mode {
                NotebookEditMode::Replace => FileMutationKind::NotebookReplace,
                NotebookEditMode::Insert => FileMutationKind::NotebookInsert,
                NotebookEditMode::Delete => FileMutationKind::NotebookDelete,
            },
            bytes_written,
            created: false,
            overwritten: true,
            appended: false,
            original_hash: prior_read.version.content_hash.clone(),
            updated_hash,
            changed_line_count: Self::changed_line_count(&prior_read.content, &final_content),
            patch_summary: None,
            preview_excerpt: Some(Self::preview_text_fragment(&final_content)),
            artifact: FileArtifactPayload {
                original_content: Some(prior_read.content),
                final_content,
            },
        })
    }

    fn new_notebook_cell(
        new_source: &str,
        cell_type: Option<NotebookCellType>,
    ) -> serde_json::Value {
        let cell_type_name = cell_type
            .as_ref()
            .map(Self::cell_type_name)
            .unwrap_or("code");
        let mut cell = serde_json::json!({
            "cell_type": cell_type_name,
            "id": format!("retina-{}", Utc::now().timestamp_millis()),
            "metadata": {},
            "source": Self::source_lines(new_source),
        });
        if cell_type_name == "code" {
            cell["outputs"] = serde_json::json!([]);
            cell["execution_count"] = serde_json::Value::Null;
        }
        cell
    }

    fn source_lines(new_source: &str) -> serde_json::Value {
        let lines = if new_source.is_empty() {
            Vec::new()
        } else {
            new_source
                .split_inclusive('\n')
                .map(|line| serde_json::Value::String(line.to_string()))
                .collect::<Vec<_>>()
        };
        serde_json::Value::Array(lines)
    }

    fn cell_type_name(cell_type: &NotebookCellType) -> &'static str {
        match cell_type {
            NotebookCellType::Code => "code",
            NotebookCellType::Markdown => "markdown",
        }
    }
}
