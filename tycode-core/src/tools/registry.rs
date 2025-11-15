use crate::agents::tool_type::ToolType;
use crate::ai::{ToolDefinition, ToolUseData};
use crate::file::access::FileAccessManager;
use crate::file::resolver::Resolver;
use crate::settings::config::FileModificationApi;
use crate::tools::ask_user_question::AskUserQuestion;
use crate::tools::complete_task::CompleteTask;
use crate::tools::file::apply_codex_patch::ApplyCodexPatchTool;
use crate::tools::file::delete_file::DeleteFileTool;
use crate::tools::file::list_files::ListFilesTool;
use crate::tools::file::read_file::ReadFileTool;
use crate::tools::file::replace_in_file::ReplaceInFileTool;
use crate::tools::file::search_files::SearchFilesTool;
use crate::tools::file::set_tracked_files::SetTrackedFilesTool;
use crate::tools::file::write_file::WriteFileTool;
use crate::tools::mcp::manager::McpManager;
use crate::tools::r#trait::{ToolCategory, ToolExecutor, ToolRequest, ValidatedToolCall};
use crate::tools::spawn::spawn_agent::SpawnAgent;
use crate::tools::spawn::spawn_coder::SpawnCoder;
use crate::tools::tasks::manage_task_list::ManageTaskListTool;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{debug, error, info};

use crate::tools::analyzer::get_type_docs::GetTypeDocsTool;
use crate::tools::analyzer::search_types::SearchTypesTool;

use super::run_build_test::RunBuildTestTool;

/// File modification API for tool registry (without Default variant)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistryFileModificationApi {
    Patch,
    FindReplace,
}

/// Helper function to map from settings enum + model to registry enum
pub fn resolve_file_modification_api(
    settings_api: FileModificationApi,
    model: crate::ai::model::Model,
) -> RegistryFileModificationApi {
    match settings_api {
        FileModificationApi::Patch => RegistryFileModificationApi::Patch,
        FileModificationApi::FindReplace => RegistryFileModificationApi::FindReplace,
        FileModificationApi::Default => model.preferred_file_modification_api(),
    }
}

pub struct ToolRegistry {
    tools: BTreeMap<String, Arc<dyn ToolExecutor>>,
    mcp_tools: BTreeSet<String>,
}

impl ToolRegistry {
    pub async fn new(
        workspace_roots: Vec<PathBuf>,
        file_modification_api: RegistryFileModificationApi,
        mcp_manager: Option<&McpManager>,
        enable_type_analyzer: bool,
    ) -> anyhow::Result<Self> {
        let mut registry = Self {
            tools: BTreeMap::new(),
            mcp_tools: BTreeSet::new(),
        };

        registry.register_file_tools(workspace_roots.clone(), file_modification_api);
        registry.register_command_tools(workspace_roots.clone());
        registry.register_agent_tools();

        if enable_type_analyzer {
            registry.register_lsp_tools(workspace_roots.clone())?;
        }

        if let Some(manager) = mcp_manager {
            registry.register_mcp_tools(manager)?;
        }

        Ok(registry)
    }

    fn register_file_tools(
        &mut self,
        workspace_roots: Vec<PathBuf>,
        file_modification_api: RegistryFileModificationApi,
    ) {
        self.register_tool(Arc::new(ReadFileTool::new(workspace_roots.clone())));
        self.register_tool(Arc::new(WriteFileTool::new(workspace_roots.clone())));
        self.register_tool(Arc::new(ListFilesTool::new(workspace_roots.clone())));
        self.register_tool(Arc::new(SearchFilesTool::new(FileAccessManager::new(
            workspace_roots.clone(),
        ))));
        self.register_tool(Arc::new(DeleteFileTool::new(workspace_roots.clone())));
        self.register_tool(Arc::new(SetTrackedFilesTool::new(workspace_roots.clone())));

        match file_modification_api {
            RegistryFileModificationApi::Patch => {
                debug!("Registering ApplyCodexPatchTool for Patch API");
                self.register_tool(Arc::new(ApplyCodexPatchTool::new(workspace_roots)));
            }
            RegistryFileModificationApi::FindReplace => {
                debug!("Registering ReplaceInFileTool for FindReplace API");
                self.register_tool(Arc::new(ReplaceInFileTool::new(workspace_roots)));
            }
        }
    }

    fn register_command_tools(&mut self, workspace_roots: Vec<PathBuf>) {
        self.register_tool(Arc::new(RunBuildTestTool::new(workspace_roots)));
    }

    fn register_agent_tools(&mut self) {
        self.register_tool(Arc::new(SpawnAgent));
        self.register_tool(Arc::new(SpawnCoder));
        self.register_tool(Arc::new(CompleteTask));
        self.register_tool(Arc::new(AskUserQuestion));
        self.register_tool(Arc::new(ManageTaskListTool));
    }

    fn register_lsp_tools(&mut self, workspace_roots: Vec<PathBuf>) -> anyhow::Result<()> {
        debug!("Registering LSP analyzer tools");
        let resolver = Resolver::new(workspace_roots)?;
        self.register_tool(Arc::new(SearchTypesTool::new(resolver.clone())));
        self.register_tool(Arc::new(GetTypeDocsTool::new(resolver)));
        Ok(())
    }

