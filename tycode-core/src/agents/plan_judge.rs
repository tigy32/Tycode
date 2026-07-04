use crate::agents::agent::Agent;
use crate::spawn::complete_task::CompleteTask;
use crate::tools::ToolName;

const CORE_PROMPT: &str = r#"You are one member of a planning panel seeking consensus on an execution plan. Your task presents the competing candidate plans, each under a header of the form `### <label>`. This is an elimination tournament: after each round, the plan voted worst is eliminated together with its author's seat on the panel. Consensus is reached when every remaining panelist approves the same plan.

Evaluate every candidate by:
1. **Repo grounding** - references real files/symbols with concrete evidence, not invented structure
2. **Shared-surface precision** - exact signatures/types for anything spanning multiple files
3. **Decomposition quality** - assignments are genuinely independent and complete
4. **Risk awareness** - edge cases and verification steps identified

Respond with complete_task (success=true). The result must contain two parts:

**Part 1 - your position, EXACTLY one of:**
- `APPROVE: <label>` on its own line (for example `APPROVE: plan:1:claude-fable`) if that candidate is correct as-is. It may be your own or another panelist's. Prefer approving over proposing trivial edits.
- A complete revised plan, replacing your own candidate. Merge the best elements across candidates: one plan may be right about a detail the others missed. Begin with a short critique naming what you took from which candidate and what you fixed, then the full plan, ending with the fenced ```json assignments block in the same shape the candidates use. Never begin a revision with the word APPROVE.

**Part 2 - always, as the final line:**
- `WORST: <label>` naming the weakest candidate."#;

pub struct PlanJudgeAgent;

impl PlanJudgeAgent {
    pub const NAME: &'static str = "plan_judge";
}

impl Agent for PlanJudgeAgent {
    fn name(&self) -> &str {
        Self::NAME
    }

    fn description(&self) -> &str {
        "Endorses or revises candidate execution plans until consensus; used by multi-model swarm planning"
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
