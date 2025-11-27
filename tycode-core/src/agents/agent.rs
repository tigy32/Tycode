use crate::{agents::tool_type::ToolType, ai::types::Message};

pub trait Agent: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn system_prompt(&self) -> String;
    fn available_tools(&self) -> Vec<ToolType>;

    fn requires_tool_use(&self) -> bool {
        false
    }
}

pub struct ActiveAgent {
    pub agent: Box<dyn Agent>,
    pub conversation: Vec<Message>,
    pub completion_result: Option<String>,
}

impl ActiveAgent {
    pub fn new(agent: Box<dyn Agent>) -> Self {
        Self {
            agent,
            conversation: Vec::new(),
            completion_result: None,
        }
    }
}