    fn register_mcp_tools(&mut self, mcp_manager: &McpManager) -> anyhow::Result<()> {
        let mcp_tools = mcp_manager.get_tools_as_executors();

        for tool in mcp_tools {
            let name = tool.name().to_string();
            debug!(tool_name = %name, "Registering MCP tool");
            self.mcp_tools.insert(name.clone());
            self.tools.insert(name, tool);
        }

        let stats = mcp_manager.get_stats();
        info!(
            servers = stats.server_count,
            tools = stats.tool_count,
            "MCP tools registered"
        );

        Ok(())
    }

    pub fn register_tool(&mut self, tool: Arc<dyn ToolExecutor>) {
        let name = tool.name().to_string();
        debug!(tool_name = %name, "Registering tool");
        self.tools.insert(name, tool);
    }

    /// Gets tool definitions for a specific set of tool types
    pub fn get_tool_definitions_for_types(&self, tool_types: &[ToolType]) -> Vec<ToolDefinition> {
        tool_types
            .iter()
            .map(|&tool_type| tool_type.name())
            .filter_map(|tool_name| self.tools.get(tool_name))
            .map(|tool| ToolDefinition {
                name: tool.name().to_string(),
                description: tool.description().to_string(),
                input_schema: tool.input_schema(),
            })
            .chain(self.get_mcp_definitions())
            .collect()
    }

    pub fn get_mcp_definitions(&self) -> Vec<ToolDefinition> {
        self.mcp_tools
            .iter()
            .map(|tool| {
                let tool = self.tools.get(tool).unwrap();
                ToolDefinition {
                    name: tool.name().to_string(),
                    description: tool.description().to_string(),
                    input_schema: tool.input_schema(),
                }
            })
            .collect()
    }

    pub fn get_tool_definitions(&self) -> Vec<ToolDefinition> {
        self.tools
            .values()
            .map(|tool| ToolDefinition {
                name: tool.name().to_string(),
                description: tool.description().to_string(),
                input_schema: tool.input_schema(),
            })
            .collect()
    }

    pub async fn validate_tools(
        &self,
        tool_use: &ToolUseData,
        allowed_tool_types: &[ToolType],
    ) -> crate::tools::r#trait::ValidatedToolCall {
        // Build list of allowed tools for this agent (only include tools actually in registry)
        let allowed_names: Vec<&str> = allowed_tool_types
            .iter()
            .map(|&tool_type| tool_type.name())
            .filter(|name| self.tools.contains_key(*name))
            .chain(self.mcp_tools.iter().map(|s| s.as_str()))
            .collect();

        // Attempt to retrieve the requested tool. If it does not exist, include a list of available tools.
        let tool = match self.tools.get(&tool_use.name) {
            Some(tool) => tool,
            None => {
                // Build a comma‑separated list of allowed tool names for diagnostics.
                let available = allowed_names.join(", ");
                error!(tool_name = %tool_use.name, "Unknown tool");
                return crate::tools::r#trait::ValidatedToolCall::Error(format!(
                    "Unknown tool: {}. Available tools: {}",
                    tool_use.name, available
                ));
            }
        };

        if !allowed_names.contains(&tool_use.name.as_str()) {
            debug!(
                tool_name = %tool_use.name,
                allowed_tools = ?allowed_names,
                "Tool not in allowed list for current agent"
            );
            return crate::tools::r#trait::ValidatedToolCall::Error(format!(
                "Tool not available for current agent: {}",
                tool_use.name
            ));
        }

        // Apply fuzzy JSON coercion to handle common model mistakes
        let schema = tool.input_schema();
        let coerced_arguments =
            match crate::tools::fuzzy_json::coerce_to_schema(&tool_use.arguments, &schema) {
                Ok(args) => args,
                Err(e) => {
                    error!(?e, tool_name = %tool_use.name, "Failed to coerce tool arguments");
                    return crate::tools::r#trait::ValidatedToolCall::Error(format!(
                        "Failed to coerce arguments: {e:?}"
                    ));
                }
            };

        let request = ToolRequest::new(coerced_arguments, tool_use.id.clone());
        match tool.validate(&request).await {
            Ok(result) => result,
            Err(e) => {
                error!(?e, tool_name = %tool_use.name, "Tool execution failed");
                ValidatedToolCall::Error(format!("Error: {e:?}"))
            }
        }
    }

    pub fn list_tools(&self) -> Vec<&str> {
        self.tools.keys().map(|s| s.as_str()).collect()
    }

    /// Get tool executor by name
    pub fn get_tool_executor_by_name(&self, name: &str) -> Option<&Arc<dyn ToolExecutor>> {
        self.tools.get(name)
    }

    /// Get tool category by name
    pub fn get_tool_category_by_name(&self, name: &str) -> Option<ToolCategory> {
        self.tools.get(name).map(|executor| executor.category())
    }
}
