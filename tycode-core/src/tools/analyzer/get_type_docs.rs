use crate::analyzer::rust_analyzer::RustAnalyzer;
use crate::analyzer::TypeAnalyzer;
use crate::file::resolver::Resolver;
use crate::tools::analyzer::SupportedLanguage;
use crate::tools::r#trait::{ToolCategory, ToolExecutor, ToolRequest, ValidatedToolCall};
use anyhow::{bail, Result};
use serde_json::{json, Value};

pub struct GetTypeDocsTool {
    resolver: Resolver,
}

impl GetTypeDocsTool {
    pub fn new(resolver: Resolver) -> Self {
        Self { resolver }
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

                let mut analyzer = RustAnalyzer::new(workspace_root);
                let docs = analyzer.get_type_docs(type_path).await?;

                Ok(ValidatedToolCall::context_only(json!({
                    "documentation": docs
                })))
            }
        }
    }
}
