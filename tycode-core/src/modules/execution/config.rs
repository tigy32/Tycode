use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, JsonSchema)]
pub enum RunBuildTestOutputMode {
    #[default]
    ToolResponse,
    Context,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, JsonSchema)]
pub enum CommandExecutionMode {
    #[default]
    Direct,
    Bash,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(title = "Execution")]
pub struct ExecutionConfig {
    /// Where command output appears - in tool response or context section
    #[serde(default)]
    pub output_mode: RunBuildTestOutputMode,

    /// How commands are executed - direct exec or bash wrapper
    #[serde(default)]
    pub execution_mode: CommandExecutionMode,

    /// Maximum bytes of command output to include. Large outputs are compacted
    /// by keeping the first half and last half with a truncation marker.
    /// Defaults to 200KB.
    #[serde(default = "default_max_output_bytes")]
    pub max_output_bytes: Option<usize>,
}

fn default_max_output_bytes() -> Option<usize> {
    Some(200_000)
}

impl Default for ExecutionConfig {
    fn default() -> Self {
        Self {
            output_mode: RunBuildTestOutputMode::default(),
            execution_mode: CommandExecutionMode::default(),
            max_output_bytes: default_max_output_bytes(),
        }
    }
}
