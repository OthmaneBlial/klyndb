use super::{PlanNode, guard};
use klyndb_driver_api::{Cell, Error, Result, Row};
use std::collections::{BTreeMap, HashSet};

pub(super) fn decode(
    columns: &[String],
    rows: &[Row],
    count: &mut usize,
    warnings: &mut Vec<String>,
) -> Result<Vec<PlanNode>> {
    let index = |name: &str| {
        columns
            .iter()
            .position(|c| c == name)
            .ok_or_else(|| Error::new("Unexpected SQL Server native plan columns"))
    };
    let statement = index("StmtId")?;
    let id = index("NodeId")?;
    let parent = index("Parent")?;
    index("StmtText")?;
    let warning = index("Warnings")?;
    let mut entries = BTreeMap::new();
    if rows.is_empty() || columns.iter().collect::<HashSet<_>>().len() != columns.len() {
        return Err(Error::new("Invalid SQL Server native plan"));
    }
    for (i, row) in rows.iter().enumerate() {
        if row.len() != columns.len() {
            return Err(Error::new("Unexpected SQL Server plan row width"));
        }
        let number = |column: usize| {
            row[column]
                .text()
                .parse::<u32>()
                .map_err(|_| Error::new("Invalid SQL Server plan ID"))
        };
        let key = (number(statement)?, number(id)?);
        let parent = number(parent)?;
        if key.1 == 0 || entries.insert(key, (parent, i)).is_some() {
            return Err(Error::new("Duplicate/invalid SQL Server plan ID"));
        }
        if !matches!(row[warning], Cell::Null) && !row[warning].text().is_empty() {
            warnings.push(row[warning].text());
        }
    }
    let mut children: BTreeMap<(u32, u32), Vec<(u32, u32)>> = BTreeMap::new();
    let mut roots = vec![];
    for (&key, &(parent, _)) in &entries {
        if parent == 0 {
            roots.push(key);
        } else if !entries.contains_key(&(key.0, parent)) {
            return Err(Error::new("SQL Server plan parent is missing"));
        } else {
            children.entry((key.0, parent)).or_default().push(key);
        }
    }
    fn node(
        key: (u32, u32),
        depth: usize,
        entries: &BTreeMap<(u32, u32), (u32, usize)>,
        children: &BTreeMap<(u32, u32), Vec<(u32, u32)>>,
        rows: &[Row],
        columns: &[String],
        state: &mut (usize, HashSet<(u32, u32)>),
    ) -> Result<PlanNode> {
        guard(depth, &mut state.0)?;
        if !state.1.insert(key) {
            return Err(Error::new("Cyclic SQL Server plan"));
        }
        let row = &rows[entries[&key].1];
        let label = columns
            .iter()
            .position(|c| c == "StmtText")
            .expect("validated StmtText");
        let nodes = children
            .get(&key)
            .into_iter()
            .flatten()
            .map(|&child| node(child, depth + 1, entries, children, rows, columns, state))
            .collect::<Result<_>>()?;
        Ok(PlanNode {
            label: row[label].text().trim().trim_start_matches("|--").into(),
            attributes: columns
                .iter()
                .zip(row)
                .filter(|(_, v)| !matches!(v, Cell::Null))
                .map(|(c, v)| (c.clone(), v.text()))
                .collect(),
            children: nodes,
        })
    }
    let mut state = (*count, HashSet::new());
    let nodes = roots
        .into_iter()
        .map(|key| node(key, 0, &entries, &children, rows, columns, &mut state))
        .collect::<Result<_>>()?;
    if state.1.len() != entries.len() {
        return Err(Error::new("SQL Server returned an unreachable/cyclic plan"));
    }
    *count = state.0;
    Ok(nodes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn statement_scoped_ids_and_bounded_native_tree() {
        let columns = ["StmtText", "StmtId", "NodeId", "Parent", "Warnings"].map(str::to_owned);
        let row = |statement: u32, id: u32, parent: u32| {
            vec![
                Cell::Text("|--Index Seek".into()),
                Cell::Number(statement.to_string()),
                Cell::Number(id.to_string()),
                Cell::Number(parent.to_string()),
                Cell::Null,
            ]
        };
        let mut rows = vec![row(1, 1, 0), row(1, 2, 1), row(2, 1, 0)];
        rows[1][4] = Cell::Text("NO STATS".into());
        let mut warnings = vec![];
        let nodes = decode(&columns, &rows, &mut 0, &mut warnings).unwrap();
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[0].children.len(), 1);
        assert_eq!(nodes[0].label, "Index Seek");
        assert_eq!(warnings, ["NO STATS"]);
        for malformed in [
            vec![row(1, 1, 0), row(1, 1, 0)],
            vec![row(1, 1, 2), row(1, 2, 1)],
            vec![row(1, 1, 9)],
        ] {
            assert!(decode(&columns, &malformed, &mut 0, &mut vec![]).is_err());
        }
        let deep = (1..=66).map(|id| row(1, id, id - 1)).collect::<Vec<_>>();
        assert!(decode(&columns, &deep, &mut 0, &mut vec![]).is_err());
    }
}
