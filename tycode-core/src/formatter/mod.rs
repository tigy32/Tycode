mod compact;
mod verbose;

use crate::ai::TokenUsage;
use crate::chat::events::{ToolExecutionResult, ToolRequest};
use crate::chat::ModelInfo;
use crate::tools::tasks::TaskList;

pub use compact::CompactFormatter;
pub use verbose::VerboseFormatter;

/// Trait for formatting and displaying events in the terminal
pub trait EventFormatter: Send + Sync {
    fn print_system(&mut self, msg: &str);

    fn print_ai(
        &mut self,
        msg: &str,
        agent: &str,
        model_info: &Option<ModelInfo>,
        token_usage: &Option<TokenUsage>,
    );

    fn print_warning(&mut self, msg: &str);

    fn print_error(&mut self, msg: &str);

    fn print_retry_attempt(&mut self, attempt: u32, max_retries: u32, error: &str);

    fn print_tool_request(&mut self, tool_request: &ToolRequest);

    fn print_tool_result(
        &mut self,
        name: &str,
        success: bool,
        result: ToolExecutionResult,
        verbose: bool,
    );

    fn print_thinking(&mut self);

    fn print_task_update(&mut self, task_list: &TaskList);

    fn on_typing_status_changed(&mut self, _typing: bool) {}

    fn clone_box(&self) -> Box<dyn EventFormatter>;
}

impl Clone for Box<dyn EventFormatter> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}
