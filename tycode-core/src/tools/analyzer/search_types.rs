use crate::analyzer::rust_analyzer::RustAnalyzer;
use crate::analyzer::TypeAnalyzer;
use crate::chat::events::{ToolExecutionResult, ToolRequest as ToolRequestEvent, ToolRequestType};
use crate::file::resolver::Resolver;
use crate::tools::analyzer::SupportedLanguage;
use crate::tools::r#trait::{
    ContinuationPreference, ToolCallHandle, ToolCategory, ToolExecutor, ToolOutput, ToolRequest,
};
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::path::PathBuf;

pub struct SearchTypesTool {
    resolver: Resolver,
}

impl SearchTypesTool {
    pub fn new(resolver: Resolver) -> Self {
        Self { resolver }
    }
}

#[async_trait::async_trait(?Send)]
impl ToolExecutor for SearchTypesTool {
    fn name(&self) -> &'static str {
        "search_types"
    }

    fn description(&self) -> &'static str {
        "Search for types by name using LSP workspace/symbol. Returns type identifiers formatted as container::name."
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
                "type_name": {
                    "type": "string",
                    "description": "The type name to search for (substring match)"
                },
            },
            "required": ["language", "workspace_root", "type_name"]
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
        let Some(type_name) = request.arguments["type_name"].as_str() else {
            bail!("Missing required argument \"type_name\"");
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

                Ok(Box::new(SearchTypesHandle {
                    language: language_str.to_string(),
                    workspace_root: workspace_root.to_path_buf(),
                    type_name: type_name.to_string(),
                    tool_use_id: request.tool_use_id.clone(),
                }))
            }
        }
    }
}

struct SearchTypesHandle {
    language: String,
    workspace_root: PathBuf,
    type_name: String,
    tool_use_id: String,
}

#[async_trait::async_trait(?Send)]
impl ToolCallHandle for SearchTypesHandle {
    fn tool_request(&self) -> ToolRequestEvent {
        ToolRequestEvent {
            tool_call_id: self.tool_use_id.clone(),
            tool_name: "search_types".to_string(),
            tool_type: ToolRequestType::SearchTypes {
                language: self.language.clone(),
                workspace_root: self.workspace_root.display().to_string(),
                type_name: self.type_name.clone(),
            },
        }
    }

    async fn execute(self: Box<Self>) -> ToolOutput {
        let mut analyzer = RustAnalyzer::new(self.workspace_root.clone());

        match analyzer.search_types_by_name(&self.type_name).await {
            Ok(types) => {
                let count = types.len();
                let content = json!({
                    "types": types,
                    "count": count,
                });
                ToolOutput::Result {
                    content: content.to_string(),
                    is_error: false,
                    continuation: ContinuationPreference::Continue,
                    ui_result: ToolExecutionResult::SearchTypes { types },
                }
            }
            Err(e) => ToolOutput::Result {
                content: format!("Failed to search types: {e:?}"),
                is_error: true,
                continuation: ContinuationPreference::Continue,
                ui_result: ToolExecutionResult::Error {
                    short_message: "Search failed".to_string(),
                    detailed_message: format!("Failed to search types: {e:?}"),
                },
            },
        }
    }
}
