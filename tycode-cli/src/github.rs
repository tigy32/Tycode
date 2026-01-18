use anyhow::{Context, Result};
use serde::Deserialize;
use std::process::Command;

#[derive(Debug, Deserialize)]
pub struct Issue {
    pub number: u32,
    pub title: String,
    pub body: String,
}

pub fn fetch_issue(issue_number: u32) -> Result<Issue> {
    let output = Command::new("gh")
        .args([
            "issue",
            "view",
            &issue_number.to_string(),
            "--json",
            "number,title,body",
        ])
        .output()
        .context("Failed to execute gh command - ensure gh CLI is installed and authenticated")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("gh issue view failed: {}", stderr);
    }

    let issue: Issue =
        serde_json::from_slice(&output.stdout).context("Failed to parse issue JSON from gh CLI")?;

    Ok(issue)
}

pub fn ensure_clean_working_tree() -> Result<()> {
    let output = Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .context("Failed to execute git status")?;

    if !output.status.success() {
        anyhow::bail!("git status failed");
    }

    let status = String::from_utf8_lossy(&output.stdout);
    if !status.trim().is_empty() {
        anyhow::bail!(
            "Working tree has uncommitted changes. Please commit or stash them before using --auto-pr"
        );
    }

    Ok(())
}

pub fn create_branch(issue_number: u32) -> Result<String> {
    let branch_name = format!("tycode-issue-{}", issue_number);

    let output = Command::new("git")
        .args(["checkout", "-b", &branch_name])
        .output()
        .context("Failed to create branch")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git checkout -b failed: {}", stderr);
    }

    Ok(branch_name)
}

pub fn commit_changes(message: &str) -> Result<()> {
    let add_output = Command::new("git")
        .args(["add", "-A"])
        .output()
        .context("Failed to stage changes")?;

    if !add_output.status.success() {
        let stderr = String::from_utf8_lossy(&add_output.stderr);
        anyhow::bail!("git add failed: {}", stderr);
    }

    let commit_output = Command::new("git")
        .args(["commit", "-m", message])
        .output()
        .context("Failed to commit changes")?;

    if !commit_output.status.success() {
        let stderr = String::from_utf8_lossy(&commit_output.stderr);
        anyhow::bail!("git commit failed: {}", stderr);
    }

    Ok(())
}

pub fn push_branch() -> Result<()> {
    let output = Command::new("git")
        .args(["push", "-u", "origin", "HEAD"])
        .output()
        .context("Failed to push branch")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git push failed: {}", stderr);
    }

    Ok(())
}

pub fn create_pr(issue_number: u32, title: &str, body: &str, draft: bool) -> Result<String> {
    let head_branch = format!("tycode-issue-{}", issue_number);

    let mut command = Command::new("gh");
    command.args([
        "pr",
        "create",
        "--title",
        title,
        "--body",
        body,
        "--head",
        &head_branch,
    ]);

    if draft {
        command.arg("--draft");
    }

    let output = command.output().context("Failed to create PR")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("gh pr create failed: {}", stderr);
    }

    let pr_url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(pr_url)
}
