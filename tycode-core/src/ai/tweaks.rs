use crate::ai::model::Model;
use crate::ai::provider::AiProvider;
use crate::settings::config::{FileModificationApi, Settings, ToolCallStyle};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistryFileModificationApi {
    Patch,
    FindReplace,
}

#[derive(Debug, Clone, Default)]
pub struct ModelTweaks {
    pub file_modification_api: Option<RegistryFileModificationApi>,
    pub tool_call_style: Option<ToolCallStyle>,
}

impl ModelTweaks {
    pub fn merge_with(&self, other: &ModelTweaks) -> ModelTweaks {
        ModelTweaks {
            file_modification_api: other.file_modification_api.or(self.file_modification_api),
            tool_call_style: other.tool_call_style.or(self.tool_call_style),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ResolvedTweaks {
    pub file_modification_api: RegistryFileModificationApi,
    pub tool_call_style: ToolCallStyle,
}

pub fn resolve_tweaks(
    settings_file_api: FileModificationApi,
    settings_tool_style: Option<ToolCallStyle>,
    provider: &dyn AiProvider,
    model: Model,
) -> ResolvedTweaks {
    let model_tweaks = model.tweaks();
    let provider_tweaks = provider.tweaks();

    let merged = model_tweaks.merge_with(&provider_tweaks);

    let file_modification_api = match settings_file_api {
        FileModificationApi::Patch => RegistryFileModificationApi::Patch,
        FileModificationApi::FindReplace => RegistryFileModificationApi::FindReplace,
        FileModificationApi::Default => merged
            .file_modification_api
            .unwrap_or(RegistryFileModificationApi::FindReplace),
    };

    let tool_call_style = settings_tool_style
        .or(merged.tool_call_style)
        .unwrap_or(ToolCallStyle::Json);

    ResolvedTweaks {
        file_modification_api,
        tool_call_style,
    }
}

pub fn resolve_from_settings(
    settings: &Settings,
    provider: &dyn AiProvider,
    model: crate::ai::model::Model,
) -> ResolvedTweaks {
    let settings_tool_style = if settings.xml_tool_mode {
        Some(ToolCallStyle::Xml)
    } else {
        None
    };
    resolve_tweaks(
        settings.file_modification_api.clone(),
        settings_tool_style,
        provider,
        model,
    )
}
