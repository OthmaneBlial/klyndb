use super::*;
use std::collections::HashMap;

pub(super) async fn list(session: &Mysql, search: &str, offset: u32) -> Result<RoutinePage> {
    if search.len() > 1024 || offset > 1_000_000 {
        return Err(Error::new(
            "Routine search is limited to 1024 bytes and 1,000,000 rows of paging",
        ));
    }
    let pattern = format!(
        "%{}%",
        search
            .replace('!', "!!")
            .replace('%', "!%")
            .replace('_', "!_")
    );
    let mut guard = session.connection.lock().await;
    let conn = guard
        .as_mut()
        .ok_or_else(|| Error::new("Connection is closed"))?;
    let rows: Vec<(String, String, String, Option<String>, String)> = conn.exec(
        "SELECT ROUTINE_SCHEMA,ROUTINE_NAME,ROUTINE_TYPE,DTD_IDENTIFIER,ROUTINE_BODY FROM information_schema.ROUTINES WHERE ROUTINE_SCHEMA NOT IN ('mysql','information_schema','performance_schema','sys') AND CONCAT(ROUTINE_SCHEMA,'.',ROUTINE_NAME) LIKE ? ESCAPE '!' ORDER BY ROUTINE_SCHEMA,ROUTINE_NAME,ROUTINE_TYPE LIMIT 101 OFFSET ?",
        (pattern, offset),
    ).await.map_err(err)?;
    let has_more = rows.len() > 100;
    let rows: Vec<_> = rows.into_iter().take(100).collect();
    let mut arguments: HashMap<(String, String, String), Vec<String>> = HashMap::new();
    if !rows.is_empty() {
        // Read only the visible page's parameters, without GROUP_CONCAT's truncation limit.
        let filters =
            vec!["(SPECIFIC_SCHEMA=? AND SPECIFIC_NAME=? AND ROUTINE_TYPE=?)"; rows.len()]
                .join(" OR ");
        let values: Vec<Value> = rows
            .iter()
            .flat_map(|(schema, name, kind, _, _)| {
                [
                    schema.clone().into(),
                    name.clone().into(),
                    kind.clone().into(),
                ]
            })
            .collect();
        let parameters: Vec<(String, String, String, String, String, String)> = conn.exec(
            format!("SELECT SPECIFIC_SCHEMA,SPECIFIC_NAME,ROUTINE_TYPE,PARAMETER_MODE,PARAMETER_NAME,DTD_IDENTIFIER FROM information_schema.PARAMETERS WHERE ORDINAL_POSITION>0 AND ({filters}) ORDER BY SPECIFIC_SCHEMA,SPECIFIC_NAME,ROUTINE_TYPE,ORDINAL_POSITION LIMIT 50001"),
            values,
        ).await.map_err(err)?;
        if parameters.len() > 50_000 {
            return Err(Error::new(
                "Routine page exceeds the 50,000-parameter viewer limit",
            ));
        }
        for (schema, name, kind, mode, parameter, ty) in parameters {
            arguments
                .entry((schema, name, kind))
                .or_default()
                .push(format!(
                    "{mode} {} {ty}",
                    session.quote_identifier(&parameter)
                ));
        }
    }
    Ok(RoutinePage {
        has_more,
        routines: rows
            .into_iter()
            .map(|(schema, name, kind, returns, language)| Routine {
                id: serde_json::json!([&schema, &name, &kind]).to_string(),
                arguments: arguments
                    .remove(&(schema.clone(), name.clone(), kind.clone()))
                    .unwrap_or_default()
                    .join(", "),
                returns: if kind == "FUNCTION" { returns } else { None },
                kind: kind.to_ascii_lowercase(),
                schema,
                name,
                language,
            })
            .collect(),
    })
}

pub(super) async fn definition(session: &Mysql, id: &str) -> Result<String> {
    if id.len() > 2048 {
        return Err(Error::new("Invalid routine identifier"));
    }
    let (schema, name, kind): (String, String, String) =
        serde_json::from_str(id).map_err(|_| Error::new("Invalid routine identifier"))?;
    if schema.is_empty()
        || name.is_empty()
        || schema.len() > 256
        || name.len() > 256
        || !matches!(kind.as_str(), "FUNCTION" | "PROCEDURE")
        || matches!(
            schema.as_str(),
            "mysql" | "information_schema" | "performance_schema" | "sys"
        )
    {
        return Err(Error::new("Invalid routine identifier"));
    }
    let mut guard = session.connection.lock().await;
    let conn = guard
        .as_mut()
        .ok_or_else(|| Error::new("Connection is closed"))?;
    let row: Option<mysql_async::Row> = conn
        .query_first(format!(
            "SHOW CREATE {kind} {}.{}",
            session.quote_identifier(&schema),
            session.quote_identifier(&name)
        ))
        .await
        .map_err(err)?;
    let definition = row.and_then(|row| row.get_opt::<Option<String>, _>(2)).transpose().map_err(|_| Error::new("Could not decode routine definition"))?.flatten().ok_or_else(|| Error::new("Routine definition unavailable. Check SHOW CREATE permissions and refresh the catalog."))?;
    if definition.len() > 2 * 1024 * 1024 {
        return Err(Error::new(
            "Routine definition exceeds the 2 MiB viewer limit",
        ));
    }
    Ok(definition)
}
