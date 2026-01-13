use crate::analyzer::rust_analyzer::RustAnalyzer;
use crate::analyzer::{SupportedLanguage, TypeAnalyzer};
use crate::chat::events::{ToolExecutionResult, ToolRequest as ToolRequestEvent, ToolRequestType};
use crate::file::resolver::Resolver;
use crate::tools::r#trait::{
    ContinuationPreference, ToolCallHandle, ToolCategory, ToolExecutor, ToolOutput, ToolRequest,
};
use crate::tools::ToolName;
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::path::PathBuf;

pub struct GetTypeDocsTool {
    resolver: Resolver,
}

impl GetTypeDocsTool {
    pub fn new(resolver: Resolver) -> Self {
        Self { resolver }
    }

    pub fn tool_name() -> ToolName {
        ToolName::new("get_type_docs")
    }
}

#[async_trait::async_trait(?Send)]
impl ToolExecutor for GetTypeDocsTool {
    fn name(&self) -> &'static str {
        "get_type_docs"
    }

    fn description(&self) -> &'static str {
        "Get documentation and code outline for a type. Returns the type definition with doc comments, fields/variants, method signatures (with bodies stripped), and trait implementations. Use this tool to understanding types from dependencies or unfamiliar parts of the codebase rather than guessing or hallucinating."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "language": {
                    "type": "string",
                    "description": "The programming language to analyze",
                    "enum": SupportedLanguage::all()
                },
                "workspace_root": {
                    "type": "string",
                    "description": "The workspace name to search in"
                },
                "type_path": {
                    "type": "string",
                    "description": "Type identifier as container::name (e.g., \"std::vec::Vec\")"
                },
            },
            "required": ["language", "workspace_root", "type_path"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Execution
    }

    async fn process(&self, request: &ToolRequest) -> Result<Box<dyn ToolCallHandle>> {
        let Some(language_str) = request.arguments["language"].as_str() else {
            bail!("Missing required argument \"language\"");
        };
        let Some(language) = SupportedLanguage::from_str(language_str) else {
            bail!(
                "Unsupported language \"{}\". Supported: {:?}",
                language_str,
                SupportedLanguage::all()
            );
        };
        let Some(workspace_root_str) = request.arguments["workspace_root"].as_str() else {
            bail!("Missing required argument \"workspace_root\"");
        };
        let Some(type_path) = request.arguments["type_path"].as_str() else {
            bail!("Missing required argument \"type_path\"");
        };

        let Some(workspace_root) = self.resolver.root(workspace_root_str) else {
            bail!(
                "workspace_root must be one of the configured workspace roots: {:?}",
                self.resolver.roots()
            );
        };

        match language {
            SupportedLanguage::Rust => {
                if !workspace_root.join("Cargo.toml").exists() {
                    bail!("workspace_root does not contain a Cargo.toml");
                }

                Ok(Box::new(GetTypeDocsHandle {
                    language: language_str.to_string(),
                    workspace_root: workspace_root.to_path_buf(),
                    type_path: type_path.to_string(),
                    tool_use_id: request.tool_use_id.clone(),
                }))
            }
        }
    }
}

struct GetTypeDocsHandle {
    language: String,
    workspace_root: PathBuf,
    type_path: String,
    tool_use_id: String,
}

#[async_trait::async_trait(?Send)]
impl ToolCallHandle for GetTypeDocsHandle {
    fn tool_request(&self) -> ToolRequestEvent {
        ToolRequestEvent {
            tool_call_id: self.tool_use_id.clone(),
            tool_name: "get_type_docs".to_string(),
            tool_type: ToolRequestType::GetTypeDocs {
                language: self.language.clone(),
                workspace_root: self.workspace_root.display().to_string(),
                type_path: self.type_path.clone(),
            },
        }
    }

    async fn execute(self: Box<Self>) -> ToolOutput {
        let mut analyzer = RustAnalyzer::new(self.workspace_root.clone());

        match analyzer.get_type_docs(&self.type_path).await {
            Ok(docs) => {
                let content = json!({
                    "type_path": self.type_path,
                    "documentation": docs,
                });
                ToolOutput::Result {
                    content: content.to_string(),
                    is_error: false,
                    continuation: ContinuationPreference::Continue,
                    ui_result: ToolExecutionResult::GetTypeDocs {
                        documentation: docs,
                    },
                }
            }
            Err(e) => ToolOutput::Result {
                content: format!("Failed to get type docs: {e:?}"),
                is_error: true,
                continuation: ContinuationPreference::Continue,
                ui_result: ToolExecutionResult::Error {
                    short_message: "Get docs failed".to_string(),
                    detailed_message: format!("Failed to get type docs: {e:?}"),
                },
            },
        }
    }
}
