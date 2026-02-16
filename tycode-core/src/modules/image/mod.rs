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
    ContinuationPreference, SharedTool, ToolCallHandle, ToolCategory, ToolExecutor, ToolOutput,
    ToolRequest,
};
use crate::tools::ToolName;

pub mod config;

use config::Image;

const MAX_READ_IMAGE_BYTES: usize = 5 * 1024 * 1024;

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

    fn tools(&self) -> Vec<SharedTool> {
        let config: Image = self.settings.settings().get_module_config("image");
        if !config.enabled {
            return vec![];
        }
        let provider = self.provider.read().unwrap();
        if !provider.supports_image_generation() {
            return vec![];
        }
        vec![
            Arc::new(GenerateImageTool {
                provider: self.provider.clone(),
                file_access: self.file_access.clone(),
                config,
            }),
            Arc::new(ReadImageTool {
                file_access: self.file_access.clone(),
            }),
        ]
    }

    fn settings_namespace(&self) -> Option<&'static str> {
        Some("image")
    }

    fn settings_json_schema(&self) -> Option<schemars::schema::RootSchema> {
        Some(schemars::schema_for!(Image))
    }
}

pub struct ReadImageTool {
    file_access: Arc<FileAccessManager>,
}

impl ReadImageTool {
    pub fn tool_name() -> ToolName {
        ToolName::new("read_image")
    }
}

#[derive(Debug, Deserialize)]
struct ReadImageInput {
    file_path: String,
}

fn media_type_from_extension(path: &str) -> Option<&'static str> {
    let ext = path.rsplit('.').next()?.to_ascii_lowercase();
    match ext.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "svg" => Some("image/svg+xml"),
        "bmp" => Some("image/bmp"),
        _ => None,
    }
}

#[async_trait::async_trait(?Send)]
impl ToolExecutor for ReadImageTool {
    fn name(&self) -> String {
        "read_image".to_string()
    }

    fn description(&self) -> String {
        "Read an image file from disk and return it as visual content the model can see."
            .to_string()
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Path to the image file to read (e.g., /Project/assets/sprite.png)"
                }
            },
            "required": ["file_path"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Execution
    }

    async fn process(&self, request: &ToolRequest) -> Result<Box<dyn ToolCallHandle>> {
        let input: ReadImageInput = serde_json::from_value(request.arguments.clone())?;

        Ok(Box::new(ReadImageHandle {
            input,
            tool_use_id: request.tool_use_id.clone(),
            file_access: self.file_access.clone(),
        }))
    }
}

struct ReadImageHandle {
    input: ReadImageInput,
    tool_use_id: String,
    file_access: Arc<FileAccessManager>,
}

#[async_trait::async_trait(?Send)]
impl ToolCallHandle for ReadImageHandle {
    fn tool_request(&self) -> ToolRequestEvent {
        ToolRequestEvent {
            tool_call_id: self.tool_use_id.clone(),
            tool_name: "read_image".to_string(),
            tool_type: ToolRequestType::Other {
                args: json!({ "file_path": self.input.file_path }),
            },
        }
    }

    async fn execute(self: Box<Self>) -> ToolOutput {
        let media_type = match media_type_from_extension(&self.input.file_path) {
            Some(mt) => mt,
            None => {
                return ToolOutput::Result {
                    content: format!(
                        "Unsupported image format for: {}. Supported: png, jpg, jpeg, gif, webp, svg, bmp",
                        self.input.file_path
                    ),
                    is_error: true,
                    continuation: ContinuationPreference::Continue,
                    ui_result: ToolExecutionResult::Other {
                        result: json!({"error": "unsupported_format"}),
                    },
                };
            }
        };

        let data = match self.file_access.read_bytes(&self.input.file_path).await {
            Ok(d) => d,
            Err(e) => {
                return ToolOutput::Result {
                    content: format!("Failed to read image {}: {e:?}", self.input.file_path),
                    is_error: true,
                    continuation: ContinuationPreference::Continue,
                    ui_result: ToolExecutionResult::Other {
                        result: json!({"error": format!("{e:?}")}),
                    },
                };
            }
        };

        if data.len() > MAX_READ_IMAGE_BYTES {
            return ToolOutput::Result {
                content: format!(
                    "Image too large: {} bytes (max {} bytes). Resize the image first.",
                    data.len(),
                    MAX_READ_IMAGE_BYTES
                ),
                is_error: true,
                continuation: ContinuationPreference::Continue,
                ui_result: ToolExecutionResult::Other {
                    result: json!({"error": "image_too_large", "size": data.len()}),
                },
            };
        }

        let size_bytes = data.len();
        ToolOutput::ImageResult {
            content: format!(
                "Image loaded: {} ({}, {} bytes)",
                self.input.file_path, media_type, size_bytes
            ),
            image_data: data,
            media_type: media_type.to_string(),
            continuation: ContinuationPreference::Continue,
            ui_result: ToolExecutionResult::Other {
                result: json!({
                    "path": self.input.file_path,
                    "media_type": media_type,
                    "size_bytes": size_bytes,
                }),
            },
        }
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
