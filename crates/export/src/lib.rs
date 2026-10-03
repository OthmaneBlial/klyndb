use klyndb_driver_api::{Cell, Error, Result, Row, quote_clickhouse_identifier, quote_identifier};
use std::io::Write;

pub fn export(
    out: impl Write,
    columns: &[String],
    rows: impl Iterator<Item = Result<Row>>,
    format: &str,
    table: &str,
) -> Result<u64> {
    export_for_engine(out, columns, rows, format, table, "sqlite")
}

pub fn export_for_engine(
    mut out: impl Write,
    columns: &[String],
    rows: impl Iterator<Item = Result<Row>>,
    format: &str,
    table: &str,
    engine: &str,
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
            let quote = |name: &str| {
                if engine == "clickhouse" {
                    quote_clickhouse_identifier(name)
                } else if engine == "mssql" {
                    format!("[{}]", name.replace(']', "]]"))
                } else if engine == "mysql" {
                    format!("`{}`", name.replace('`', "``"))
                } else {
                    quote_identifier(name)
                }
            };
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
                        Cell::Boolean(b) if engine == "mssql" => if *b { "1" } else { "0" }.into(),
                        Cell::Boolean(b) => {
                            if *b {
                                "TRUE".into()
                            } else {
                                "FALSE".into()
                            }
                        }
                        Cell::Binary(b) if engine == "postgres" => format!("decode('{b}', 'hex')"),
                        Cell::Binary(b) if engine == "clickhouse" => format!("unhex('{b}')"),
                        Cell::Binary(b) if engine == "duckdb" => format!("from_hex('{b}')"),
                        Cell::Binary(b) if engine == "mssql" => format!("0x{b}"),
                        Cell::Binary(b) => format!("X'{b}'"),
                        _ if engine == "mysql" || engine == "clickhouse" => {
                            // Hex UTF-8 avoids mode-dependent backslash and quote interpretation.
                            let hex: String = c
                                .text()
                                .bytes()
                                .flat_map(|b| {
                                    let digits = b"0123456789abcdef";
                                    [
                                        digits[(b >> 4) as usize] as char,
                                        digits[(b & 15) as usize] as char,
                                    ]
                                })
                                .collect();
                            if engine == "clickhouse" {
                                format!("unhex('{hex}')")
                            } else {
                                format!("CONVERT(X'{hex}' USING utf8mb4)")
                            }
                        }
                        _ if engine == "postgres" => {
                            format!("E'{}'", c.text().replace('\\', "\\\\").replace('\'', "''"))
                        }
                        _ if engine == "mssql" => format!("N'{}'", c.text().replace('\'', "''")),
                        _ => format!("'{}'", c.text().replace('\'', "''")),
                    })
                    .collect::<Vec<String>>();
                writeln!(
                    out,
                    "INSERT INTO {} ({}) VALUES ({});",
                    quote(table),
                    columns
                        .iter()
                        .map(|c| quote(c))
                        .collect::<Vec<_>>()
                        .join(", "),
                    values.join(", ")
                )
                .map_err(io)?;
                if engine == "mssql" {
                    out.write_all(b"GO\n").map_err(io)?;
                }
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
