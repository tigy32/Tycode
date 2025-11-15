pub mod rust_analyzer;

use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::Mutex;

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
