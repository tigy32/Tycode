use std::collections::HashSet;
use std::fs;
use std::path::Path;

pub struct Ignored {
    patterns: HashSet<String>,
}

impl Ignored {
    pub fn new(root: &Path) -> anyhow::Result<Self> {
        // Default patterns we respect even if there is no .gitignore
        let mut patterns = HashSet::new();
        patterns.insert(".git".to_string());
        patterns.insert("node_modules".to_string());
        patterns.insert("*.pyc".to_string());
        patterns.insert("__pycache__".to_string());
        patterns.insert(".DS_Store".to_string());

        let gitignore_path = root.join(".gitignore");
        if gitignore_path.exists() {
            let content = fs::read_to_string(&gitignore_path)?;
            for line in content.lines() {
                let line = line.trim();
                if !line.is_empty() && !line.starts_with('#') {
                    patterns.insert(line.to_string());
                }
            }
        }

        Ok(Self { patterns })
    }

    pub fn is_ignored(&self, path: &str, is_dir: bool) -> bool {
        // Hard-code ignoring dot directories to prevent accidental modification,
        // but allow dot files like .gitignore - balances safety with utility.
        let components: Vec<&str> = path.split('/').collect();
        for (i, c) in components.iter().enumerate() {
            if c.starts_with('.') {
                // Skip the first real component (workspace root) as it's already validated
                if i == 1 && !c.is_empty() {
                    continue;
                }

                if i < components.len() - 1 {
                    return true; // intermediate dot component, like .git in path
                } else {
                    // last component starts with '.', only ignore if directory
                    if is_dir {
                        return true;
                    }
                }
            }
        }

        for pattern in &self.patterns {
            if let Some(root_pattern) = pattern.strip_prefix('/') {
                if let Some(dir_name) = root_pattern.strip_suffix('/') {
                    if path == dir_name || path.starts_with(&format!("{dir_name}/")) {
                        return true;
                    }
                } else if path == root_pattern || path.starts_with(&format!("{root_pattern}/")) {
                    return true;
                }
            } else if pattern.ends_with('/') {
                let dir_name = &pattern[..pattern.len() - 1];

                if path == dir_name || path.starts_with(&format!("{dir_name}/")) {
                    return true;
                }

                let components: Vec<&str> = path.split('/').collect();
                if components.contains(&dir_name) {
                    return true;
                }
            } else if pattern.starts_with("*.") {
                let ext = &pattern[1..];
                if path.ends_with(ext) {
                    return true;
                }
            } else if pattern.contains('/') {
                if path == pattern || path.starts_with(&format!("{pattern}/")) {
                    return true;
                }
            } else {
                let components: Vec<&str> = path.split('/').collect();

                if components.contains(&pattern.as_str()) {
                    return true;
                }

                if path == pattern {
                    return true;
                }
            }
        }
        false
    }
}
