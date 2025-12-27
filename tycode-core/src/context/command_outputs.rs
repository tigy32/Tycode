use std::collections::VecDeque;
use std::sync::RwLock;

use crate::context::{ContextComponent, ContextComponentId};

pub const ID: ContextComponentId = ContextComponentId("command_outputs");

/// A stored command output with its command and result.
#[derive(Debug, Clone)]
pub struct CommandOutput {
    pub command: String,
    pub output: String,
    pub exit_code: Option<i32>,
}

/// Manages command output history and provides context rendering.
/// Stores a fixed-size buffer of recent command outputs.
pub struct CommandOutputsManager {
    outputs: RwLock<VecDeque<CommandOutput>>,
    max_outputs: usize,
}

impl CommandOutputsManager {
    pub fn new(max_outputs: usize) -> Self {
        Self {
            outputs: RwLock::new(VecDeque::with_capacity(max_outputs)),
            max_outputs,
        }
    }

    /// Add a command output to the buffer.
    /// If buffer is full, oldest output is removed.
    pub fn add_output(&self, command: String, output: String, exit_code: Option<i32>) {
        let mut outputs = self.outputs.write().unwrap();
        if outputs.len() >= self.max_outputs {
            outputs.pop_front();
        }
        outputs.push_back(CommandOutput {
            command,
            output,
            exit_code,
        });
    }

    /// Clear all stored outputs.
    pub fn clear(&self) {
        self.outputs.write().unwrap().clear();
    }

    /// Get the number of stored outputs.
    pub fn len(&self) -> usize {
        self.outputs.read().unwrap().len()
    }

    /// Check if buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.outputs.read().unwrap().is_empty()
    }
}

#[async_trait::async_trait(?Send)]
impl ContextComponent for CommandOutputsManager {
    fn id(&self) -> ContextComponentId {
        ID
    }

    async fn build_context_section(&self) -> Option<String> {
        let outputs = self.outputs.read().unwrap();
        if outputs.is_empty() {
            return None;
        }

        let mut result = String::from("Recent Command Outputs:\n");
        for output in outputs.iter() {
            result.push_str(&format!("\n$ {}\n", output.command));
            if let Some(code) = output.exit_code {
                result.push_str(&format!("Exit code: {}\n", code));
            }
            if !output.output.is_empty() {
                result.push_str(&output.output);
                if !output.output.ends_with('\n') {
                    result.push('\n');
                }
            }
        }
        Some(result)
    }
}
