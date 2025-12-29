use crate::agents::catalog::AgentCatalog;
use crate::agents::tool_type::ToolType;
use crate::ai::tweaks::RegistryFileModificationApi;
use crate::ai::{ToolDefinition, ToolUseData};
use crate::file::access::FileAccessManager;
use crate::file::resolver::Resolver;
use crate::memory::AppendMemoryTool;
use crate::memory::MemoryLog;
use crate::settings::SettingsManager;
use crate::tools::ask_user_question::AskUserQuestion;
use crate::tools::complete_task::CompleteTask;
use crate::tools::file::apply_codex_patch::ApplyCodexPatchTool;
use crate::tools::file::delete_file::DeleteFileTool;
use crate::tools::file::list_files::ListFilesTool;
use crate::tools::file::read_file::ReadFileTool;
use crate::tools::file::replace_in_file::ReplaceInFileTool;
use crate::tools::file::search_files::SearchFilesTool;
use crate::tools::file::write_file::WriteFileTool;
use crate::tools::r#trait::{ToolCallHandle, ToolCategory, ToolExecutor, ToolRequest};
use crate::tools::spawn::spawn_agent::SpawnAgent;
use crate::tools::spawn::spawn_coder::SpawnCoder;
use crate::tools::spawn::spawn_recon::SpawnRecon;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{debug, error};

use crate::tools::analyzer::get_type_docs::GetTypeDocsTool;
use crate::tools::analyzer::search_types::SearchTypesTool;

use super::run_build_test::RunBuildTestTool;

pub struct ToolRegistry {
    tools: BTreeMap<String, Arc<dyn ToolExecutor>>,
    mcp_tools: BTreeSet<String>,
}

impl ToolRegistry {
    pub async fn new(
        workspace_roots: Vec<PathBuf>,
        file_modification_api: RegistryFileModificationApi,
        enable_type_analyzer: bool,
        memory_log: Arc<MemoryLog>,
        additional_tools: Vec<Arc<dyn ToolExecutor>>,
        settings: SettingsManager,
        agent_catalog: Arc<AgentCatalog>,
    ) -> anyhow::Result<Self> {
        let mut registry = Self {
            tools: BTreeMap::new(),
            mcp_tools: BTreeSet::new(),
        };

        registry.register_file_tools(workspace_roots.clone(), file_modification_api)?;
        registry.register_command_tools(workspace_roots.clone(), settings)?;
        registry.register_agent_tools(agent_catalog);
        registry.register_memory_tools(memory_log);

        if enable_type_analyzer {
            registry.register_lsp_tools(workspace_roots.clone())?;
        }

        for tool in additional_tools {
            registry.register_tool(tool);
        }

        Ok(registry)
    }

    fn register_file_tools(
        &mut self,
        workspace_roots: Vec<PathBuf>,
        file_modification_api: RegistryFileModificationApi,
    ) -> anyhow::Result<()> {
        self.register_tool(Arc::new(ReadFileTool::new(workspace_roots.clone())?));
        self.register_tool(Arc::new(WriteFileTool::new(workspace_roots.clone())?));
        self.register_tool(Arc::new(ListFilesTool::new(workspace_roots.clone())?));
        self.register_tool(Arc::new(SearchFilesTool::new(FileAccessManager::new(
            workspace_roots.clone(),
        )?)));
        self.register_tool(Arc::new(DeleteFileTool::new(workspace_roots.clone())?));

        match file_modification_api {
            RegistryFileModificationApi::Patch => {
                debug!("Registering ApplyCodexPatchTool for Patch API");
                self.register_tool(Arc::new(ApplyCodexPatchTool::new(workspace_roots)?));
            }
            RegistryFileModificationApi::FindReplace => {
                debug!("Registering ReplaceInFileTool for FindReplace API");
                self.register_tool(Arc::new(ReplaceInFileTool::new(workspace_roots)?));
            }
        }
        Ok(())
    }

    fn register_command_tools(
        &mut self,
        workspace_roots: Vec<PathBuf>,
        settings: SettingsManager,
    ) -> anyhow::Result<()> {
        self.register_tool(Arc::new(RunBuildTestTool::new(workspace_roots, settings)?));
        Ok(())
    }

    fn register_agent_tools(&mut self, catalog: Arc<AgentCatalog>) {
        self.register_tool(Arc::new(SpawnAgent::new(catalog.clone())));
        self.register_tool(Arc::new(SpawnCoder::new(catalog.clone())));
        self.register_tool(Arc::new(SpawnRecon::new(catalog)));
        self.register_tool(Arc::new(CompleteTask));
        self.register_tool(Arc::new(AskUserQuestion));
    }

    fn register_lsp_tools(&mut self, workspace_roots: Vec<PathBuf>) -> anyhow::Result<()> {
        debug!("Registering LSP analyzer tools");
        let resolver = Resolver::new(workspace_roots)?;
        self.register_tool(Arc::new(SearchTypesTool::new(resolver.clone())));
        self.register_tool(Arc::new(GetTypeDocsTool::new(resolver)));
        Ok(())
    }

    fn register_memory_tools(&mut self, memory_log: Arc<MemoryLog>) {
        debug!("Registering memory tools");
        self.register_tool(Arc::new(AppendMemoryTool::new(memory_log)));
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

    pub async fn process_tools(
        &self,
        tool_use: &ToolUseData,
        allowed_tool_types: &[ToolType],
    ) -> Result<Box<dyn ToolCallHandle>, String> {
        let allowed_names: Vec<&str> = allowed_tool_types
            .iter()
            .map(|&tool_type| tool_type.name())
            .filter(|name| self.tools.contains_key(*name))
            .chain(self.mcp_tools.iter().map(|s| s.as_str()))
            .collect();

        let tool = match self.tools.get(&tool_use.name) {
            Some(tool) => tool,
            None => {
                let available = allowed_names.join(", ");
                error!(tool_name = %tool_use.name, "Unknown tool");
                return Err(format!(
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
            return Err(format!(
                "Tool not available for current agent: {}",
                tool_use.name
            ));
        }

        let schema = tool.input_schema();
        let coerced_arguments =
            match crate::tools::fuzzy_json::coerce_to_schema(&tool_use.arguments, &schema) {
                Ok(args) => args,
                Err(e) => {
                    error!(?e, tool_name = %tool_use.name, "Failed to coerce tool arguments");
                    return Err(format!("Failed to coerce arguments: {e:?}"));
                }
            };

        let request = ToolRequest::new(coerced_arguments, tool_use.id.clone());
        tool.process(&request).await.map_err(|e| {
            error!(?e, tool_name = %tool_use.name, "Tool processing failed");
            format!("Error: {e:?}")
        })
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
