use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::agents::defaults;

#[derive(Copy, Clone, Debug)]
pub enum Builtin {
    UnderstandingTools,
    StyleMandates,
    CommunicationGuidelines,
    TaskListManagement,
}

impl Builtin {
    pub fn all() -> &'static [Builtin] {
        &[
            Builtin::UnderstandingTools,
            Builtin::StyleMandates,
            Builtin::CommunicationGuidelines,
            Builtin::TaskListManagement,
        ]
    }

    fn as_str(&self) -> &'static str {
        match self {
            Builtin::UnderstandingTools => "understanding_tools",
            Builtin::StyleMandates => "style_mandates",
            Builtin::CommunicationGuidelines => "communication_guidelines",
            Builtin::TaskListManagement => "task_list_management",
        }
    }
}

#[derive(Clone)]
pub struct SteeringDocuments {
    workspace_roots: Vec<PathBuf>,
    home_dir: PathBuf,
}

impl SteeringDocuments {
    pub fn new(workspace_roots: Vec<PathBuf>, home_dir: PathBuf) -> Self {
        Self {
            workspace_roots,
            home_dir,
        }
    }

    pub fn get_builtin(&self, builtin: Builtin) -> String {
        let name = builtin.as_str();
        if let Some(content) = self.load_from_workspace(name) {
            return content;
        }

        if let Some(content) = self.load_from_home(name) {
            return content;
        }

        self.get_default(name)
    }

    pub fn get_custom_documents(&self) -> Vec<String> {
        let mut documents = Vec::new();
        let mut seen_paths = HashSet::new();

        for workspace in &self.workspace_roots {
            let tycode_dir = workspace.join(".tycode");
            self.collect_custom_from_dir(&tycode_dir, &mut documents, &mut seen_paths);
        }

        let home_tycode = self.home_dir.join(".tycode");
        self.collect_custom_from_dir(&home_tycode, &mut documents, &mut seen_paths);

        documents
    }

    pub fn get_external_documents(&self) -> Vec<String> {
        let mut documents = Vec::new();

        for workspace in &self.workspace_roots {
            self.collect_cursor_docs(workspace, &mut documents);
            self.collect_cline_docs(workspace, &mut documents);
            self.collect_roo_docs(workspace, &mut documents);
            self.collect_kiro_docs(workspace, &mut documents);
        }

        documents
    }

    pub fn build_steering_content(&self) -> String {
        let mut sections = Vec::new();

        for builtin in Builtin::all() {
            sections.push(self.get_builtin(*builtin));
        }

        for doc in self.get_custom_documents() {
            sections.push(doc);
        }

        for doc in self.get_external_documents() {
            sections.push(doc);
        }

        sections.join("\n\n")
    }

    pub fn build_system_prompt(&self, core_prompt: &str, builtins: &[Builtin]) -> String {
        let mut prompt = core_prompt.to_string();

        for builtin in builtins {
            prompt.push_str("\n\n");
            prompt.push_str(&self.get_builtin(*builtin));
        }

        for doc in self.get_custom_documents() {
            prompt.push_str("\n\n");
            prompt.push_str(&doc);
        }

        for doc in self.get_external_documents() {
            prompt.push_str("\n\n");
            prompt.push_str(&doc);
        }

        prompt
    }

    fn load_from_workspace(&self, name: &str) -> Option<String> {
        let filename = format!("{}.md", name);

        for workspace in &self.workspace_roots {
            let path = workspace.join(".tycode").join(&filename);
            if let Some(content) = self.read_file(&path) {
                tracing::debug!(
                    "Loaded steering document override from workspace: {}",
                    path.display()
                );
                return Some(content);
            }
        }

        None
    }

    fn load_from_home(&self, name: &str) -> Option<String> {
        let filename = format!("{}.md", name);
        let path = self.home_dir.join(".tycode").join(&filename);

        if let Some(content) = self.read_file(&path) {
            tracing::debug!(
                "Loaded steering document override from home: {}",
                path.display()
            );
            return Some(content);
        }

        None
    }

