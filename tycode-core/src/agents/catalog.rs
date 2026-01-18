use std::sync::Arc;

use crate::agents::agent::Agent;

/// Information about an available agent
#[derive(Clone, Debug)]
pub struct AgentInfo {
    pub name: String,
    pub description: String,
}

/// Registry of available agents
#[derive(Default)]
pub struct AgentCatalog {
    agents: Vec<Arc<dyn Agent>>,
}

impl AgentCatalog {
    /// Create a new empty agent catalog
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an agent in the catalog
    pub fn register_agent(&mut self, agent: Arc<dyn Agent>) {
        self.agents.push(agent);
    }

    /// Get all registered agents
    fn agents(&self) -> &[Arc<dyn Agent>] {
        &self.agents
    }

    /// Get all available agents with their descriptions - names derived from trait
    pub fn list_agents(&self) -> Vec<AgentInfo> {
        self.agents()
            .iter()
            .map(|agent| AgentInfo {
                name: agent.name().to_string(),
                description: agent.description().to_string(),
            })
            .collect()
    }

    /// Create an agent instance by name
    pub fn create_agent(&self, name: &str) -> Option<Arc<dyn Agent>> {
        self.agents()
            .iter()
            .find(|agent| agent.name() == name)
            .cloned()
    }

    /// Get agent descriptions as a formatted string for tool schemas
    pub fn get_agent_descriptions(&self) -> String {
        let agents = self.list_agents();
        agents
            .iter()
            .map(|a| format!("'{}': {}", a.name, a.description))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Get valid agent names for enum schema
    pub fn get_agent_names(&self) -> Vec<String> {
        self.list_agents().iter().map(|a| a.name.clone()).collect()
    }
}
