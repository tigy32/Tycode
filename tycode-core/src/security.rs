use anyhow::bail;
use serde::{Deserialize, Serialize};

use crate::{settings::SettingsManager, tools::r#trait::ValidatedToolCall};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
#[derive()]
pub enum SecurityMode {
    ReadOnly,
    #[default]
    Auto,
    All,
}

/// Permission result for a tool operation
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolPermission {
    Allowed,
    Denied,
}

/// Risk level for operations
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    /// No risk - read-only operations
    ReadOnly,
    /// Low risk - safe modifications within workspace
    LowRisk,
    /// High risk - command execution, sensitive paths, or outside workspace
    HighRisk,
}

/// Configuration for security policies
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SecurityConfig {
    /// Current security mode
    #[serde(default)]
    pub mode: SecurityMode,
}

pub fn evaluate<'a>(
    settings: &SettingsManager,
    calls: impl Iterator<Item = &'a ValidatedToolCall>,
) -> anyhow::Result<()> {
    let mode = settings.get_mode();
    for call in calls {
        match call {
            ValidatedToolCall::RunCommand { .. } | ValidatedToolCall::McpCall { .. } => {
                if mode < SecurityMode::All {
                    bail!("Security mode {mode:?} does not allow command execution. `/security set all` to allow");
                }
            }
            ValidatedToolCall::FileModification(_) => {
                if mode < SecurityMode::Auto {
                    bail!("Security mode {mode:?} does not allow file modification. `/security set auto` to allow");
                }
            }
            ValidatedToolCall::NoOp { .. }
            | ValidatedToolCall::PromptUser { .. }
            | ValidatedToolCall::PushAgent { .. }
            | ValidatedToolCall::PopAgent { .. }
            | ValidatedToolCall::SetTrackedFiles { .. }
            | ValidatedToolCall::PerformTaskListOp(_)
            | ValidatedToolCall::SearchTypes { .. }
            | ValidatedToolCall::GetTypeDocs { .. }
            | ValidatedToolCall::Error(_) => (),
        }
    }
    Ok(())
}
