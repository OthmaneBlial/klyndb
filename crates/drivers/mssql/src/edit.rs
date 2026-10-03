use super::{NativeClient, catalog_rows, err, identifier, object_id};
use klyndb_driver_api::*;
use std::time::Duration;
use tiberius::Query;
use tokio::time::Instant;

fn bind(query: &mut Query<'static>, value: &Cell) -> Result<()> {
    match value {
        Cell::Null => query.bind(None::<String>),
        Cell::Text(s) => query.bind(s.clone()),
        Cell::Number(s) => {
            normalized_number(s)?;
            // Bind original text; SQL Server performs the destination conversion without f64 rounding.
            query.bind(s.clone());
        }
        Cell::Boolean(b) => query.bind(*b),
        Cell::Binary(s) => {
            query.bind(hex::decode(s).map_err(|_| Error::new("Binary values must be hexadecimal"))?)
        }
        Cell::Json(v) => query.bind(v.to_string()),
    }
    Ok(())
}
fn normalized_number(s: &str) -> Result<(String, i64)> {
    let invalid = || Error::new("Enter a valid numeric literal");
    let s = serde_json::from_str::<serde_json::Number>(s)
        .map_err(|_| invalid())?
        .to_string();
    let (mantissa, exponent) = s.split_once(['e', 'E']).unwrap_or((&s, "0"));
    let exponent = exponent.parse::<i64>().map_err(|_| invalid())?;
    let negative = mantissa.starts_with('-');
    let mantissa = mantissa.trim_start_matches('-');
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let digits = format!("{whole}{fraction}");
    let digits = digits.trim_start_matches('0');
    if digits.is_empty() {
        return Ok(("0".into(), 0));
    }
    let significant = digits.trim_end_matches('0');
    let scale = (fraction.len() as i64)
        .checked_sub(exponent)
        .and_then(|n| n.checked_sub((digits.len() - significant.len()) as i64))
        .ok_or_else(invalid)?;
    Ok((
        format!("{}{significant}", if negative { "-" } else { "" }),
        scale,
    ))
}
async fn run(
    client: &mut NativeClient,
    query: Query<'static>,
    deadline: Instant,
    poison: &mut bool,
) -> Result<Vec<Row>> {
    let result = tokio::time::timeout_at(deadline, async {
        let stream = query.query(client).await.map_err(err)?;
        catalog_rows(stream).await
    })
    .await
    .unwrap_or_else(|_| Err(Error::new("Table editing timed out")));
    recover(client, result.is_err(), poison).await;
    result
}
async fn recover(client: &mut NativeClient, failed: bool, poison: &mut bool) {
    if failed
        && !matches!(
            tokio::time::timeout(Duration::from_secs(3), client.cancel_query()).await,
            Ok(Ok(()))
        )
    {
        *poison = true;
    }
}
async fn command(
    client: &mut NativeClient,
    sql: String,
    deadline: Instant,
    poison: &mut bool,
) -> Result<()> {
    // Session SET/transaction commands must be batches: sp_executesql scopes SET
    // options and reports error 266 when BEGIN changes its entry transaction count.
    let result = tokio::time::timeout_at(deadline, async {
        let stream = client.simple_query(sql).await.map_err(err)?;
        catalog_rows(stream).await.map(|_| ())
    })
    .await
    .unwrap_or_else(|_| Err(Error::new("Table editing timed out")));
    recover(client, result.is_err(), poison).await;
    result
}
pub(super) fn column_sql(table: &Table) -> String {
    format!(
        "SELECT TOP (50001) c.name,CASE WHEN t.is_user_defined=1 THEN QUOTENAME(SCHEMA_NAME(t.schema_id))+N'.'+QUOTENAME(t.name) WHEN t.name IN ('decimal','numeric') THEN t.name+N'('+CONVERT(nvarchar(3),c.precision)+N','+CONVERT(nvarchar(3),c.scale)+N')' WHEN t.name IN ('varchar','char','varbinary','binary','nvarchar','nchar') THEN t.name+N'('+CASE WHEN c.max_length=-1 THEN N'max' ELSE CONVERT(nvarchar(5),CASE WHEN t.name IN ('nvarchar','nchar') THEN c.max_length/2 ELSE c.max_length END) END+N')' WHEN t.name IN ('datetime2','datetimeoffset','time') THEN t.name+N'('+CONVERT(nvarchar(3),c.scale)+N')' ELSE t.name END,c.is_nullable,CAST(CASE WHEN EXISTS(SELECT 1 FROM sys.indexes i JOIN sys.index_columns ic ON ic.object_id=i.object_id AND ic.index_id=i.index_id WHERE i.object_id=c.object_id AND i.is_primary_key=1 AND ic.column_id=c.column_id) THEN 1 ELSE 0 END AS bit),OBJECT_DEFINITION(c.default_object_id),CAST(CASE WHEN c.is_identity=1 OR c.is_computed=1 OR c.generated_always_type<>0 OR t.name='timestamp' THEN 1 ELSE 0 END AS bit) FROM sys.columns c JOIN sys.types t ON t.user_type_id=c.user_type_id WHERE c.object_id={} ORDER BY c.column_id",
        object_id(table)
    )
}
pub(super) fn decode_columns(rows: Vec<Row>) -> Vec<Column> {
    rows.into_iter()
        .map(|r| Column {
            name: r[0].text(),
            data_type: r[1].text(),
            nullable: r[2].text() == "true",
            primary_key: r[3].text() == "true",
            default: if matches!(r[4], Cell::Null) {
                None
            } else {
                Some(r[4].text())
            },
            generated: r[5].text() == "true",
        })
        .collect()
}
pub(super) fn editable_sql(table: &Table) -> String {
    format!(
        "SELECT CAST(CASE WHEN EXISTS(SELECT 1 FROM sys.tables t WHERE t.object_id={} AND t.is_ms_shipped=0 AND t.is_memory_optimized=0 AND NOT EXISTS(SELECT 1 FROM sys.triggers tr WHERE tr.parent_id=t.object_id AND tr.is_instead_of_trigger=1 AND tr.is_disabled=0)) THEN 1 ELSE 0 END AS bit)",
        object_id(table)
    )
}
pub(super) fn decode_editable(rows: Vec<Row>) -> bool {
    rows.first()
        .and_then(|r| r.first())
        .is_some_and(|c| c == &Cell::Boolean(true))
}
fn conversion_matches(original: &Cell, converted: &Cell, ty: &str) -> Result<bool> {
    if matches!(original, Cell::Null) {
        return Ok(matches!(converted, Cell::Null));
    }
    if let Cell::Number(actual) = converted {
        let expected = match original {
            Cell::Boolean(b) => {
                if *b {
                    "1".into()
                } else {
                    "0".into()
                }
            }
            _ => original.text(),
        };
        return Ok(normalized_number(&expected)? == normalized_number(actual)?);
    }
    if let Cell::Boolean(actual) = converted {
        return Ok(match original {
            Cell::Boolean(expected) => expected == actual,
            _ => {
                normalized_number(&original.text())?
                    == normalized_number(if *actual { "1" } else { "0" })?
            }
        });
    }
    if let Cell::Binary(actual) = converted {
        let expected = match original {
            Cell::Binary(s) => hex::decode(s).map_err(|_| Error::new("Invalid binary value"))?,
            _ => return Ok(false),
        };
        let actual = hex::decode(actual).map_err(|_| Error::new("Invalid native binary value"))?;
        return Ok(actual == expected
            || (ty.starts_with("binary(")
                && actual.starts_with(&expected)
                && actual[expected.len()..].iter().all(|b| *b == 0)));
    }
    if matches!(
        ty.split('(').next(),
        Some("varchar" | "nvarchar" | "text" | "ntext" | "char" | "nchar")
    ) {
        let expected = original.text();
        let actual = converted.text();
        return Ok(if ty.starts_with("char(") || ty.starts_with("nchar(") {
            expected.trim_end_matches(' ') == actual.trim_end_matches(' ')
        } else {
            expected == actual
        });
    }
    Ok(true)
}
pub(super) async fn apply(
    client: &mut NativeClient,
    table: &Table,
    changes: &[Change],
    poison: &mut bool,
) -> Result<MutationResult> {
    let deadline = Instant::now() + Duration::from_secs(60);
    let state = run(
        client,
        Query::new("SELECT @@TRANCOUNT,XACT_STATE(),@@OPTIONS"),
        deadline,
        poison,
    )
    .await?;
    let state = state
        .first()
        .filter(|r| r.len() == 3)
        .ok_or_else(|| Error::new("Transaction state is unavailable"))?;
    let count = state[0]
        .text()
        .parse::<u32>()
        .map_err(|_| Error::new("Invalid transaction state"))?;
    if state[1].text() == "-1" {
        return Err(Error::new(
            "Roll back the failed transaction before editing",
        ));
    }
    let options = state[2]
        .text()
        .parse::<u32>()
        .map_err(|_| Error::new("Invalid session options"))?;
    let own = count == 0 && options & 2 == 0;
    let savepoint = format!("k{}", &uuid::Uuid::new_v4().simple().to_string()[..30]);
    let restore = format!(
        "SET IMPLICIT_TRANSACTIONS {}; SET ANSI_WARNINGS {}; SET ARITHABORT {}",
        if options & 2 != 0 { "ON" } else { "OFF" },
        if options & 8 != 0 { "ON" } else { "OFF" },
        if options & 64 != 0 { "ON" } else { "OFF" }
    );
    let mut saved = false;
    let result = async {
        command(client,"SET IMPLICIT_TRANSACTIONS OFF; SET ANSI_WARNINGS ON; SET ARITHABORT ON".into(),deadline,poison).await?;
        if count==0 {command(client,"BEGIN TRANSACTION".into(),deadline,poison).await?;}
        command(client,format!("SAVE TRANSACTION {savepoint}"),deadline,poison).await?;saved=true;
        let qualified=format!("{}.{}",identifier(&table.schema),identifier(&table.name));
        // ponytail: one table-wide lock protects metadata; use narrower locks after a verified schema-change guard exists.
        command(client,format!("SELECT TOP (1) 1 FROM {qualified} WITH (TABLOCKX,HOLDLOCK)"),deadline,poison).await?;
        if !decode_editable(run(client,Query::new(editable_sql(table)),deadline,poison).await?) {return Err(Error::new("Only disk-based base tables without enabled INSTEAD OF triggers can be edited"));}
        let columns=decode_columns(run(client,Query::new(column_sql(table)),deadline,poison).await?);
        for change in changes {change.validate(&columns)?;}
        for change in changes {
            if change.values().map_or(0,|v|v.len()) + if change.old().is_some() {columns.iter().filter(|c|c.primary_key).count()} else {0} > Query::MAX_PARAMETERS {return Err(Error::new("Editing one row exceeds SQL Server's parameter limit"));}
            let mut keys=vec![];
            if let Some(old)=change.old() {
                for (c,_) in columns.iter().zip(old).filter(|(c,_)|c.primary_key) {keys.push(format!("{}=@P{}",identifier(&c.name),keys.len()+1));}
                let mut q=Query::new(format!("SELECT TOP (2) * FROM {qualified} WITH (UPDLOCK,HOLDLOCK) WHERE {}",keys.join(" AND ")));
                for (_,v) in columns.iter().zip(old).filter(|(c,_)|c.primary_key) {bind(&mut q,v)?;}
                let current=run(client,q,deadline,poison).await?;
                if current.len()!=1 || &current[0]!=old {return Err(Error::new("Row changed or was removed. Refresh the table; the batch was aborted."));}
            }
            let values=change.values();
            let mut names=vec![]; let mut expressions=vec![];
            if let Some(values)=values {
                for name in values.keys() {
                    let c=columns.iter().find(|c|&c.name==name).ok_or_else(||Error::new("Column changed"))?;
                    names.push(identifier(name));expressions.push(format!("CAST(@P{} AS {})",expressions.len()+1,c.data_type));
                }
                if !expressions.is_empty() {
                    let mut q=Query::new(format!("SELECT {}",expressions.join(",")));
                    for value in values.values() {bind(&mut q,value)?;}
                    let converted=run(client,q,deadline,poison).await?;
                    let converted=converted.first().filter(|r|r.len()==values.len()).ok_or_else(||Error::new("Native value conversion is unavailable"))?;
                    for ((name,value),converted) in values.iter().zip(converted) {
                        let c=columns.iter().find(|c|&c.name==name).ok_or_else(||Error::new("Column changed"))?;
                        if !conversion_matches(value,converted,&c.data_type)? {return Err(Error::new("A value loses precision or data in its destination type; the batch was aborted."));}
                    }
                }
            }
            let output=" OUTPUT 1 INTO @klyndb_changed";
            let sql=match change {
                Change::Insert {values} if values.is_empty()=>format!("INSERT INTO {qualified}{output} DEFAULT VALUES"),
                Change::Insert {..}=>format!("INSERT INTO {qualified} ({}){output} VALUES ({})",names.join(","),expressions.join(",")),
                Change::Update {..}=>format!("UPDATE {qualified} SET {}{output}",names.iter().zip(&expressions).map(|(n,e)|format!("{n}={e}")).collect::<Vec<_>>().join(",")),
                Change::Delete {..}=>format!("DELETE FROM {qualified}{output}"),
            };
            let mut predicates=vec![];
            if let Some(old)=change.old() {
                for (c,_) in columns.iter().zip(old).filter(|(c,_)|c.primary_key) {predicates.push(format!("{}=@P{}",identifier(&c.name),expressions.len()+predicates.len()+1));}
            }
            let sql=if predicates.is_empty() {sql} else {format!("{sql} WHERE {}",predicates.join(" AND "))};
            let mut q=Query::new(format!("DECLARE @klyndb_changed TABLE(n int); {sql}; SELECT COUNT_BIG(*) FROM @klyndb_changed"));
            if let Some(values)=values {for value in values.values() {bind(&mut q,value)?;}}
            if let Some(old)=change.old() {for (_,v) in columns.iter().zip(old).filter(|(c,_)|c.primary_key) {bind(&mut q,v)?;}}
            let rows=run(client,q,deadline,poison).await?;
            if rows.last()!=Some(&vec![Cell::Number("1".into())]) {return Err(Error::new("Native editing did not affect exactly one row; the batch was aborted."));}
        }
        let state=run(client,Query::new("SELECT @@TRANCOUNT,XACT_STATE()"),deadline,poison).await?;
        if state.first()!=Some(&vec![Cell::Number(count.max(1).to_string()),Cell::Number("1".into())]) {return Err(Error::new("The transaction changed during editing; verify trigger behavior"));}
        command(client,restore.clone(),deadline,poison).await?;
        if own {
            // Never interrupt COMMIT: an unknown acknowledgement closes the session and requires verification.
            let commit=async {
                let stream=client.simple_query("COMMIT TRANSACTION").await.map_err(err)?;
                catalog_rows(stream).await.map(|_| ())
            };
            match tokio::time::timeout(Duration::from_secs(3),commit).await {
                Ok(Ok(_))=>(),_=>{*poison=true;return Err(Error::new("Commit could not be confirmed. Connection closed; verify writes before retrying."));}
            }
        }
        Ok(MutationResult {affected:changes.len() as u64,pending_transaction:!own})
    }.await;
    if result.is_err() && !*poison {
        let cleanup = Instant::now() + Duration::from_secs(3);
        let rollback = if count == 0 {
            "IF XACT_STATE()<>0 ROLLBACK TRANSACTION".into()
        } else if saved {
            format!("ROLLBACK TRANSACTION {savepoint}")
        } else {
            String::new()
        };
        if (!rollback.is_empty() && command(client, rollback, cleanup, poison).await.is_err())
            || command(client, restore, cleanup, poison).await.is_err()
        {
            *poison = true;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_numeric_comparison_rejects_loss_without_float_conversion() {
        for (left, right) in [
            ("1000.00", "1e3"),
            ("-0.0", "0"),
            ("1e-18", "0.000000000000000001"),
        ] {
            assert_eq!(
                normalized_number(left).unwrap(),
                normalized_number(right).unwrap()
            );
        }
        assert_ne!(
            normalized_number("9223372036854775807").unwrap(),
            normalized_number("9223372036854775806").unwrap()
        );
        assert_ne!(
            normalized_number("0.1234567890123456789").unwrap(),
            normalized_number("0.123456789012345679").unwrap()
        );
        for invalid in [
            "NaN",
            "Infinity",
            "1; DROP TABLE x",
            "1e9999999999999999999999",
        ] {
            assert!(normalized_number(invalid).is_err());
        }
    }
}
