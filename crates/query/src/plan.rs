use klyndb_driver_api::{Error, PlanFormat, Result, Row};
use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Serialize)]
pub struct PlanNode {
    pub label: String,
    pub attributes: Vec<(String, String)>,
    pub children: Vec<PlanNode>,
}
#[derive(Debug, Serialize)]
pub struct Plan {
    pub format: PlanFormat,
    pub raw: String,
    pub nodes: Vec<PlanNode>,
    pub warnings: Vec<String>,
}
fn guard(depth: usize, count: &mut usize) -> Result<()> {
    *count += 1;
    if depth > 64 || *count > 5000 {
        return Err(Error::new(
            "Plan exceeds the 64-level / 5,000-node tree limit. Export the raw results.",
        ));
    }
    Ok(())
}
fn json_node(label: &str, value: &Value, depth: usize, count: &mut usize) -> Result<PlanNode> {
    guard(depth, count)?;
    let mut node = PlanNode {
        label: label.into(),
        attributes: vec![],
        children: vec![],
    };
    match value {
        Value::Object(values) => {
            for key in ["Node Type", "table_name", "access_type"] {
                if let Some(Value::String(name)) = values.get(key) {
                    node.label = format!("{label} · {name}");
                    break;
                }
            }
            for (key, value) in values {
                if value.is_object() || value.is_array() {
                    node.children.push(json_node(key, value, depth + 1, count)?);
                } else {
                    node.attributes.push((
                        key.clone(),
                        value
                            .as_str()
                            .map_or_else(|| value.to_string(), str::to_owned),
                    ));
                }
            }
        }
        Value::Array(values) => {
            for (i, value) in values.iter().enumerate() {
                node.children
                    .push(json_node(&format!("{}", i + 1), value, depth + 1, count)?);
            }
        }
        _ => node.attributes.push(("Value".into(), value.to_string())),
    }
    Ok(node)
}
fn text_node(
    lines: &[(usize, String)],
    at: &mut usize,
    depth: usize,
    count: &mut usize,
) -> Result<PlanNode> {
    guard(depth, count)?;
    let (indent, label) = &lines[*at];
    let mut node = PlanNode {
        label: label.clone(),
        attributes: vec![],
        children: vec![],
    };
    *at += 1;
    while *at < lines.len() && lines[*at].0 > *indent {
        node.children.push(text_node(lines, at, depth + 1, count)?);
    }
    Ok(node)
}
fn sqlite_node(
    id: i64,
    rows: &HashMap<i64, (i64, String)>,
    depth: usize,
    count: &mut usize,
    visited: &mut HashSet<i64>,
) -> Result<PlanNode> {
    guard(depth, count)?;
    if !visited.insert(id) {
        return Err(Error::new("SQLite returned a cyclic query plan"));
    }
    let (parent, label) = &rows[&id];
    let mut node = PlanNode {
        label: label.clone(),
        attributes: vec![
            ("id".into(), id.to_string()),
            ("parent".into(), parent.to_string()),
        ],
        children: vec![],
    };
    // ponytail: bounded 5,000-node scan; build an adjacency map if profiling shows a bottleneck.
    let mut children: Vec<_> = rows
        .iter()
        .filter(|(child, (parent, _))| **child != id && *parent == id)
        .map(|(id, _)| *id)
        .collect();
    children.sort_unstable();
    for child in children {
        node.children
            .push(sqlite_node(child, rows, depth + 1, count, visited)?);
    }
    Ok(node)
}
pub fn decode(
    format: PlanFormat,
    columns: &[String],
    rows: &[Row],
    warnings: Vec<String>,
) -> Result<Plan> {
    let mut count = 0;
    let raw = if format == PlanFormat::Sqlite {
        serde_json::to_string_pretty(&serde_json::json!({"columns": columns, "rows": rows}))
            .map_err(|e| Error::new(e.to_string()))?
    } else {
        if rows.len() != 1 || rows[0].len() != 1 {
            return Err(Error::new(
                "The server did not return a single native execution plan",
            ));
        }
        rows[0][0].text()
    };
    if raw.len() > 4 * 1024 * 1024 {
        return Err(Error::new("Plan exceeds 4 MiB. Export the raw results."));
    }
    let nodes = match format {
        PlanFormat::Sqlite => {
            let mut entries = HashMap::new();
            for row in rows {
                if row.len() != 4 {
                    return Err(Error::new("Unexpected SQLite QUERY PLAN columns"));
                }
                let id = row[0]
                    .text()
                    .parse::<i64>()
                    .map_err(|_| Error::new("Invalid SQLite plan ID"))?;
                let parent = row[1]
                    .text()
                    .parse::<i64>()
                    .map_err(|_| Error::new("Invalid SQLite plan parent"))?;
                if entries.insert(id, (parent, row[3].text())).is_some() {
                    return Err(Error::new("Duplicate SQLite plan ID"));
                }
            }
            let mut roots: Vec<_> = entries
                .iter()
                .filter(|(id, (parent, _))| **id == *parent || !entries.contains_key(parent))
                .map(|(id, _)| *id)
                .collect();
            roots.sort_unstable();
            let mut visited = HashSet::new();
            let nodes = roots
                .into_iter()
                .map(|id| sqlite_node(id, &entries, 0, &mut count, &mut visited))
                .collect::<Result<Vec<_>>>()?;
            if visited.len() != entries.len() {
                return Err(Error::new(
                    "SQLite returned an unreachable/cyclic plan node",
                ));
            }
            nodes
        }
        PlanFormat::MysqlTree => {
            let mut lines: Vec<(usize, String)> = vec![];
            for line in raw.lines().filter(|line| !line.trim().is_empty()) {
                if let Some((indent, label)) = line.split_once("->") {
                    lines.push((indent.len(), label.trim().into()));
                } else if let Some((_, label)) = lines.last_mut() {
                    label.push('\n');
                    label.push_str(line.trim());
                } else {
                    lines.push((0, line.trim().into()));
                }
            }
            let mut at = 0;
            let mut nodes = vec![];
            while at < lines.len() {
                nodes.push(text_node(&lines, &mut at, 0, &mut count)?);
            }
            nodes
        }
        _ => {
            let value: Value = serde_json::from_str(&raw)
                .map_err(|e| Error::new(format!("Invalid native JSON plan: {e}")))?;
            vec![json_node("Execution plan", &value, 0, &mut count)?]
        }
    };
    let plan = Plan {
        format,
        raw,
        nodes,
        warnings,
    };
    if serde_json::to_vec(&plan)
        .map_err(|e| Error::new(e.to_string()))?
        .len()
        > 8 * 1024 * 1024
    {
        return Err(Error::new(
            "Formatted plan exceeds 8 MiB. Export the raw results.",
        ));
    }
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use klyndb_driver_api::Cell;
    #[test]
    fn native_shapes_and_malformed_trees() {
        let raw = r#"[{"Plan":{"Node Type":"Seq Scan","Total Cost":1.2,"Actual Total Time":0.002,"Actual Rows":3,"Actual Loops":1},"Execution Time":0.03}]"#;
        let plan = decode(
            PlanFormat::PostgresJson,
            &[],
            &[vec![Cell::Text(raw.into())]],
            vec![],
        )
        .unwrap();
        assert_eq!(plan.raw, raw);
        assert!(
            plan.nodes[0].children[0].children[0]
                .attributes
                .contains(&("Actual Loops".into(), "1".into()))
        );
        let plan = decode(
            PlanFormat::MysqlTree,
            &[],
            &[vec![Cell::Text(
                "-> Filter (actual time=0.1..0.2 rows=3 loops=1)\n    -> Scan (cost=2 rows=3)"
                    .into(),
            )]],
            vec![],
        )
        .unwrap();
        assert_eq!(plan.nodes[0].children.len(), 1);
        let row = |id: &str, parent: &str| {
            vec![
                Cell::Number(id.into()),
                Cell::Number(parent.into()),
                Cell::Number("0".into()),
                Cell::Text("SCAN t".into()),
            ]
        };
        assert!(
            decode(
                PlanFormat::Sqlite,
                &[],
                &[row("1", "2"), row("2", "1")],
                vec![]
            )
            .is_err()
        );
        assert_eq!(
            decode(
                PlanFormat::Sqlite,
                &[],
                &[row("1", "0"), row("2", "1")],
                vec![]
            )
            .unwrap()
            .nodes[0]
                .children
                .len(),
            1
        );
        let mut value = serde_json::json!(1);
        for _ in 0..66 {
            value = serde_json::json!({"child": value});
        }
        assert!(
            decode(
                PlanFormat::MysqlJson,
                &[],
                &[vec![Cell::Json(value)]],
                vec![]
            )
            .is_err()
        );
    }
}
