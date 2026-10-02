use super::{edit, err};
use klyndb_driver_api::*;
use mysql_async::{Conn, consts::StatusFlags, prelude::Queryable};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

pub(super) async fn apply(
    conn: &mut Conn,
    table: &Table,
    mut input: mpsc::Receiver<Result<InsertBatch>>,
    cancel: &CancellationToken,
    committing: &AtomicBool,
    poison: &AtomicBool,
    interruptible: &AtomicBool,
) -> Result<MutationResult> {
    conn.query_drop("SELECT 1").await.map_err(err)?;
    let flags = conn
        .last_ok_packet()
        .map(|p| p.status_flags())
        .unwrap_or_default();
    let active = flags.contains(StatusFlags::SERVER_STATUS_IN_TRANS);
    let own = !active && flags.contains(StatusFlags::SERVER_STATUS_AUTOCOMMIT);
    if !active {
        conn.query_drop("START TRANSACTION").await.map_err(err)?;
    }
    let savepoint = format!("klyndb_import_{}", uuid::Uuid::new_v4().simple());
    let mut has_savepoint = false;
    let result = async {
        conn.query_drop(format!("SAVEPOINT {savepoint}"))
            .await
            .map_err(err)?;
        has_savepoint = true;
        let mut affected = 0;
        loop {
            interruptible.store(false, Ordering::Relaxed);
            match next_insert_batch(&mut input, cancel).await? {
                InsertBatch::Rows(changes) => {
                    validate_insert_batch(&changes)?;
                    interruptible.store(true, Ordering::Relaxed);
                    let batch = edit::apply(
                        conn,
                        table,
                        &changes,
                        cancel,
                        committing,
                        poison,
                        Some(interruptible),
                    )
                    .await;
                    interruptible.store(false, Ordering::Relaxed);
                    affected += batch?.affected;
                }
                InsertBatch::Complete => break,
            }
        }
        if cancel.is_cancelled() {
            return Err(Error::new("Import cancelled"));
        }
        committing.store(true, Ordering::Relaxed);
        if own {
            conn.query_drop("COMMIT AND NO CHAIN NO RELEASE")
                .await
                .map_err(err)?;
        } else {
            conn.query_drop(format!("RELEASE SAVEPOINT {savepoint}"))
                .await
                .map_err(err)?;
        }
        Ok(MutationResult {
            affected,
            pending_transaction: !own,
        })
    }
    .await;
    if result.is_err() && !poison.load(Ordering::Relaxed) {
        interruptible.store(false, Ordering::Relaxed);
        let undo = if own {
            "ROLLBACK AND NO CHAIN NO RELEASE".into()
        } else if has_savepoint {
            format!("ROLLBACK TO SAVEPOINT {savepoint}")
        } else {
            return result;
        };
        let rollback_ok = conn.query_drop(undo).await.is_ok() && conn.get_warnings() == 0;
        let release_ok = own
            || (rollback_ok
                && conn
                    .query_drop(format!("RELEASE SAVEPOINT {savepoint}"))
                    .await
                    .is_ok());
        if !rollback_ok || !release_ok {
            poison.store(true, Ordering::Relaxed);
            return Err(Error::new(
                "Import rollback could not be confirmed; connection closed. Verify data before retrying; nontransactional trigger effects may remain.",
            ));
        }
    }
    result
}
