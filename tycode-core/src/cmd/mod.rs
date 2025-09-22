use std::{path::PathBuf, process::Stdio, time::Duration};

use serde::Serialize;
use tokio::process::Command;

#[derive(Debug, Clone, Serialize)]
pub struct CommandResult {
    pub code: i32,
    pub out: String,
    pub err: String,
}

pub async fn run_cmd(
    dir: PathBuf,
    cmd: String,
    timeout: Duration,
) -> anyhow::Result<CommandResult> {
    let parts: Vec<_> = cmd.split(" ").collect();
    let (program, args) = (parts[0], &parts[1..]);

    // Spawn the command as a child process
    let child = Command::new(program)
        .args(args)
        .current_dir(&dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true) // Ensure process is killed when dropped
        .spawn()?;

    // Try to get output with timeout
    let output = tokio::time::timeout(timeout, async {
        let output = child.wait_with_output().await?;
        Ok::<_, std::io::Error>(output)
    })
    .await??;

    let code = output.status.code().unwrap_or(1);
    let out = String::from_utf8_lossy(&output.stdout).to_string();
    let err = String::from_utf8_lossy(&output.stderr).to_string();

    Ok(CommandResult { code, out, err })
}
