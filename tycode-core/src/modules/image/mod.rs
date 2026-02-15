use std::sync::{Arc, RwLock};

use anyhow::Result;
use serde::Deserialize;
use serde_json::json;

use crate::ai::provider::AiProvider;
use crate::ai::types::ImageGenerationRequest;
use crate::chat::events::{ToolExecutionResult, ToolRequest as ToolRequestEvent, ToolRequestType};
use crate::file::access::FileAccessManager;
use crate::module::{ContextComponent, Module, PromptComponent};
use crate::settings::SettingsManager;
use crate::tools::r#trait::{
    ContinuationPreference, ToolCallHandle, ToolCategory, ToolExecutor, ToolOutput, ToolRequest,
};
use crate::tools::ToolName;

pub mod config;

use config::Image;

pub type SharedProvider = Arc<RwLock<Arc<dyn AiProvider>>>;

pub struct ImageModule {
    provider: SharedProvider,
    file_access: Arc<FileAccessManager>,
    settings: Arc<SettingsManager>,
}

impl ImageModule {
    pub fn new(
        provider: SharedProvider,
        file_access: Arc<FileAccessManager>,
        settings: Arc<SettingsManager>,
    ) -> Self {
        Self {
            provider,
            file_access,
            settings,
        }
    }
}

impl Module for ImageModule {
    fn prompt_components(&self) -> Vec<Arc<dyn PromptComponent>> {
        vec![]
    }

    fn context_components(&self) -> Vec<Arc<dyn ContextComponent>> {
        vec![]
    }

    fn tools(&self) -> Vec<Arc<dyn ToolExecutor>> {
        let config: Image = self.settings.settings().get_module_config("image");
        if !config.enabled {
            return vec![];
        }
        let provider = self.provider.read().unwrap();
        if !provider.supports_image_generation() {
            return vec![];
        }
        vec![Arc::new(GenerateImageTool {
            provider: self.provider.clone(),
            file_access: self.file_access.clone(),
            config,
        })]
    }

    fn settings_namespace(&self) -> Option<&'static str> {
        Some("image")
    }

    fn settings_json_schema(&self) -> Option<schemars::schema::RootSchema> {
        Some(schemars::schema_for!(Image))
    }
}

pub struct GenerateImageTool {
    provider: SharedProvider,
    file_access: Arc<FileAccessManager>,
    config: Image,
}

impl GenerateImageTool {
    pub fn tool_name() -> ToolName {
        ToolName::new("generate_image")
    }
}

#[derive(Debug, Deserialize)]
struct GenerateImageInput {
    prompt: String,
    output_path: String,
    aspect_ratio: Option<String>,
    image_size: Option<String>,
}

#[async_trait::async_trait(?Send)]
impl ToolExecutor for GenerateImageTool {
    fn name(&self) -> String {
        "generate_image".to_string()
    }

    fn description(&self) -> String {
        "Generate an image from a text prompt and save it to a file in the workspace.".to_string()
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "Description of the image to generate"
                },
                "output_path": {
                    "type": "string",
                    "description": "Path where the generated image will be saved (e.g., /Project/assets/logo.png)"
                },
                "aspect_ratio": {
                    "type": "string",
                    "description": "Aspect ratio: 1:1, 2:3, 3:2, 3:4, 4:3, 4:5, 5:4, 9:16, 16:9, 21:9"
                },
                "image_size": {
                    "type": "string",
                    "description": "Image resolution: 1K, 2K, or 4K"
                }
            },
            "required": ["prompt", "output_path"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Execution
    }

    async fn process(&self, request: &ToolRequest) -> Result<Box<dyn ToolCallHandle>> {
        let input: GenerateImageInput = serde_json::from_value(request.arguments.clone())?;

        Ok(Box::new(GenerateImageHandle {
            input,
            tool_use_id: request.tool_use_id.clone(),
            provider: self.provider.clone(),
            file_access: self.file_access.clone(),
            config: self.config.clone(),
        }))
    }
}

struct GenerateImageHandle {
    input: GenerateImageInput,
    tool_use_id: String,
    provider: SharedProvider,
    file_access: Arc<FileAccessManager>,
    config: Image,
}

#[async_trait::async_trait(?Send)]
impl ToolCallHandle for GenerateImageHandle {
    fn tool_request(&self) -> ToolRequestEvent {
        ToolRequestEvent {
            tool_call_id: self.tool_use_id.clone(),
            tool_name: "generate_image".to_string(),
            tool_type: ToolRequestType::Other {
                args: json!({
                    "prompt": self.input.prompt,
                    "output_path": self.input.output_path
                }),
            },
        }
    }

    async fn execute(self: Box<Self>) -> ToolOutput {
        let provider = self.provider.read().unwrap().clone();

        let request = ImageGenerationRequest {
            prompt: self.input.prompt.clone(),
            model_id: self.config.model.clone(),
            aspect_ratio: self
                .input
                .aspect_ratio
                .or(Some(self.config.default_aspect_ratio.clone())),
            image_size: self
                .input
                .image_size
                .or(Some(self.config.default_image_size.clone())),
        };

        let result = match provider.generate_image(request).await {
            Ok(response) => response,
            Err(e) => {
                return ToolOutput::Result {
                    content: format!("Image generation failed: {e:?}"),
                    is_error: true,
                    continuation: ContinuationPreference::Continue,
                    ui_result: ToolExecutionResult::Other {
                        result: json!({"error": format!("{e:?}")}),
                    },
                };
            }
        };

        match self
            .file_access
            .write_bytes(&self.input.output_path, &result.image_data)
            .await
        {
            Ok(()) => ToolOutput::Result {
                content: format!(
                    "Image generated and saved to {} ({} bytes, {})",
                    self.input.output_path,
                    result.image_data.len(),
                    result.media_type
                ),
                is_error: false,
                continuation: ContinuationPreference::Continue,
                ui_result: ToolExecutionResult::Other {
                    result: json!({
                        "path": self.input.output_path,
                        "size_bytes": result.image_data.len(),
                        "media_type": result.media_type
                    }),
                },
            },
            Err(e) => ToolOutput::Result {
                content: format!("Failed to save image to {}: {e:?}", self.input.output_path),
                is_error: true,
                continuation: ContinuationPreference::Continue,
                ui_result: ToolExecutionResult::Other {
                    result: json!({"error": format!("{e:?}")}),
                },
            },
        }
    }
}
