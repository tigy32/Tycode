use crate::file::access::FileAccessManager;
use crate::tools::r#trait::FileModification;
use anyhow::{Context, Result};

/// Statistics returned by file modification operations
#[derive(Debug, Clone)]
pub struct FileModificationStats {
    pub lines_added: u32,
    pub lines_removed: u32,
}

/// Manages file modifications with security enforcement and future review capabilities
pub struct FileModificationManager {
    file_access: FileAccessManager,
}

impl FileModificationManager {
    pub fn new(file_access: FileAccessManager) -> Self {
        Self { file_access }
    }

    pub async fn apply_modification(
        &self,
        modification: FileModification,
    ) -> Result<FileModificationStats> {
        let stats = match modification.operation {
            crate::tools::r#trait::FileOperation::Create => {
                let content = modification
                    .new_content
                    .ok_or_else(|| anyhow::anyhow!("Create operation requires new_content"))?;

                self.file_access
                    .write_file(
                        modification
                            .path
                            .to_str()
                            .ok_or_else(|| anyhow::anyhow!("Invalid file path"))?,
                        &content,
                    )
                    .await
                    .with_context(|| {
                        format!("Failed to create file: {}", modification.path.display())
                    })?;

                let lines_added = content.lines().count() as u32;
                let lines_removed = 0;

                tracing::info!(
                    "Created file: {} ({} lines)",
                    modification.path.display(),
                    lines_added
                );

                FileModificationStats {
                    lines_added,
                    lines_removed,
                }
            }
            crate::tools::r#trait::FileOperation::Update => {
                let content = modification
                    .new_content
                    .ok_or_else(|| anyhow::anyhow!("Update operation requires new_content"))?;

                let original_content = modification.original_content.as_deref().unwrap_or("");
                let lines_added = count_lines_added(original_content, &content);
                let lines_removed = count_lines_removed(original_content, &content);

                self.file_access
                    .write_file(
                        modification
                            .path
                            .to_str()
                            .ok_or_else(|| anyhow::anyhow!("Invalid file path"))?,
                        &content,
                    )
                    .await
                    .with_context(|| {
                        format!("Failed to update file: {}", modification.path.display())
                    })?;

                tracing::info!(
                    "Updated file: {} (+{} lines, -{} lines)",
                    modification.path.display(),
                    lines_added,
                    lines_removed
                );

                FileModificationStats {
                    lines_added,
                    lines_removed,
                }
            }
            crate::tools::r#trait::FileOperation::Delete => {
                // Read the original content before deleting to count lines
                let original_content = self
                    .file_access
                    .read_file(
                        modification
                            .path
                            .to_str()
                            .ok_or_else(|| anyhow::anyhow!("Invalid file path"))?,
                    )
                    .await
                    .with_context(|| {
                        format!(
                            "Failed to read file before deletion: {}",
                            modification.path.display()
                        )
                    })?;

                let lines_removed = original_content.lines().count() as u32;

                self.file_access
                    .delete_file(
                        modification
                            .path
                            .to_str()
                            .ok_or_else(|| anyhow::anyhow!("Invalid file path"))?,
                    )
                    .await
                    .with_context(|| {
                        format!("Failed to delete file: {}", modification.path.display())
                    })?;

                tracing::info!(
                    "Deleted file: {} ({} lines)",
                    modification.path.display(),
                    lines_removed
                );

                FileModificationStats {
                    lines_added: 0,
                    lines_removed,
                }
            }
        };

        Ok(stats)
    }
}

/// Counts the number of lines added when comparing original to new content
fn count_lines_added(original: &str, new: &str) -> u32 {
    let original_lines: std::collections::HashSet<&str> = original.lines().collect();
    let new_lines: std::collections::HashSet<&str> = new.lines().collect();

    // Lines that are in new but not in original are added
    new_lines.difference(&original_lines).count() as u32
}

/// Counts the number of lines removed when comparing original to new content
fn count_lines_removed(original: &str, new: &str) -> u32 {
    let original_lines: std::collections::HashSet<&str> = original.lines().collect();
    let new_lines: std::collections::HashSet<&str> = new.lines().collect();

    // Lines that are in original but not in new are removed
    original_lines.difference(&new_lines).count() as u32
}
