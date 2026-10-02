use crate::Engine;
use klyndb_driver_api::{Column, Error, ForeignKey, Result, Table};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

pub const WIDTH: f64 = 280.0;
pub const HEADER: f64 = 50.0;
pub const ROW: f64 = 24.0;
#[derive(Clone, Serialize, Deserialize)]
pub struct DiagramTable {
    pub table: Table,
    pub columns: Vec<Column>,
    pub relationships: Vec<ForeignKey>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Diagram {
    pub tables: Vec<DiagramTable>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}
pub type Positions = BTreeMap<String, Point>;
pub fn key(schema: &str, name: &str) -> String {
    serde_json::to_string(&(schema, name)).expect("string tuples serialize")
}
impl Engine {
    pub async fn diagram(&self, id: &str, tables: &[Table]) -> Result<Diagram> {
        if tables.is_empty() || tables.len() > 50 {
            return Err(Error::new("Select 1–50 tables for a diagram"));
        }
        let driver = self.driver(id).await?;
        if !driver.capabilities().diagrams {
            return Err(Error::new(
                "This driver does not support relationship diagrams",
            ));
        }
        let mut seen = HashSet::new();
        for table in tables {
            if table.schema.len() + table.name.len() > 16 * 1024
                || !seen.insert(key(&table.schema, &table.name))
            {
                return Err(Error::new("Select distinct valid table names"));
            }
        }
        tokio::time::timeout(std::time::Duration::from_secs(30), async {
            let mut model=Diagram {tables:vec![]};
            for table in tables {
                let info=driver.inspect(table).await?;
                model.tables.push(DiagramTable {table:table.clone(),columns:info.columns,relationships:driver.relationships(table).await?});
            }
            model.validate()?;
            Ok(model)
        }).await.map_err(|_| Error::new("Diagram metadata timed out. Select fewer tables or wait for the current query to finish."))?
    }
}
impl Diagram {
    fn validate(&self) -> Result<()> {
        if self.tables.is_empty()
            || self.tables.len() > 50
            || self.tables.iter().map(|t| t.columns.len()).sum::<usize>() > 2000
            || self
                .tables
                .iter()
                .flat_map(|t| &t.relationships)
                .map(|r| r.columns.len())
                .sum::<usize>()
                > 4000
        {
            return Err(Error::new(
                "A diagram supports 1–50 tables and 2,000 columns and 4,000 relationship column pairs. Select a smaller area of the schema.",
            ));
        }
        let bytes = serde_json::to_vec(self).map_err(|e| Error::new(e.to_string()))?;
        if bytes.len() > 4 * 1024 * 1024 {
            return Err(Error::new(
                "Diagram metadata exceeds 4 MiB. Select fewer tables.",
            ));
        }
        let mut seen = HashSet::new();
        for table in &self.tables {
            if !seen.insert(key(&table.table.schema, &table.table.name))
                || table
                    .relationships
                    .iter()
                    .any(|r| r.columns.is_empty() || r.columns.len() != r.target_columns.len())
            {
                return Err(Error::new("Invalid diagram metadata"));
            }
        }
        Ok(())
    }
    pub fn svg(&self, positions: &Positions) -> Result<String> {
        self.validate()?;
        if positions.len() > 50
            || positions.values().any(|p| {
                !p.x.is_finite()
                    || !p.y.is_finite()
                    || p.x.abs() > 100_000.0
                    || p.y.abs() > 100_000.0
            })
        {
            return Err(Error::new("Invalid diagram positions"));
        }
        let nodes: Vec<_> = self
            .tables
            .iter()
            .map(|t| {
                let id = key(&t.table.schema, &t.table.name);
                positions
                    .get(&id)
                    .map(|p| (t, p))
                    .ok_or_else(|| Error::new("Position each selected table before exporting"))
            })
            .collect::<Result<_>>()?;
        let min_x = nodes.iter().map(|(_, p)| p.x).fold(f64::INFINITY, f64::min) - 100.0;
        let min_y = nodes.iter().map(|(_, p)| p.y).fold(f64::INFINITY, f64::min) - 60.0;
        let width = nodes
            .iter()
            .map(|(_, p)| p.x + WIDTH + 100.0)
            .fold(f64::NEG_INFINITY, f64::max)
            - min_x;
        let height = nodes
            .iter()
            .map(|(t, p)| p.y + HEADER + ROW * t.columns.len() as f64 + 60.0)
            .fold(f64::NEG_INFINITY, f64::max)
            - min_y;
        let mut svg = format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="{min_x} {min_y} {width} {height}"><title>Klyndb relationship diagram</title><rect x="{min_x}" y="{min_y}" width="{width}" height="{height}" fill="#141b1f"/><defs><marker id="arrow" markerWidth="8" markerHeight="8" refX="7" refY="4" orient="auto"><path d="M0 0 L8 4 L0 8" fill="#93d4b5"/></marker></defs>"##
        );
        for (source, p) in &nodes {
            for fk in &source.relationships {
                let Some((target, q)) = nodes.iter().find(|(t, _)| {
                    t.table.schema == fk.target_schema && t.table.name == fk.target_table
                }) else {
                    continue;
                };
                for (from, to) in fk.columns.iter().zip(&fk.target_columns) {
                    let sy = p.y
                        + HEADER
                        + ROW
                            * (source
                                .columns
                                .iter()
                                .position(|c| &c.name == from)
                                .unwrap_or(0) as f64
                                + 0.5);
                    let ty = q.y
                        + HEADER
                        + ROW
                            * (target
                                .columns
                                .iter()
                                .position(|c| Some(&c.name) == to.as_ref())
                                .unwrap_or(0) as f64
                                + 0.5);
                    let right = q.x >= p.x;
                    let sx = p.x + if right { WIDTH } else { 0.0 };
                    let same = source.table.schema == target.table.schema
                        && source.table.name == target.table.name;
                    let tx = q.x + if same || !right { WIDTH } else { 0.0 };
                    let bend = if right { 70.0 } else { -70.0 };
                    svg.push_str(&format!(r##"<path d="M {sx} {sy} C {} {sy}, {} {ty}, {tx} {ty}" fill="none" stroke="#93d4b5" stroke-width="1.5" marker-end="url(#arrow)"><title>{}</title></path>"##,sx+bend,tx+if same {70.0} else {-bend},xml(&short(&fk.name,120))));
                }
            }
        }
        for (table, p) in nodes {
            let height = HEADER + ROW * table.columns.len() as f64 + 8.0;
            svg.push_str(&format!(r##"<g font-family="monospace"><rect x="{}" y="{}" width="{WIDTH}" height="{height}" rx="8" fill="#20292f" stroke="#496355"/><text x="{}" y="{}" fill="#93d4b5" font-size="10">{}</text><text x="{}" y="{}" fill="#f3f3eb" font-size="13" font-weight="bold">{}</text>"##,p.x,p.y,p.x+12.0,p.y+17.0,xml(&short(&table.table.schema,32)),p.x+12.0,p.y+36.0,xml(&short(&table.table.name,30))));
            for (i, c) in table.columns.iter().enumerate() {
                let prefix = if c.primary_key {
                    "◆ "
                } else if table
                    .relationships
                    .iter()
                    .any(|r| r.columns.contains(&c.name))
                {
                    "↗ "
                } else {
                    "  "
                };
                let y = p.y + HEADER + ROW * i as f64 + 17.0;
                svg.push_str(&format!(r##"<text x="{}" y="{y}" fill="#e7e9e8" font-size="11">{}{}</text><text x="{}" y="{y}" fill="#a2b2ba" font-size="10">{}</text>"##,p.x+12.0,prefix,xml(&short(&c.name,20)),p.x+185.0,xml(&short(&c.data_type,12))));
            }
            svg.push_str("</g>");
        }
        svg.push_str("</svg>");
        Ok(svg)
    }
}
fn xml(value: &str) -> String {
    value
        .chars()
        .filter(|c| *c >= ' ' || matches!(*c, '\t' | '\n' | '\r'))
        .collect::<String>()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
fn short(value: &str, max: usize) -> String {
    if value.chars().count() > max {
        format!("{}…", value.chars().take(max - 1).collect::<String>())
    } else {
        value.into()
    }
}
