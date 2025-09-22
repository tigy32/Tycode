use crate::ai::{error::AiError, model::Model, types::*};

#[async_trait::async_trait]
pub trait AiProvider: Send + Sync {
    fn name(&self) -> &'static str;

    fn supported_models(&self) -> Vec<Model>;

    async fn converse(&self, request: ConversationRequest)
        -> Result<ConversationResponse, AiError>;

    fn get_cost(&self, model: &Model) -> Cost;
}
