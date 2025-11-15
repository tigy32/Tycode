use crate::analyzer::rust_analyzer::RustAnalyzer;
use crate::analyzer::TypeAnalyzer;
use crate::file::resolver::Resolver;
use crate::tools::analyzer::SupportedLanguage;
use crate::tools::r#trait::{ToolCategory, ToolExecutor, ToolRequest, ValidatedToolCall};
use anyhow::{bail, Result};
use serde_json::{json, Value};

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

    async fn validate(&self, request: &ToolRequest) -> Result<ValidatedToolCall> {
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

                let mut analyzer = RustAnalyzer::new(workspace_root);
                let results = analyzer.search_types_by_name(type_name).await?;

                Ok(ValidatedToolCall::context_only(json!({
                    "type_paths": results
                })))
            }
        }
    }
}