    fn get_default(&self, name: &str) -> String {
        match name {
            "style_mandates" => defaults::STYLE_MANDATES.to_string(),
            "communication_guidelines" => defaults::COMMUNICATION_GUIDELINES.to_string(),
            "understanding_tools" => defaults::UNDERSTANDING_TOOLS.to_string(),
            "task_list_management" => defaults::TASK_LIST_MANAGEMENT.to_string(),
            _ => String::new(),
        }
    }

    fn collect_custom_from_dir(
        &self,
        dir: &Path,
        documents: &mut Vec<String>,
        seen_paths: &mut HashSet<PathBuf>,
    ) {
        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return,
            Err(e) => {
                tracing::warn!("Failed to read directory {}: {:?}", dir.display(), e);
                return;
            }
        };

        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    tracing::warn!(
                        "Error reading directory entry in {}: {:?}",
                        dir.display(),
                        e
                    );
                    continue;
                }
            };
            let path = entry.path();

            if !path.extension().map_or(false, |ext| ext == "md") {
                continue;
            }

            if seen_paths.contains(&path) {
                continue;
            }

            let stem = match path.file_stem().and_then(|s| s.to_str()) {
                Some(s) => s,
                None => continue,
            };

            if Builtin::all().iter().any(|b| b.as_str() == stem) {
                continue;
            }

            if let Some(content) = self.read_file(&path) {
                tracing::debug!("Loaded custom steering document: {}", path.display());
                seen_paths.insert(path);
                documents.push(content);
            }
        }
    }

    fn collect_cursor_docs(&self, workspace: &Path, documents: &mut Vec<String>) {
        let rules_dir = workspace.join(".cursor").join("rules");
        self.collect_md_files_from_dir(&rules_dir, documents);

        let cursorrules = workspace.join(".cursorrules");
        if let Some(content) = self.read_file(&cursorrules) {
            tracing::debug!("Loaded Cursor rules: {}", cursorrules.display());
            documents.push(content);
        }
    }

    fn collect_cline_docs(&self, workspace: &Path, documents: &mut Vec<String>) {
        let cline_dir = workspace.join(".cline");
        self.collect_md_files_from_dir(&cline_dir, documents);

        let clinerules = workspace.join(".clinerules");
        if let Some(content) = self.read_file(&clinerules) {
            tracing::debug!("Loaded Cline rules: {}", clinerules.display());
            documents.push(content);
        }
    }

    fn collect_roo_docs(&self, workspace: &Path, documents: &mut Vec<String>) {
        let rules_dir = workspace.join(".roo").join("rules");
        self.collect_md_files_from_dir(&rules_dir, documents);

        let roorules = workspace.join(".roorules");
        if let Some(content) = self.read_file(&roorules) {
            tracing::debug!("Loaded Roo rules: {}", roorules.display());
            documents.push(content);
        }
    }

    fn collect_kiro_docs(&self, workspace: &Path, documents: &mut Vec<String>) {
        let steering_dir = workspace.join(".kiro").join("steering-docs");
        self.collect_md_files_from_dir(&steering_dir, documents);
    }

    fn collect_md_files_from_dir(&self, dir: &Path, documents: &mut Vec<String>) {
        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return,
            Err(e) => {
                tracing::warn!("Failed to read directory {}: {:?}", dir.display(), e);
                return;
            }
        };

        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    tracing::warn!(
                        "Error reading directory entry in {}: {:?}",
                        dir.display(),
                        e
                    );
                    continue;
                }
            };
            let path = entry.path();

            if !path.extension().map_or(false, |ext| ext == "md") {
                continue;
            }

            if let Some(content) = self.read_file(&path) {
                tracing::debug!("Loaded external steering document: {}", path.display());
                documents.push(content);
            }
        }
    }

    fn read_file(&self, path: &Path) -> Option<String> {
        match fs::read_to_string(path) {
            Ok(content) => Some(content),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => {
                tracing::warn!(
                    "Failed to read steering document {}: {:?}",
                    path.display(),
                    e
                );
                None
            }
        }
    }
}
