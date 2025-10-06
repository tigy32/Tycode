use crate::agents::tool_type::ToolType;
use crate::ai::{ToolDefinition, ToolUseData};
use crate::chat::state::FileModificationApi;
use crate::file::access::FileAccessManager;
use crate::tools::ask_user_question::AskUserQuestion;
use crate::tools::complete_task::CompleteTask;
use crate::tools::file::apply_patch::ApplyPatchTool;
use crate::tools::file::delete_file::DeleteFileTool;
use crate::tools::file::list_files::ListFilesTool;
use crate::tools::file::read_file::ReadFileTool;
use crate::tools::file::replace_in_file::ReplaceInFileTool;
use crate::tools::file::search_files::SearchFilesTool;
use crate::tools::file::set_tracked_files::SetTrackedFilesTool;
use crate::tools::file::write_file::WriteFileTool;
use crate::tools::r#trait::{ToolExecutor, ToolRequest, ValidatedToolCall};
use crate::tools::spawn::spawn_agent::SpawnAgent;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{debug, error};

use super::run_build_test::RunBuildTestTool;

pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn ToolExecutor>>,
}

impl ToolRegistry {
    pub fn new(workspace_roots: Vec<PathBuf>, file_modification_api: FileModificationApi) -> Self {
        let mut registry = Self {
            tools: HashMap::new(),
        };

        registry.register_file_tools(workspace_roots.clone(), file_modification_api);
        registry.register_command_tools(workspace_roots);
        registry.register_agent_tools();
        registry
    }

    fn register_file_tools(
        &mut self,
        workspace_roots: Vec<PathBuf>,
        file_modification_api: FileModificationApi,
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
            FileModificationApi::Patch => {
                debug!("Registering ApplyPatchTool for Patch API");
                self.register_tool(Arc::new(ApplyPatchTool::new(workspace_roots)));
            }
            FileModificationApi::FindReplace => {
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
        self.register_tool(Arc::new(CompleteTask));
        self.register_tool(Arc::new(AskUserQuestion));
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
        // Attempt to retrieve the requested tool. If it does not exist, include a list of available tools.
        let tool = match self.tools.get(&tool_use.name) {
            Some(tool) => tool,
            None => {
                // Build a comma‑separated list of tool names for diagnostics.
                let available = self.list_tools().join(", ");
                error!(tool_name = %tool_use.name, "Unknown tool");
                return crate::tools::r#trait::ValidatedToolCall::Error(format!(
                    "Unknown tool: {}. Available tools: {}",
                    tool_use.name, available
                ));
            }
        };

        // Then check if the tool is allowed by the agent (if restrictions are provided)
        let allowed_names: Vec<&str> = allowed_tool_types
            .iter()
            .map(|&tool_type| tool_type.name())
            .collect();

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

        let request = ToolRequest::new(tool_use.arguments.clone(), tool_use.id.clone());
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
}
