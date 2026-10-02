use super::err;
use klyndb_driver_api::*;
use tokio_postgres::{Client, error::SqlState, types::ToSql};

fn text(cell: &Cell) -> Option<String> {
    match cell {
        Cell::Null => None,
        Cell::Binary(v) => Some(format!("\\x{v}")),
        _ => Some(cell.text()),
    }
}

pub(super) async fn apply(
    client: &Client,
    table: &Table,
    changes: &[Change],
    cancel: &tokio_util::sync::CancellationToken,
    interruptible: Option<&std::sync::atomic::AtomicBool>,
) -> Result<MutationResult> {
    // SAVEPOINT distinguishes an existing user transaction without BEGIN accidentally committing it later.
    let own_transaction = match client.batch_execute("SAVEPOINT klyndb_edit").await {
        Ok(()) => false,
        Err(e) if e.code() == Some(&SqlState::NO_ACTIVE_SQL_TRANSACTION) => {
            client
                .batch_execute("BEGIN; SAVEPOINT klyndb_edit")
                .await
                .map_err(err)?;
            true
        }
        Err(e) => return Err(err(e)),
    };
    let batch=async{
        let old_timeout:String=client.query_one("SELECT current_setting('statement_timeout')",&[]).await.map_err(err)?.get(0);
        client.batch_execute("SET LOCAL statement_timeout='60s'").await.map_err(err)?;
        let qualified=format!("{}.{}",quote_identifier(&table.schema),quote_identifier(&table.name));
        let kind:Option<String>=client.query_opt("SELECT c.relkind::text FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1 AND c.relname=$2",&[&table.schema,&table.name]).await.map_err(err)?.map(|r|r.get(0));
        if !matches!(kind.as_deref(),Some("r"|"p")){return Err(Error::new("Only base tables can be edited"));}
        client.batch_execute(&format!("LOCK TABLE {qualified} IN ROW EXCLUSIVE MODE")).await.map_err(err)?;
        let rows=client.query("SELECT a.attname,pg_catalog.format_type(a.atttypid,a.atttypmod),NOT a.attnotnull,EXISTS(SELECT 1 FROM pg_index i WHERE i.indrelid=a.attrelid AND i.indisprimary AND a.attnum=ANY(i.indkey)),(a.attgenerated!='' OR a.attidentity='a') FROM pg_attribute a WHERE a.attrelid=$1::pg_catalog.text::regclass AND a.attnum>0 AND NOT a.attisdropped ORDER BY a.attnum",&[&qualified]).await.map_err(err)?;
        let columns:Vec<Column>=rows.iter().map(|r|Column{name:r.get(0),data_type:r.get(1),nullable:r.get(2),primary_key:r.get(3),generated:r.get(4),default:None}).collect();
        for change in changes {change.validate(&columns)?;}
        let mut affected=0;
        for change in changes {
            if cancel.is_cancelled() { return Err(Error::new("Import cancelled")); }
            let mut params:Vec<Option<String>>=vec![];
            let mut names=vec![];
            let mut bindings=vec![];
            if let Some(values)=change.values(){
                for (name,cell) in values {
                    let column=columns.iter().find(|c|&c.name==name).ok_or_else(||Error::new("Column changed"))?;
                    params.push(text(cell));
                    names.push(quote_identifier(name));
                    // Both the identifier and cast type come from validated server metadata; values are always bound.
                    bindings.push(format!("(${}::pg_catalog.text)::{}",params.len(),column.data_type));
                }
            }
            let sql=match change {
                Change::Insert{values} if values.is_empty()=>format!("INSERT INTO {qualified} DEFAULT VALUES"),
                Change::Insert{..}=>format!("INSERT INTO {qualified} ({}) VALUES ({})",names.join(","),bindings.join(",")),
                Change::Update{..}=>format!("UPDATE {qualified} SET {}",names.iter().zip(&bindings).map(|(n,b)|format!("{n}={b}")).collect::<Vec<_>>().join(",")),
                Change::Delete{..}=>format!("DELETE FROM {qualified}"),
            };
            let sql=if let Some(old)=change.old(){
                let predicates=columns.iter().zip(old).map(|(c,v)|{
                    params.push(text(v));
                    format!("({}::pg_catalog.text COLLATE \"C\") IS NOT DISTINCT FROM ${}::pg_catalog.text",quote_identifier(&c.name),params.len())
                }).collect::<Vec<_>>();
                format!("{sql} WHERE {}",predicates.join(" AND "))
            }else{sql};
            let refs:Vec<&(dyn ToSql+Sync)>=params.iter().map(|v|v as &(dyn ToSql+Sync)).collect();
            let count=client.execute(&sql,&refs).await.map_err(err)?;
            if count!=1{return Err(Error::new("Row changed or was removed. Refresh the table; no changes in this batch were applied."));}
            affected+=count;
        }
        client.query_one("SELECT set_config('statement_timeout',$1,true)",&[&old_timeout]).await.map_err(err)?;
        client.batch_execute("RELEASE SAVEPOINT klyndb_edit").await.map_err(err)?;
        if own_transaction {client.batch_execute("COMMIT").await.map_err(err)?;}
        Ok(MutationResult{affected,pending_transaction:!own_transaction})
    }.await;
    if batch.is_err() {
        if let Some(flag) = interruptible {
            flag.store(false, std::sync::atomic::Ordering::Relaxed);
        }
        let rollback = if own_transaction {
            "ROLLBACK"
        } else {
            "ROLLBACK TO SAVEPOINT klyndb_edit; RELEASE SAVEPOINT klyndb_edit"
        };
        if let Err(e) = client.batch_execute(rollback).await {
            return Err(Error::new(format!(
                "Editing failed and rollback could not be verified ({}). Disconnect before further writes.",
                err(e)
            )));
        }
    }
    batch
}
