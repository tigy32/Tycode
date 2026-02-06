use std::collections::HashSet;
use std::pin::Pin;

use tokio_stream::Stream;

use crate::ai::tweaks::ModelTweaks;
use crate::ai::{error::AiError, model::Model, types::*};

#[async_trait::async_trait]
pub trait AiProvider: Send + Sync {
    fn name(&self) -> &'static str;

    fn supported_models(&self) -> HashSet<Model>;

    async fn converse(&self, request: ConversationRequest)
        -> Result<ConversationResponse, AiError>;

    fn get_cost(&self, model: &Model) -> Cost;

    async fn converse_stream(
        &self,
        request: ConversationRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent, AiError>> + Send>>, AiError> {
        let response = self.converse(request).await?;
        Ok(Box::pin(tokio_stream::once(Ok(
            StreamEvent::MessageComplete { response },
        ))))
    }

    fn tweaks(&self) -> ModelTweaks {
        ModelTweaks::default()
    }
}
