use klyndb_driver_api::{Cell, Error, Result, Row, quote_identifier};
use std::io::Write;

pub fn export(
    mut out: impl Write,
    columns: &[String],
    rows: impl Iterator<Item = Result<Row>>,
    format: &str,
    table: &str,
) -> Result<u64> {
    let io = |e: std::io::Error| Error::new(e.to_string());
    let mut count = 0;
    match format {
        "csv" => {
            let mut csv = csv::Writer::from_writer(&mut out);
            csv.write_record(columns)
                .map_err(|e| Error::new(e.to_string()))?;
            for row in rows {
                csv.write_record(row?.iter().map(Cell::text))
                    .map_err(|e| Error::new(e.to_string()))?;
                count += 1;
            }
            csv.flush().map_err(io)?;
        }
        "json" | "jsonl" => {
            if format == "json" {
                out.write_all(b"[\n").map_err(io)?;
            }
            for row in rows {
                let row = row?;
                // Duplicate column names remain lossless as arrays, instead of overwriting keys.
                let value = serde_json::json!({"columns":columns,"values":row});
                if format == "json" && count > 0 {
                    out.write_all(b",\n").map_err(io)?;
                }
                serde_json::to_writer(&mut out, &value).map_err(|e| Error::new(e.to_string()))?;
                if format == "jsonl" {
                    out.write_all(b"\n").map_err(io)?;
                }
                count += 1;
            }
            if format == "json" {
                out.write_all(b"\n]\n").map_err(io)?;
            }
        }
        "sql" => {
            if table.trim().is_empty() {
                return Err(Error::new(
                    "Enter a target table name for SQL INSERT export",
                ));
            }
            for row in rows {
                let values = row?
                    .iter()
                    .map(|c| match c {
                        Cell::Null => "NULL".into(),
                        Cell::Number(n) => n.clone(),
                        Cell::Boolean(b) => {
                            if *b {
                                "TRUE".into()
                            } else {
                                "FALSE".into()
                            }
                        }
                        Cell::Binary(b) => format!("X'{b}'"),
                        _ => format!("'{}'", c.text().replace('\'', "''")),
                    })
                    .collect::<Vec<String>>();
                writeln!(
                    out,
                    "INSERT INTO {} ({}) VALUES ({});",
                    quote_identifier(table),
                    columns
                        .iter()
                        .map(|c| quote_identifier(c))
                        .collect::<Vec<_>>()
                        .join(", "),
                    values.join(", ")
                )
                .map_err(io)?;
                count += 1;
            }
        }
        "markdown" => {
            fn escaped(s: &str) -> String {
                s.replace('|', "\\|")
                    .replace('\n', "<br>")
                    .replace('\r', "")
            }
            writeln!(
                out,
                "| {} |",
                columns
                    .iter()
                    .map(|c| escaped(c))
                    .collect::<Vec<_>>()
                    .join(" | ")
            )
            .map_err(io)?;
            writeln!(
                out,
                "| {} |",
                columns
                    .iter()
                    .map(|_| "---")
                    .collect::<Vec<_>>()
                    .join(" | ")
            )
            .map_err(io)?;
            for row in rows {
                writeln!(
                    out,
                    "| {} |",
                    row?.iter()
                        .map(|c| if *c == Cell::Null {
                            "NULL".into()
                        } else {
                            escaped(&c.text())
                        })
                        .collect::<Vec<_>>()
                        .join(" | ")
                )
                .map_err(io)?;
                count += 1;
            }
        }
        _ => return Err(Error::new("Unsupported export format")),
    }
    out.flush().map_err(io)?;
    Ok(count)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn csv_quotes_and_json_preserves_null_and_large_integer() {
        let row = vec![
            Cell::Text("a,\"b\n".into()),
            Cell::Null,
            Cell::Number("9223372036854775807".into()),
        ];
        let mut csv = vec![];
        export(
            &mut csv,
            &["a".into(), "b".into(), "c".into()],
            vec![Ok(row.clone())].into_iter(),
            "csv",
            "",
        )
        .unwrap();
        assert!(String::from_utf8(csv).unwrap().contains("\"a,\"\"b\n\""));
        let mut json = vec![];
        export(
            &mut json,
            &["a".into(), "a".into(), "c".into()],
            vec![Ok(row)].into_iter(),
            "json",
            "",
        )
        .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&json).unwrap();
        assert_eq!(value[0]["values"][1]["kind"], "null");
        assert_eq!(value[0]["values"][2]["value"], "9223372036854775807");
    }
}
