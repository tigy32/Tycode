use crate::chat::actor::ActorState;
use crate::chat::events::ChatMessage;
use crate::file::config::File;
use crate::module::SlashCommand;
use crate::settings::config::FileModificationApi;

pub struct FileApiSlashCommand;

#[async_trait::async_trait(?Send)]
impl SlashCommand for FileApiSlashCommand {
    fn name(&self) -> &'static str {
        "fileapi"
    }

    fn description(&self) -> &'static str {
        "Set the file modification API (patch or find-replace)"
    }

    fn usage(&self) -> &'static str {
        "/fileapi <patch|findreplace>"
    }

    fn hidden(&self) -> bool {
        false
    }

    async fn execute(&self, state: &mut ActorState, args: &[&str]) -> Vec<ChatMessage> {
        let Some(api_name) = args.first() else {
            return show_current(state);
        };

        let (api, label) = match api_name.to_lowercase().as_str() {
            "patch" => (FileModificationApi::Patch, "patch"),
            "findreplace" | "find-replace" => (FileModificationApi::FindReplace, "find-replace"),
            _ => {
                return vec![ChatMessage::error(
                    "Unknown file API. Use: patch, findreplace".to_string(),
                )];
            }
        };

        let mut config: File = state.settings.get_module_config(File::NAMESPACE);
        config.file_modification_api = api;
        state.settings.set_module_config(File::NAMESPACE, config);

        vec![ChatMessage::system(format!(
            "File modification API set to: {label}"
        ))]
    }
}

fn show_current(state: &ActorState) -> Vec<ChatMessage> {
    let file_config: File = state.settings.get_module_config(File::NAMESPACE);
    let current_api = match file_config.file_modification_api {
        FileModificationApi::Patch => "patch",
        FileModificationApi::FindReplace => "find-replace",
        FileModificationApi::Default => "default",
    };
    vec![ChatMessage::system(format!(
        "Current file modification API: {current_api}. Usage: /fileapi <patch|findreplace>"
    ))]
}
