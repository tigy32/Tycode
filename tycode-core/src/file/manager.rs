use crate::file::access::FileAccessManager;
use crate::security::manager::SecurityManager;
use crate::security::types::{SecurityConfig, ToolPermission};
use crate::security::RiskLevel;
use crate::tools::r#trait::FileModification;
use anyhow::{Context, Result};

/// Manages file modifications with security enforcement and future review capabilities
pub struct FileModificationManager {
    file_access: FileAccessManager,
    security_manager: SecurityManager,
}

impl FileModificationManager {
    pub fn new(file_access: FileAccessManager, security_config: SecurityConfig) -> Self {
        Self {
            file_access,
            security_manager: SecurityManager::new(security_config),
        }
    }

    /// Apply a file modification after security checks
    pub async fn apply_modification(&self, modification: FileModification) -> Result<()> {
        // Check security permission
        let risk = RiskLevel::LowRisk;
        let permission = self.security_manager.check_permission(risk);
        if permission != ToolPermission::Allowed {
            anyhow::bail!(
                "File modification denied by security policy: {} (risk: {:?}, mode: {:?})",
                modification.path.display(),
                risk,
                self.security_manager.get_mode()
            );
        }

        // Apply the modification
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
