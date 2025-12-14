use crate::agents::{
    agent::Agent, auto_pr::AutoPrAgent, code_review::CodeReviewAgent, coder::CoderAgent,
    coordinator::CoordinatorAgent, file_writer::FileWriterAgent, one_shot::OneShotAgent,
    recon::ReconAgent,
};

/// Information about an available agent
#[derive(Clone, Debug)]
pub struct AgentInfo {
    pub name: String,
    pub description: String,
}

/// Registry of available agents
pub struct AgentCatalog;

impl AgentCatalog {
    /// Single source of truth for all available agents
    fn all_agents() -> Vec<Box<dyn Agent>> {
        vec![
            Box::new(CoordinatorAgent),
            Box::new(OneShotAgent),
            Box::new(ReconAgent),
            Box::new(CoderAgent),
            Box::new(CodeReviewAgent),
            Box::new(FileWriterAgent),
            Box::new(AutoPrAgent),
        ]
    }

    /// Get all available agents with their descriptions - names derived from trait
    pub fn list_agents() -> Vec<AgentInfo> {
        Self::all_agents()
            .iter()
            .map(|agent| AgentInfo {
                name: agent.name().to_string(),
                description: agent.description().to_string(),
            })
            .collect()
    }

    /// Create an agent instance by name
    pub fn create_agent(name: &str) -> Option<Box<dyn Agent>> {
        Self::all_agents()
            .into_iter()
            .find(|agent| agent.name() == name)
    }

    /// Get agent descriptions as a formatted string for tool schemas
    pub fn get_agent_descriptions() -> String {
        let agents = Self::list_agents();
        agents
            .iter()
            .map(|a| format!("'{}': {}", a.name, a.description))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Get valid agent names for enum schema
    pub fn get_agent_names() -> Vec<String> {
        Self::list_agents().iter().map(|a| a.name.clone()).collect()
    }
}
