use std::collections::HashSet;

use crate::ai::tweaks::ModelTweaks;
use crate::ai::{error::AiError, model::Model, types::*};

#[async_trait::async_trait]
pub trait AiProvider: Send + Sync {
    fn name(&self) -> &'static str;

    fn supported_models(&self) -> HashSet<Model>;

    async fn converse(&self, request: ConversationRequest)
        -> Result<ConversationResponse, AiError>;

    fn get_cost(&self, model: &Model) -> Cost;

    fn tweaks(&self) -> ModelTweaks {
        ModelTweaks::default()
    }
}
