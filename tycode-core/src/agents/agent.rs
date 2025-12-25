use std::sync::Arc;

use crate::{agents::tool_type::ToolType, ai::types::Message, steering::Builtin};

pub trait Agent: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn core_prompt(&self) -> &'static str;
    fn requested_builtins(&self) -> &'static [Builtin];
    fn available_tools(&self) -> Vec<ToolType>;

    fn requires_tool_use(&self) -> bool {
        false
    }
}

pub struct ActiveAgent {
    pub agent: Arc<dyn Agent>,
    pub conversation: Vec<Message>,
    pub completion_result: Option<String>,
}

impl ActiveAgent {
    pub fn new(agent: Arc<dyn Agent>) -> Self {
        Self {
            agent,
            conversation: Vec::new(),
            completion_result: None,
        }
    }
}
