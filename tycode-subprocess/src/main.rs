use std::env;
use tokio::task::LocalSet;
use tycode_subprocess::run_subprocess;

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = env::args().collect();
    let mut workspace_roots: Vec<String> = vec![];
    let mut settings_path: Option<String> = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--workspace-roots" => {
                i += 1;
                if i < args.len() {
                    workspace_roots = serde_json::from_str(&args[i])?;
                }
            }
            "--settings-path" => {
                i += 1;
                if i < args.len() {
                    settings_path = Some(args[i].clone());
                }
            }
            _ => {}
        }
        i += 1;
    }

    let local = LocalSet::new();
    local
        .run_until(run_subprocess(workspace_roots, settings_path))
        .await?;
    Ok(())
}
