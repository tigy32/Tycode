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
    catalog: Arc<AgentCatalog>,
    agent_stack: AgentStack,
}

impl SpawnModule {
    pub fn new(catalog: Arc<AgentCatalog>, initial_agent: &str) -> Self {
        Self {
            catalog,
            agent_stack: Arc::new(RwLock::new(vec![initial_agent.to_string()])),
        }
    }

    /// Get the current agent type (top of stack)
    pub fn current_agent(&self) -> Option<String> {
        self.agent_stack.read().ok()?.last().cloned()
    }
}

/// Agent hierarchy for spawn permissions.
/// Lower level = higher privilege (can spawn more agents).
/// Agents can only spawn agents at levels below them.
///
/// Hierarchy:
///   tycode (L0) > coordinator (L1) > coder (L2) > leaves (L3)
///   Leaves: context, debugger, planner, review
fn agent_level(agent: &str) -> u8 {
    match agent {
        "tycode" => 0,
        "coordinator" => 1,
        "coder" => 2,
        // Leaf agents - cannot spawn anything
        "context" | "debugger" | "planner" | "review" => 3,
        // Unknown agents default to leaf (most restrictive)
        _ => 3,
    }
}

/// Returns the set of agents that can be spawned by the given agent.
/// Based on hierarchical chain: can only spawn agents at lower levels.
pub fn allowed_agents_for(agent: &str) -> HashSet<String> {
    let level = agent_level(agent);

    // Collect all agents at levels below this agent's level
    let all_agents = [
        ("coordinator", 1),
        ("coder", 2),
        ("context", 3),
        ("debugger", 3),
        ("planner", 3),
        ("review", 3),
    ];

    all_agents
        .into_iter()
        .filter(|(_, agent_level)| *agent_level > level)
        .map(|(name, _)| name.to_string())
        .collect()
}

impl Module for SpawnModule {
    fn prompt_components(&self) -> Vec<Arc<dyn PromptComponent>> {
        vec![]
    }

    fn context_components(&self) -> Vec<Arc<dyn ContextComponent>> {
        vec![]
    }

    fn tools(&self) -> Vec<Arc<dyn ToolExecutor>> {
        let current = self.current_agent().unwrap_or_default();
        let allowed = allowed_agents_for(&current);

        let mut tools: Vec<Arc<dyn ToolExecutor>> =
            vec![Arc::new(CompleteTask::new(self.agent_stack.clone()))];

        if !allowed.is_empty() {
            tools.push(Arc::new(SpawnAgent::new(
                self.catalog.clone(),
                allowed,
                self.agent_stack.clone(),
            )));
        }

        tools
    }
}
