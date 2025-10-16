use crate::tools::r#trait::{ToolCategory, ToolExecutor, ToolRequest, ValidatedToolCall};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Serialize, Deserialize)]
struct CompleteTaskParams {
    /// Result of the task - summary, code outline, failure details, etc.
    result: String,
    /// Whether the task was successfully completed
    success: bool,
}

pub struct CompleteTask;

#[async_trait::async_trait(?Send)]
impl ToolExecutor for CompleteTask {
    fn name(&self) -> &'static str {
        "complete_task"
    }

    fn description(&self) -> &'static str {
        "Signal task completion (success or failure) and return control to parent agent. \
         FAIL a task when: \
         • Required resources/files don't exist \
         • The task requirements are unclear or contradictory \
         • You encounter errors you cannot resolve \
         • The requested change would break existing functionality \
         • You lack necessary permissions or access \
         SUCCEED when: \
         • All requested changes are implemented \
         • The task objectives are met \
         NOTE: Sub-agents must use this with failure instead of spawning more agents when stuck. \
         Parent agents have more context to handle failures properly."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["result", "success"],
            "properties": {
                "result": {
                    "type": "string",
                    "description": "Result of the task - summary of what was accomplished, failure details, code outline, or any other output"
                },
                "success": {
                    "type": "boolean",
                    "description": "Whether the task completed successfully"
                }
            }
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::ControlFlow
    }

    async fn validate(&self, request: &ToolRequest) -> Result<ValidatedToolCall> {
        let params: CompleteTaskParams = serde_json::from_value(request.arguments.clone())?;

        Ok(ValidatedToolCall::PopAgent {
            success: params.success,
            result: params.result,
        })
    }
}
