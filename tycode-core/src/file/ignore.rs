use ignore::gitignore::{Gitignore, GitignoreBuilder};
use std::path::Path;

#[derive(Clone)]
pub struct Ignored {
    matcher: Gitignore,
}

impl Ignored {
    pub fn new(root: &Path) -> anyhow::Result<Self> {
        let mut builder = GitignoreBuilder::new(root);

        let git_dir = root.join(".git");
        if git_dir.exists() {
            let gitignore_path = root.join(".gitignore");
            if gitignore_path.exists() {
                builder.add(&gitignore_path);
            }
        }

        let matcher = builder.build()?;
        Ok(Self { matcher })
    }

    pub fn is_ignored(&self, real_path: &Path) -> anyhow::Result<bool> {
        let is_dir = real_path.is_dir();
        Ok(self.matcher.matched(real_path, is_dir).is_ignore())
    }
}
