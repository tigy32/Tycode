//! Spawn module for agent lifecycle management.
//!
//! Owns both agent spawning (spawn_agent) and completion (complete_task).
//! Tracks the agent stack internally to manage agent hierarchy:
//! - Coordinator: can spawn ["coder", "recon"]
//! - Coder: can spawn ["recon"] (prevents coder→coder loops via self-spawn check)
//! - Recon: can spawn [] (leaf agent)

use std::collections::HashSet;
use std::sync::{Arc, RwLock};

use crate::agents::catalog::AgentCatalog;
use crate::module::{ContextComponent, Module, PromptComponent};
use crate::tools::r#trait::ToolExecutor;

pub mod complete_task;
pub mod spawn_agent;

pub use complete_task::CompleteTask;
pub use spawn_agent::SpawnAgent;

/// Shared agent stack for tracking current agent hierarchy.
/// The top of the stack is the currently executing agent.
pub type AgentStack = Arc<RwLock<Vec<String>>>;

pub struct SpawnModule {
    allowed_agents: HashSet<String>,
    catalog: Arc<AgentCatalog>,
    agent_stack: AgentStack,
}

impl SpawnModule {
    pub fn new(allowed_agents: Vec<&str>, catalog: Arc<AgentCatalog>, initial_agent: &str) -> Self {
        Self {
            allowed_agents: allowed_agents.into_iter().map(String::from).collect(),
            catalog,
            agent_stack: Arc::new(RwLock::new(vec![initial_agent.to_string()])),
        }
    }

    /// Get the current agent type (top of stack)
    pub fn current_agent(&self) -> Option<String> {
        self.agent_stack.read().ok()?.last().cloned()
    }
}

impl Module for SpawnModule {
    fn prompt_components(&self) -> Vec<Arc<dyn PromptComponent>> {
        vec![]
    }

    fn context_components(&self) -> Vec<Arc<dyn ContextComponent>> {
        vec![]
    }

    fn tools(&self) -> Vec<Arc<dyn ToolExecutor>> {
        let mut tools: Vec<Arc<dyn ToolExecutor>> = vec![
            // complete_task is always available
            Arc::new(CompleteTask::new(self.agent_stack.clone())),
        ];

        // spawn_agent only if there are allowed agents to spawn
        if !self.allowed_agents.is_empty() {
            tools.push(Arc::new(SpawnAgent::new(
                self.catalog.clone(),
                self.allowed_agents.clone(),
                self.agent_stack.clone(),
            )));
        }

        tools
    }
}
