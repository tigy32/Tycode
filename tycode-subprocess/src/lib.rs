use anyhow::anyhow;
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::task::JoinSet;
use tokio::{io, io::AsyncWriteExt};
use tycode_core::chat::actor::ChatActorBuilder;
use tycode_core::chat::ChatActorMessage;
use tycode_core::settings::config::McpServerConfig;

pub async fn run_subprocess(
    workspace_roots: Vec<String>,
    mcp_servers: HashMap<String, McpServerConfig>,
    ephemeral: bool,
) -> anyhow::Result<()> {
    let workspace_roots: Vec<PathBuf> = workspace_roots.into_iter().map(PathBuf::from).collect();

    let mut builder = ChatActorBuilder::tycode(workspace_roots, None, None)?;
    if !mcp_servers.is_empty() {
        builder = builder.with_extra_mcp_servers(mcp_servers);
    }
    if ephemeral {
        builder = builder.ephemeral();
    }
    let (chat_actor, mut event_rx) = builder.build()?;

    let mut join_set: JoinSet<anyhow::Result<()>> = JoinSet::new();

    join_set.spawn(async move {
        let mut stdout = io::stdout();
        while let Some(message) = event_rx.recv().await {
            let json = serde_json::to_string(&message)?;
            let json = format!("{json}\n");
            stdout.write_all(json.as_bytes()).await?;
        }
        Ok(())
    });

    join_set.spawn(async move {
        let mut stdin = BufReader::new(io::stdin()).lines();
        while let Some(line) = stdin.next_line().await? {
            if line == "CANCEL" {
                chat_actor.cancel()?;
                continue;
            }
            let message: ChatActorMessage = serde_json::from_str(&line)?;
            chat_actor.tx.send(message)?;
        }
        Ok(())
    });

    if let Some(result) = join_set.join_next().await {
        return match result {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => Err(anyhow!(e)),
            Err(panic) => Err(anyhow!(panic)),
        };
    }
    Ok(())
}
