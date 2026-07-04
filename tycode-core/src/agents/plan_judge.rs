use crate::agents::agent::Agent;
use crate::spawn::complete_task::CompleteTask;
use crate::tools::ToolName;

const CORE_PROMPT: &str = r#"You are a plan judge. Your task presents several competing execution plans for the same coding task, each under a header of the form `### <label> [ok]`.

Pick the plan that demonstrates the best understanding of the task and codebase. Judge by:
1. **Repo grounding** - references real files/symbols with concrete evidence, not invented structure
2. **Shared-surface precision** - exact signatures/types for anything spanning multiple files
3. **Decomposition quality** - assignments are genuinely independent and complete
4. **Risk awareness** - edge cases and verification steps identified

Use complete_task with success=true and the result set to EXACTLY the label of the winning plan (for example: `plan:1:claude-fable`). Do not include any other text in the result."#;

pub struct PlanJudgeAgent;

impl PlanJudgeAgent {
    pub const NAME: &'static str = "plan_judge";
}

impl Agent for PlanJudgeAgent {
    fn name(&self) -> &str {
        Self::NAME
    }

    fn description(&self) -> &str {
        "Votes on the best of several competing execution plans; used by multi-model consensus"
    }

    fn core_prompt(&self) -> &'static str {
        CORE_PROMPT
    }

    fn available_tools(&self) -> Vec<ToolName> {
        vec![CompleteTask::tool_name()]
    }

    fn requires_tool_use(&self) -> bool {
        true
    }
}
