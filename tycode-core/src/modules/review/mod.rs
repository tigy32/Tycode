use std::sync::Arc;

use crate::module::{ContextComponent, Module, PromptComponent, SlashCommand};
use crate::tools::r#trait::SharedTool;

mod command;
mod diff;

pub struct ReviewModule;

impl Module for ReviewModule {
    fn prompt_components(&self) -> Vec<Arc<dyn PromptComponent>> {
        vec![]
    }

    fn context_components(&self) -> Vec<Arc<dyn ContextComponent>> {
        vec![]
    }

    fn tools(&self) -> Vec<SharedTool> {
        vec![]
    }

    fn slash_commands(&self) -> Vec<Arc<dyn SlashCommand>> {
        vec![Arc::new(command::ReviewSlashCommand)]
    }
}
