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
    /// Get all available agents with their descriptions
    pub fn list_agents() -> Vec<AgentInfo> {
        vec![
            AgentInfo {
                name: "coordinator".to_string(),
                description: "Coordinates task execution, breaking requests into steps and delegating to sub‑agents".to_string(),
            },
            AgentInfo {
                name: "one_shot".to_string(),
                description: "Handles complete coding tasks in a single, all-in-one workflow".to_string(),
            },
            AgentInfo {
                name: "recon".to_string(),
                description: "Explores files and summarizes information about project structure, existing components, and relevant file locations to aid planning".to_string(),
            },
            AgentInfo {
                name: "coder".to_string(),
                description: "Executes assigned coding tasks, applying patches and managing files".to_string(),
            },
            AgentInfo {
                name: "code_reviewer".to_string(),
                description: "Approves or rejects proposed code changes to ensure compliance with style mandates".to_string(),
            },
            AgentInfo {
                name: "file_writer".to_string(),
                description: "Specializes in file operations: reading, writing, and updating files".to_string(),
            },
            AgentInfo {
                name: "auto_pr".to_string(),
                description: "Autonomous agent for auto-PR feature, follows TDD workflow to resolve issues without user interaction".to_string(),
            },
        ]
    }

    /// Create an agent instance by name
    pub fn create_agent(name: &str) -> Option<Box<dyn Agent>> {
        match name {
            "coordinator" => Some(Box::new(CoordinatorAgent)),
            "one_shot" => Some(Box::new(OneShotAgent)),
            "recon" => Some(Box::new(ReconAgent)),
            "coder" => Some(Box::new(CoderAgent)),
            "code_reviewer" => Some(Box::new(CodeReviewAgent)),
            "file_writer" => Some(Box::new(FileWriterAgent)),
            "auto_pr" => Some(Box::new(AutoPrAgent)),
            _ => None,
        }
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
