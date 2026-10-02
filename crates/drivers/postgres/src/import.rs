use super::{edit, err};
use klyndb_driver_api::*;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc;
use tokio_postgres::{Client, error::SqlState};
use tokio_util::sync::CancellationToken;

pub(super) async fn apply(
    client: &Client,
    table: &Table,
    mut input: mpsc::Receiver<Result<InsertBatch>>,
    cancel: &CancellationToken,
    committing: &AtomicBool,
    poison: &AtomicBool,
    interruptible: &AtomicBool,
) -> Result<MutationResult> {
    let savepoint = format!("klyndb_import_{}", uuid::Uuid::new_v4().simple());
    let own = match client
        .batch_execute(&format!("SAVEPOINT {savepoint}"))
        .await
    {
        Ok(()) => false,
        Err(e) if e.code() == Some(&SqlState::NO_ACTIVE_SQL_TRANSACTION) => {
            client.batch_execute("BEGIN").await.map_err(err)?;
            if client
                .batch_execute(&format!("SAVEPOINT {savepoint}"))
                .await
                .is_err()
            {
                if client.batch_execute("ROLLBACK").await.is_err() {
                    poison.store(true, Ordering::Relaxed);
                }
                return Err(Error::new("Could not initialize the import transaction"));
            }
            true
        }
        Err(e) => return Err(err(e)),
    };
    let result = async {
        let mut affected = 0;
        loop {
            interruptible.store(false, Ordering::Relaxed);
            match next_insert_batch(&mut input, cancel).await? {
                InsertBatch::Rows(changes) => {
                    validate_insert_batch(&changes)?;
                    interruptible.store(true, Ordering::Relaxed);
                    let batch =
                        edit::apply(client, table, &changes, cancel, Some(interruptible)).await;
                    interruptible.store(false, Ordering::Relaxed);
                    affected += batch?.affected;
                }
                InsertBatch::Complete => break,
            }
        }
        if cancel.is_cancelled() {
            return Err(Error::new("Import cancelled"));
        }
        // Finalization is not interrupted: a late cancel must not undo an
        // acknowledged import or lose the caller's savepoint after RELEASE.
        committing.store(true, Ordering::Relaxed);
        if own {
            client.batch_execute("COMMIT").await.map_err(err)?;
        } else {
            client
                .batch_execute(&format!("RELEASE SAVEPOINT {savepoint}"))
                .await
                .map_err(err)?;
        }
        Ok(MutationResult {
            affected,
            pending_transaction: !own,
        })
    }
    .await;
    if result.is_err() {
        interruptible.store(false, Ordering::Relaxed);
        let undo = if own {
            "ROLLBACK".into()
        } else {
            format!("ROLLBACK TO SAVEPOINT {savepoint}; RELEASE SAVEPOINT {savepoint}")
        };
        if client.batch_execute(&undo).await.is_err() {
            poison.store(true, Ordering::Relaxed);
            return Err(Error::new(
                "Import rollback could not be confirmed; connection closed. Verify data before retrying.",
            ));
        }
    }
    result
}
