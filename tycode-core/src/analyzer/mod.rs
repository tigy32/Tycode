pub mod get_type_docs;
pub mod rust_analyzer;
pub mod search_types;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::context::ContextComponent;
use crate::file::resolver::Resolver;
use crate::module::PromptComponent;
use crate::module::{Module, SessionStateComponent};
use crate::tools::r#trait::ToolExecutor;

use get_type_docs::GetTypeDocsTool;
use search_types::SearchTypesTool;

#[derive(Debug, Clone)]
pub struct BuildStatus {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

#[async_trait]
pub trait TypeAnalyzer: Send {
    async fn search_types_by_name(&mut self, type_name: &str) -> Result<Vec<String>>;
    async fn get_type_docs(&mut self, type_path: &str) -> Result<String>;
    async fn get_build_status(&mut self) -> Result<BuildStatus>;
}

#[derive(Clone)]
pub struct SharedTypeAnalyzer {
    inner: Arc<Mutex<Box<dyn TypeAnalyzer>>>,
}

impl SharedTypeAnalyzer {
    pub fn new(analyzer: Box<dyn TypeAnalyzer>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(analyzer)),
        }
    }

    pub async fn search_types_by_name(&self, type_name: &str) -> anyhow::Result<Vec<String>> {
        let mut analyzer = self.inner.lock().await;
        analyzer.search_types_by_name(type_name).await
    }

    pub async fn get_type_docs(&self, type_path: &str) -> anyhow::Result<String> {
        let mut analyzer = self.inner.lock().await;
        analyzer.get_type_docs(type_path).await
    }

    pub async fn get_build_status(&self) -> anyhow::Result<BuildStatus> {
        let mut analyzer = self.inner.lock().await;
        analyzer.get_build_status().await
    }
}

/// Supported languages for type analysis
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupportedLanguage {
    Rust,
}

impl SupportedLanguage {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "rust" => Some(Self::Rust),
            _ => None,
        }
    }

    pub fn all() -> &'static [&'static str] {
        &["rust"]
    }
}

pub struct AnalyzerModule {
    resolver: Resolver,
}

impl AnalyzerModule {
    pub fn new(workspace_roots: Vec<PathBuf>) -> Result<Self> {
        let resolver = Resolver::new(workspace_roots)?;
        Ok(Self { resolver })
    }
}

impl Module for AnalyzerModule {
    fn prompt_components(&self) -> Vec<Arc<dyn PromptComponent>> {
        Vec::new()
    }

    fn context_components(&self) -> Vec<Arc<dyn ContextComponent>> {
        Vec::new()
    }

    fn tools(&self) -> Vec<Arc<dyn ToolExecutor>> {
        vec![
            Arc::new(SearchTypesTool::new(self.resolver.clone())),
            Arc::new(GetTypeDocsTool::new(self.resolver.clone())),
        ]
    }

    fn session_state(&self) -> Option<Arc<dyn SessionStateComponent>> {
        None
    }
}
