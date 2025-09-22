#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolType {
    ReadFile,
    WriteFile,
    ListFiles,
    SearchFiles,
    ModifyFile,
    RunBuildTestCommand,
    DeleteFile,
    SetTrackedFiles,
    SpawnAgent,
    SpawnRecon,
    CompleteTask,
    AskUserQuestion,
}

impl ToolType {
    pub fn name(&self) -> &'static str {
        match self {
            Self::ReadFile => "read_file",
            Self::WriteFile => "write_file",
            Self::ListFiles => "list_files",
            Self::SearchFiles => "search_files",
            Self::ModifyFile => "modify_file",
            Self::RunBuildTestCommand => "run_build_test",
            Self::DeleteFile => "delete_file",
            Self::SetTrackedFiles => "set_tracked_files",
            Self::SpawnAgent => "spawn_agent",
            Self::SpawnRecon => "spawn_recon",
            Self::CompleteTask => "complete_task",
            Self::AskUserQuestion => "ask_user_question",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "read_file" => Some(Self::ReadFile),
            "write_file" => Some(Self::WriteFile),
            "list_files" => Some(Self::ListFiles),
            "search_files" => Some(Self::SearchFiles),
            "modify_file" => Some(Self::ModifyFile),
            "run_build_test" => Some(Self::RunBuildTestCommand),
            "delete_file" => Some(Self::DeleteFile),
            "set_tracked_files" => Some(Self::SetTrackedFiles),
            "spawn_agent" => Some(Self::SpawnAgent),
            "complete_task" => Some(Self::CompleteTask),
            "ask_user_question" => Some(Self::AskUserQuestion),
            _ => None,
        }
    }
}
