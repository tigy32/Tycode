use crate::file::access::FileAccessManager;
use crate::tools::r#trait::FileModification;
use anyhow::{Context, Result};

/// Manages file modifications with security enforcement and future review capabilities
pub struct FileModificationManager {
    file_access: FileAccessManager,
}

impl FileModificationManager {
    pub fn new(file_access: FileAccessManager) -> Self {
        Self { file_access }
    }

    pub async fn apply_modification(&self, modification: FileModification) -> Result<()> {
        match modification.operation {
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

                tracing::info!("Created file: {}", modification.path.display());
            }
            crate::tools::r#trait::FileOperation::Update => {
                let content = modification
                    .new_content
                    .ok_or_else(|| anyhow::anyhow!("Update operation requires new_content"))?;

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

                tracing::info!("Updated file: {}", modification.path.display());
            }
            crate::tools::r#trait::FileOperation::Delete => {
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

                tracing::info!("Deleted file: {}", modification.path.display());
            }
        }

        Ok(())
    }
}
