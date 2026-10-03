use super::{NativeClient, catalog, send, stream};
use klyndb_driver_api::*;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

async fn command(
    client: &mut NativeClient,
    sql: &str,
    cancel: &CancellationToken,
) -> Result<Vec<Row>> {
    tokio::select! {biased; _=cancel.cancelled()=>Err(Error::new("Query cancelled")), result=catalog(client,sql.into())=>result}
}
async fn synchronized(client: &mut NativeClient) -> bool {
    matches!(
        tokio::time::timeout(Duration::from_secs(3), client.cancel_query()).await,
        Ok(Ok(()))
    )
}
fn closed(error: Error) -> (Result<()>, bool) {
    (
        Err(Error::new(format!(
            "{} Plan settings or interruption could not be confirmed; connection closed. Reconnect and verify writes and the original transaction before retrying.",
            error.message
        ))),
        false,
    )
}
pub(super) async fn execute(
    client: &mut NativeClient,
    sql: &str,
    out: &mpsc::Sender<Batch>,
    cancel: &CancellationToken,
    limit: usize,
    analyze: bool,
) -> (Result<()>, bool) {
    // A native aggregate over VALUES emits a profile without opening an implicit table transaction.
    let probe = command(
        client,
        "SELECT SUM(v.n) FROM (VALUES(1),(2)) AS v(n)",
        cancel,
    )
    .await;
    let probe = match probe {
        Ok(rows) => rows,
        Err(error) => {
            return if synchronized(client).await {
                (Err(error), true)
            } else {
                closed(error)
            };
        }
    };
    if probe.len() != 1
        || probe[0].len() != 1
        || !matches!(&probe[0][0],Cell::Number(n) if n.parse::<u32>().is_ok())
    {
        return (
            Err(Error::new(
                "Disable existing SHOWPLAN/STATISTICS PROFILE/XML settings before requesting a plan",
            )),
            true,
        );
    }
    let option = if analyze {
        "STATISTICS PROFILE"
    } else {
        "SHOWPLAN_ALL"
    };
    // Session settings are separate native batches; SHOWPLAN must be alone in its batch.
    let result = async {
        command(client, &format!("SET {option} ON"), cancel).await?;
        // Drain surplus data rows so runtime profiling completes; retained result sets remain bounded.
        tokio::select! {biased;
            _=cancel.cancelled()=>Err(Error::new("Query cancelled")),
            _=out.closed()=>Err(Error::new("Result consumer closed")),
            result=stream(client,sql,out,limit,true)=>result,
        }
    }
    .await;
    if let Err(error) = &result
        && !synchronized(client).await
    {
        return closed(Error::new(error.message.clone()));
    }
    // Always restore before reuse, with an uncancelled cleanup request. Runtime writes are not rolled back.
    let cleanup_cancel = CancellationToken::new();
    let cleanup_sql = format!("SET {option} OFF");
    let cleanup = command(client, &cleanup_sql, &cleanup_cancel);
    if !matches!(
        tokio::time::timeout(Duration::from_secs(3), cleanup).await,
        Ok(Ok(_))
    ) {
        return closed(
            result
                .err()
                .unwrap_or_else(|| Error::new("Could not restore plan settings")),
        );
    }
    match result {
        Err(error) => (Err(error), true),
        Ok(truncated) => (
            send(
                out,
                Batch::Complete {
                    affected: 0,
                    truncated,
                },
            )
            .await,
            true,
        ),
    }
}
