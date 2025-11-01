use std::path::{Path, PathBuf};
use std::process::Command;

pub struct Ignored {
    root: PathBuf,
}

impl Ignored {
    pub fn new(root: &Path) -> anyhow::Result<Self> {
        Ok(Self {
            root: root.to_path_buf(),
        })
    }

    pub fn is_ignored(&self, real_path: &Path) -> anyhow::Result<bool> {
        let working_dir = real_path.parent().unwrap_or(&self.root);

        let output = match Command::new("git")
            .arg("check-ignore")
            .arg("-q")
            .arg(real_path)
            .current_dir(working_dir)
            .output()
        {
            Ok(output) => output,
            Err(_) => return Ok(false),
        };

        Ok(output.status.code() == Some(0))
    }
}
