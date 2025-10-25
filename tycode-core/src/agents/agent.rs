use crate::{agents::tool_type::ToolType, ai::types::Message};

pub trait Agent: Send + Sync {
    fn name(&self) -> &str;
    fn system_prompt(&self) -> String;
    fn available_tools(&self) -> Vec<ToolType>;
}

pub struct ActiveAgent {
    pub agent: Box<dyn Agent>,
    pub conversation: Vec<Message>,
}

impl ActiveAgent {
    pub fn new(agent: Box<dyn Agent>) -> Self {
        Self {
            agent,
            conversation: Vec::new(),
        }
    }
}
