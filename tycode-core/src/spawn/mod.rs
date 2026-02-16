//! Spawn module for agent lifecycle management.
//!
//! Owns the agent stack (Vec<ActiveAgent>) and all lifecycle operations.
//! Single source of truth for agent hierarchy.

use std::collections::HashSet;
use std::sync::{Arc, RwLock};

use crate::agents::agent::ActiveAgent;
use crate::agents::catalog::AgentCatalog;
use crate::module::{ContextComponent, Module, PromptComponent};
use crate::tools::ask_user_question::AskUserQuestion;
use crate::tools::r#trait::SharedTool;
use crate::Agent;

pub mod complete_task;
pub mod spawn_agent;

pub use complete_task::CompleteTask;
pub use spawn_agent::SpawnAgent;

pub struct SpawnModule {
    catalog: Arc<AgentCatalog>,
    agents: Arc<RwLock<Vec<ActiveAgent>>>,
}

impl SpawnModule {
    pub fn new(catalog: Arc<AgentCatalog>, initial_agent: Arc<dyn Agent>) -> Self {
        Self {
            catalog,
            agents: Arc::new(RwLock::new(vec![ActiveAgent::new(initial_agent)])),
        }
    }

    /// Get the current agent name (top of stack)
    pub fn current_agent_name(&self) -> Option<String> {
        self.agents
            .read()
            .ok()?
            .last()
            .map(|a| a.agent.name().to_string())
    }

    /// Execute closure with read access to current agent
    pub fn with_current_agent<F, R>(&self, f: F) -> Option<R>
    where
        F: FnOnce(&ActiveAgent) -> R,
    {
        let agents = self.agents.read().ok()?;
        agents.last().map(f)
    }

    /// Execute closure with mutable access to current agent
    pub fn with_current_agent_mut<F, R>(&self, f: F) -> Option<R>
    where
        F: FnOnce(&mut ActiveAgent) -> R,
    {
        let mut agents = self.agents.write().ok()?;
        agents.last_mut().map(f)
    }

    /// Execute closure with read access to root agent
    pub fn with_root_agent<F, R>(&self, f: F) -> Option<R>
    where
        F: FnOnce(&ActiveAgent) -> R,
    {
        let agents = self.agents.read().ok()?;
        agents.first().map(f)
    }

    /// Execute closure with mutable access to root agent
    pub fn with_root_agent_mut<F, R>(&self, f: F) -> Option<R>
    where
        F: FnOnce(&mut ActiveAgent) -> R,
    {
        let mut agents = self.agents.write().ok()?;
        agents.first_mut().map(f)
    }

    /// Push a new agent onto the stack
    pub fn push_agent(&self, agent: ActiveAgent) {
        if let Ok(mut agents) = self.agents.write() {
            agents.push(agent);
        }
    }

    /// Pop current agent if stack has > 1 agent (preserves root)
    pub fn pop_agent(&self) -> Option<ActiveAgent> {
        let mut agents = self.agents.write().ok()?;
        if agents.len() > 1 {
            agents.pop()
        } else {
            None
        }
    }

    /// Get stack depth
    pub fn stack_depth(&self) -> usize {
        self.agents.read().map(|a| a.len()).unwrap_or(0)
    }

    /// Clear stack and reset with new root agent
    pub fn reset_to_agent(&self, agent: Arc<dyn Agent>) {
        if let Ok(mut agents) = self.agents.write() {
            agents.clear();
            agents.push(ActiveAgent::new(agent));
        }
    }

    /// Get reference to catalog
    pub fn catalog(&self) -> &Arc<AgentCatalog> {
        &self.catalog
    }

    /// Execute closure with read access to all agents
    pub fn with_agents<F, R>(&self, f: F) -> Option<R>
    where
        F: FnOnce(&[ActiveAgent]) -> R,
    {
        let agents = self.agents.read().ok()?;
        Some(f(&agents))
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

    fn tools(&self) -> Vec<SharedTool> {
        let current = self.current_agent_name().unwrap_or_default();
        let allowed = allowed_agents_for(&current);

        let mut tools: Vec<SharedTool> = vec![Arc::new(CompleteTask), Arc::new(AskUserQuestion)];

        if !allowed.is_empty() {
            tools.push(Arc::new(SpawnAgent::new(
                self.catalog.clone(),
                allowed,
                current,
            )));
        }

        tools
    }
}
