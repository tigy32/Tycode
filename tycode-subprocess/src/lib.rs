use anyhow::anyhow;
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::task::JoinSet;
use tokio::{io, io::AsyncWriteExt};
use tycode_core::chat::actor::ChatActor;
use tycode_core::chat::ChatActorMessage;

// SubprocessApp logic adapted
pub async fn run_subprocess(workspace_roots: Vec<String>) -> anyhow::Result<()> {
    let (chat_actor, mut event_rx) = ChatActor::launch(
        workspace_roots.into_iter().map(PathBuf::from).collect(),
        None,
    );

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
