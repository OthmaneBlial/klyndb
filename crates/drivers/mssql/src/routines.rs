use super::*;
use std::collections::HashMap;

pub(super) async fn list(session: &SqlServer, search: &str, offset: u32) -> Result<RoutinePage> {
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
            .replace('[', "![")
    );
    let rows = session.metadata(format!("SELECT o.object_id,SCHEMA_NAME(o.schema_id),o.name,o.type FROM sys.objects o WHERE o.is_ms_shipped=0 AND o.type IN ('P','FN','IF','TF') AND (SCHEMA_NAME(o.schema_id)+N'.'+o.name) LIKE {} ESCAPE N'!' ORDER BY SCHEMA_NAME(o.schema_id),o.name,o.object_id OFFSET {offset} ROWS FETCH NEXT 101 ROWS ONLY", literal(&pattern))).await?;
    let has_more = rows.len() > 100;
    let mut routines: Vec<Routine> = rows
        .into_iter()
        .take(100)
        .map(|row| {
            let id = row[0]
                .text()
                .parse::<i32>()
                .map_err(|_| Error::new("Invalid native routine identifier"))?;
            let kind = row[3].text();
            Ok(Routine {
                id: id.to_string(),
                schema: row[1].text(),
                name: row[2].text(),
                kind: if kind.trim() == "P" {
                    "procedure"
                } else {
                    "function"
                }
                .into(),
                arguments: String::new(),
                returns: matches!(kind.trim(), "IF" | "TF").then(|| "TABLE".into()),
                language: "Transact-SQL".into(),
            })
        })
        .collect::<Result<_>>()?;
    if !routines.is_empty() {
        let ids = routines
            .iter()
            .map(|r| r.id.as_str())
            .collect::<Vec<_>>()
            .join(",");
        let ty = edit::data_type_sql("p");
        let parameters = session.metadata(format!("SELECT p.object_id,p.parameter_id,p.name,{ty},p.is_output,p.is_readonly FROM sys.parameters p JOIN sys.types t ON t.user_type_id=p.user_type_id WHERE p.object_id IN ({ids}) ORDER BY p.object_id,p.parameter_id")).await?;
        let mut by_id: HashMap<String, &mut Routine> =
            routines.iter_mut().map(|r| (r.id.clone(), r)).collect();
        for row in parameters {
            let id = row[0].text();
            if let Some(routine) = by_id.get_mut(id.as_str()) {
                if row[1].text() == "0" {
                    routine.returns = Some(row[3].text());
                } else {
                    if !routine.arguments.is_empty() {
                        routine.arguments.push_str(", ");
                    }
                    routine.arguments.push_str(&format!(
                        "{} {}{}{}",
                        row[2].text(),
                        row[3].text(),
                        if row[4].text() == "true" {
                            " OUTPUT"
                        } else {
                            ""
                        },
                        if row[5].text() == "true" {
                            " READONLY"
                        } else {
                            ""
                        }
                    ));
                }
            }
        }
    }
    Ok(RoutinePage { routines, has_more })
}

pub(super) async fn definition(session: &SqlServer, id: &str) -> Result<String> {
    let id = id
        .parse::<i32>()
        .ok()
        .filter(|id| *id > 0)
        .ok_or_else(|| Error::new("Invalid routine identifier"))?;
    let rows = session.metadata(format!("SELECT m.definition FROM sys.objects o LEFT JOIN sys.sql_modules m ON m.object_id=o.object_id WHERE o.object_id={id} AND o.is_ms_shipped=0 AND o.type IN ('P','FN','IF','TF')")).await?;
    let definition = rows.first().and_then(|r| r.first()).filter(|c| !matches!(c, Cell::Null)).map(Cell::text).ok_or_else(|| Error::new("Routine definition unavailable: it may be encrypted, hidden by permissions, or removed. Refresh the catalog."))?;
    if definition.len() > 2 * 1024 * 1024 {
        return Err(Error::new(
            "Routine definition exceeds the 2 MiB viewer limit",
        ));
    }
    Ok(definition)
}

#[cfg(test)]
mod tests;
