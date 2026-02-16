use std::path::Path;
use std::process::Stdio;

use anyhow::{bail, Result};
use tokio::process::Command;

pub struct Hunk {
    pub file_path: String,
    pub header: String,
    pub content: String,
}

pub async fn git_diff(workspace_root: &Path) -> Result<String> {
    run_git_diff(workspace_root, &["diff"]).await
}

pub async fn git_diff_expanded(workspace_root: &Path, context_lines: usize) -> Result<String> {
    let context_arg = format!("-U{context_lines}");
    run_git_diff(workspace_root, &["diff", &context_arg]).await
}

async fn run_git_diff(workspace_root: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(workspace_root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "git diff failed with exit code {}: {stderr}",
            output.status.code().unwrap_or(-1)
        );
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

pub fn parse_hunks(diff: &str) -> Vec<Hunk> {
    if diff.is_empty() {
        return Vec::new();
    }

    let file_sections: Vec<&str> = diff.split("diff --git ").collect();
    let mut hunks = Vec::new();

    for section in file_sections.iter().skip(1) {
        let file_path = parse_file_path(section);
        let file_header = extract_file_header(section);
        let hunk_parts: Vec<&str> = section.split("\n@@").collect();

        if hunk_parts.len() < 2 {
            continue;
        }

        for hunk_text in hunk_parts.iter().skip(1) {
            let header = match hunk_text.find(" @@") {
                Some(end) => format!("@@{}", &hunk_text[..end + 3]),
                None => continue,
            };

            let content = format!("{file_header}\n@@{hunk_text}");

            hunks.push(Hunk {
                file_path: file_path.clone(),
                header,
                content,
            });
        }
    }

    hunks
}

fn parse_file_path(section: &str) -> String {
    let first_line = section.lines().next().unwrap_or("");
    first_line
        .split(" b/")
        .nth(1)
        .unwrap_or(first_line)
        .to_string()
}

fn extract_file_header(section: &str) -> String {
    let mut header_lines = Vec::new();
    for line in section.lines() {
        if line.starts_with("--- ") || line.starts_with("+++ ") {
            header_lines.push(line);
        }
        if line.starts_with("+++ ") {
            break;
        }
    }
    header_lines.join("\n")
}
