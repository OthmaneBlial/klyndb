mod json;
pub use json::JsonReader;
mod sql;
pub use sql::{SqlPreview, SqlReader};

use csv_core::{ReadRecordResult, ReaderBuilder};
use klyndb_driver_api::{
    Cell, Change, Column, Error, InsertBatch, Result, ScriptBatch, validate_insert_batch,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    fs::File,
    io::{BufRead, BufReader, Chain, Cursor, Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

pub const FILE_LIMIT: u64 = 512 * 1024 * 1024;
pub const RECORD_LIMIT: usize = 8 * 1024 * 1024;
pub const COLUMN_LIMIT: usize = 1000;

fn error(e: impl std::fmt::Display) -> Error {
    Error::new(e.to_string())
}

/// Preview and execution read the same private snapshot, never a frontend path.
pub struct Snapshot {
    _directory: tempfile::TempDir,
    path: PathBuf,
    pub name: String,
    pub bytes: u64,
}
impl Snapshot {
    pub fn sql_reader(&self, engine: &str) -> Result<SqlReader<File>> {
        SqlReader::new(File::open(&self.path).map_err(error)?, engine)
    }
    pub fn preview_sql(&self, engine: &str) -> Result<SqlPreview> {
        let mut reader = self.sql_reader(engine)?;
        let mut preview = SqlPreview {
            statements: 0,
            sample: vec![],
            warnings: vec![],
        };
        while let Some(sql) = reader.next_statement()? {
            preview.statements += 1;
            if preview.sample.len() < 5 {
                let end = sql
                    .char_indices()
                    .map(|(i, _)| i)
                    .find(|&i| i >= 512)
                    .unwrap_or(sql.len());
                preview.sample.push(if end < sql.len() {
                    format!("{}…", &sql[..end])
                } else {
                    sql.clone()
                });
            }
            for warning in reader.warnings() {
                if !preview.warnings.contains(warning) {
                    preview.warnings.push(warning.clone());
                }
            }
        }
        if preview.statements == 0 {
            return Err(Error::new("The SQL file contains no statements"));
        }
        Ok(preview)
    }
    pub fn produce_sql(
        &self,
        engine: &str,
        output: mpsc::Sender<Result<ScriptBatch>>,
        cancel: CancellationToken,
        read: &AtomicU64,
    ) -> Result<u64> {
        let result = (|| {
            let mut reader = self.sql_reader(engine)?;
            reader.set_cancel(cancel.clone());
            let mut count = 0;
            while let Some(sql) = reader.next_statement()? {
                output
                    .blocking_send(Ok(ScriptBatch::Statement(sql)))
                    .map_err(|_| Error::new("SQL writer stopped"))?;
                count += 1;
                read.store(count, Ordering::Relaxed);
            }
            if cancel.is_cancelled() {
                return Err(Error::new("SQL import cancelled"));
            }
            output
                .blocking_send(Ok(ScriptBatch::Complete))
                .map_err(|_| Error::new("SQL writer stopped"))?;
            Ok(count)
        })();
        if let Err(e) = &result {
            let _ = output.blocking_send(Err(Error::new(&e.message)));
        }
        result
    }
    pub fn copy(source: &Path) -> Result<Self> {
        let file =
            File::open(source).map_err(|_| Error::new("Could not open the selected file"))?;
        let metadata = file.metadata().map_err(error)?;
        if !metadata.is_file() || metadata.len() > FILE_LIMIT {
            return Err(Error::new("Choose a regular data file of at most 512 MiB"));
        }
        let directory = tempfile::Builder::new()
            .prefix("klyndb-import-")
            .tempdir()
            .map_err(error)?;
        let path = directory.path().join("source");
        let mut out = File::create(&path).map_err(error)?;
        let bytes = std::io::copy(&mut file.take(FILE_LIMIT + 1), &mut out).map_err(error)?;
        if bytes > FILE_LIMIT {
            return Err(Error::new("The selected file grew beyond 512 MiB"));
        }
        out.flush().map_err(error)?;
        Ok(Self {
            _directory: directory,
            path,
            name: source
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into(),
            bytes,
        })
    }
    pub fn reader(&self, options: &ImportOptions) -> Result<ImportReader> {
        options.validate()?;
        let file = File::open(&self.path).map_err(error)?;
        match options.format {
            ImportFormat::Csv => Ok(ImportReader::Csv(Box::new(CsvReader::new(
                file,
                options.delimiter()?,
            )?))),
            format => Ok(ImportReader::Json(JsonReader::new(file, format)?)),
        }
    }
    pub fn preview(&self, options: &ImportOptions) -> Result<Preview> {
        let mut reader = self.reader(options)?;
        let headers = reader.headers()?;
        let mut rows = vec![];
        let mut clipped = false;
        for _ in 0..5 {
            let Some(row) = reader.next_record()? else {
                break;
            };
            reader.check_width(&row, headers.len())?;
            rows.push(
                row.into_iter()
                    .map(|cell| {
                        let value = if cell == Cell::Null {
                            return None;
                        } else {
                            cell_text(cell)
                        };
                        let end = value
                            .char_indices()
                            .map(|(i, _)| i)
                            .find(|&i| i >= 128)
                            .unwrap_or(value.len());
                        if end < value.len() {
                            clipped = true;
                            Some(format!("{}…", &value[..end]))
                        } else {
                            Some(value)
                        }
                    })
                    .collect(),
            );
        }
        let mut preview = Preview {
            headers,
            rows,
            clipped,
        };
        while serde_json::to_vec(&preview).map_err(error)?.len() > 1024 * 1024 {
            preview.rows.pop();
            preview.clipped = true;
        }
        Ok(preview)
    }
    /// Run on a blocking worker. Only an explicit Complete message permits commit.
    pub fn produce(
        &self,
        options: &ImportOptions,
        mapping: &[Mapping],
        output: mpsc::Sender<Result<InsertBatch>>,
        cancel: CancellationToken,
        read_rows: &AtomicU64,
    ) -> Result<u64> {
        let result = (|| {
            let mut reader = self.reader(options)?;
            reader.set_cancel(cancel.clone());
            let headers = reader.headers()?;
            if headers.len() != mapping.len() {
                return Err(Error::new(
                    "Source column mapping changed; preview the file again",
                ));
            }
            let mut changes = vec![];
            let (mut bytes, mut count) = (0, 0);
            while let Some(row) = reader.next_record()? {
                reader.check_width(&row, headers.len())?;
                let change = insert_cells(row, mapping, options).map_err(|e| {
                    Error::new(format!(
                        "{} record {}: {}",
                        options.format.label(),
                        reader.record(),
                        e.message
                    ))
                })?;
                let size = change.values().map_or(0, |values| {
                    values
                        .iter()
                        .map(|(key, cell)| key.len() + cell.byte_len())
                        .sum::<usize>()
                });
                validate_insert_batch(std::slice::from_ref(&change))?;
                if !changes.is_empty() && (changes.len() >= 256 || bytes + size > 256 * 1024) {
                    output
                        .blocking_send(Ok(InsertBatch::Rows(std::mem::take(&mut changes))))
                        .map_err(|_| Error::new("Import writer stopped"))?;
                    bytes = 0;
                }
                changes.push(change);
                bytes += size;
                count += 1;
                read_rows.store(count, Ordering::Relaxed);
            }
            if count == 0 {
                return Err(Error::new("The file contains no data records"));
            }
            if !changes.is_empty() {
                output
                    .blocking_send(Ok(InsertBatch::Rows(changes)))
                    .map_err(|_| Error::new("Import writer stopped"))?;
            }
            if cancel.is_cancelled() {
                return Err(Error::new("Import cancelled"));
            }
            output
                .blocking_send(Ok(InsertBatch::Complete))
                .map_err(|_| Error::new("Import writer stopped"))?;
            Ok(count)
        })();
        if let Err(e) = &result {
            let _ = output.blocking_send(Err(Error::new(&e.message)));
        }
        result
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ImportFormat {
    #[default]
    Csv,
    Json,
    KlyndbJson,
}
impl ImportFormat {
    fn label(self) -> &'static str {
        if self == Self::Csv { "CSV" } else { "JSON" }
    }
}
#[derive(Clone, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct ImportOptions {
    pub format: ImportFormat,
    /// Comma, semicolon, tab or pipe; standard double-quoted CSV fields.
    pub delimiter: String,
    pub trim: bool,
    pub null_value: Option<String>,
    pub empty_as_null: bool,
}
impl ImportOptions {
    pub fn validate(&self) -> Result<()> {
        self.delimiter()?;
        if self.null_value.as_ref().is_some_and(|s| s.len() > 256) {
            return Err(Error::new("NULL tokens are limited to 256 bytes"));
        }
        Ok(())
    }
    fn delimiter(&self) -> Result<u8> {
        match self.delimiter.as_str() {
            "" | "," => Ok(b','),
            ";" => Ok(b';'),
            "\t" => Ok(b'\t'),
            "|" => Ok(b'|'),
            _ => Err(Error::new(
                "Choose comma, semicolon, tab or pipe as the separator",
            )),
        }
    }
}
#[derive(Serialize)]
pub struct Preview {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<Option<String>>>,
    pub clipped: bool,
}

pub enum ImportReader {
    Csv(Box<CsvReader<File>>),
    Json(JsonReader<File>),
}
impl ImportReader {
    pub fn headers(&mut self) -> Result<Vec<String>> {
        match self {
            Self::Csv(r) => r.headers(),
            Self::Json(r) => r.headers(),
        }
    }
    pub fn next_record(&mut self) -> Result<Option<Vec<Cell>>> {
        match self {
            Self::Csv(r) => Ok(r
                .next_record()?
                .map(|row| row.into_iter().map(Cell::Text).collect())),
            Self::Json(r) => r.next_record(),
        }
    }
    fn record(&self) -> u64 {
        match self {
            Self::Csv(r) => r.record,
            Self::Json(r) => r.record,
        }
    }
    fn set_cancel(&mut self, cancel: CancellationToken) {
        match self {
            Self::Csv(r) => r.cancel = cancel,
            Self::Json(r) => r.cancel = cancel,
        }
    }
    fn check_width(&self, row: &[Cell], width: usize) -> Result<()> {
        if row.len() != width {
            return Err(Error::new(format!(
                "{} record {} has {} fields; expected {width}",
                if matches!(self, Self::Csv(_)) {
                    "CSV"
                } else {
                    "JSON"
                },
                self.record(),
                row.len()
            )));
        }
        Ok(())
    }
}

// csv-core decodes into bounded caller-owned buffers. This small validator rejects
// ambiguous quotation that its deliberately permissive decoder would accept.
#[derive(Clone, Copy)]
enum Quotes {
    Start,
    Text,
    Quoted,
    Closed,
}
pub struct CsvReader<R: Read> {
    input: BufReader<Chain<Cursor<Vec<u8>>, R>>,
    parser: csv_core::Reader,
    delimiter: u8,
    quotes: Quotes,
    data: Vec<u8>,
    ends: Vec<usize>,
    pub record: u64,
    cancel: CancellationToken,
}
impl<R: Read> CsvReader<R> {
    pub fn new(mut input: R, delimiter: u8) -> Result<Self> {
        if ![b',', b';', b'\t', b'|'].contains(&delimiter) {
            return Err(Error::new("Unsupported CSV separator"));
        }
        // Read a possibly fragmented UTF-8 BOM before csv-core sees any bytes.
        let mut prefix = vec![];
        for _ in 0..3 {
            let mut byte = [0];
            if input.read(&mut byte).map_err(error)? == 0 {
                break;
            }
            prefix.push(byte[0]);
        }
        if prefix == b"\xef\xbb\xbf" {
            prefix.clear();
        }
        Ok(Self {
            input: BufReader::with_capacity(64 * 1024, Cursor::new(prefix).chain(input)),
            parser: ReaderBuilder::new().delimiter(delimiter).build(),
            delimiter,
            quotes: Quotes::Start,
            data: vec![0; 64 * 1024],
            ends: vec![0; COLUMN_LIMIT + 1],
            record: 0,
            cancel: CancellationToken::new(),
        })
    }
    pub fn next_record(&mut self) -> Result<Option<Vec<String>>> {
        let (mut bytes, mut fields) = (0, 0);
        loop {
            if self.cancel.is_cancelled() {
                return Err(Error::new("Import cancelled"));
            }
            let input = self.input.fill_buf().map_err(error)?;
            if input.is_empty() && matches!(self.quotes, Quotes::Quoted) {
                return Err(Error::new(format!(
                    "CSV record {} has an unclosed quoted field",
                    self.record + 1
                )));
            }
            let (state, consumed, written, ended) =
                self.parser
                    .read_record(input, &mut self.data[bytes..], &mut self.ends[fields..]);
            for &byte in &input[..consumed] {
                self.quotes = match (self.quotes, byte) {
                    (Quotes::Start, b'"') | (Quotes::Closed, b'"') => Quotes::Quoted,
                    (Quotes::Quoted, b'"') => Quotes::Closed,
                    (Quotes::Quoted, _) => Quotes::Quoted,
                    (_, b'\r' | b'\n') => Quotes::Start,
                    (_, b) if b == self.delimiter => Quotes::Start,
                    (Quotes::Closed, _) | (Quotes::Text, b'"') => {
                        return Err(Error::new(format!(
                            "CSV record {} has invalid quotation",
                            self.record + 1
                        )));
                    }
                    _ => Quotes::Text,
                };
            }
            self.input.consume(consumed);
            bytes += written;
            fields += ended;
            if fields > COLUMN_LIMIT {
                return Err(Error::new("CSV records are limited to 1000 columns"));
            }
            match state {
                ReadRecordResult::End => return Ok(None),
                ReadRecordResult::InputEmpty => {}
                ReadRecordResult::OutputFull if self.data.len() < RECORD_LIMIT => {
                    self.data.resize((self.data.len() * 2).min(RECORD_LIMIT), 0)
                }
                ReadRecordResult::OutputFull => {
                    return Err(Error::new("A CSV record exceeds 8 MiB"));
                }
                ReadRecordResult::OutputEndsFull => {
                    return Err(Error::new("CSV records are limited to 1000 columns"));
                }
                ReadRecordResult::Record => {
                    self.record += 1;
                    let mut start = 0;
                    let row = self.ends[..fields]
                        .iter()
                        .map(|&end| {
                            let value = std::str::from_utf8(&self.data[start..end])
                                .map_err(|_| {
                                    Error::new(format!("CSV record {} is not UTF-8", self.record))
                                })?
                                .to_owned();
                            start = end;
                            Ok(value)
                        })
                        .collect::<Result<Vec<_>>>()?;
                    return Ok(Some(row));
                }
            }
        }
    }
    pub fn headers(&mut self) -> Result<Vec<String>> {
        let headers = self
            .next_record()?
            .ok_or_else(|| Error::new("The CSV file is empty; a header row is required"))?;
        if headers.iter().any(|h| h.trim().is_empty() || h.len() > 256)
            || headers.iter().map(String::len).sum::<usize>() > 64 * 1024
        {
            return Err(Error::new(
                "CSV headers must be nonempty, at most 256 bytes each and 64 KiB in total",
            ));
        }
        Ok(headers)
    }
    pub fn check_width(&self, row: &[String], width: usize) -> Result<()> {
        if row.len() != width {
            return Err(Error::new(format!(
                "CSV record {} has {} fields; expected {width}",
                self.record,
                row.len()
            )));
        }
        Ok(())
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ValueKind {
    Text,
    Number,
    Boolean,
    Binary,
    Json,
}
#[derive(Clone, Deserialize, Serialize)]
pub struct Mapping {
    pub column: Option<String>,
    pub kind: ValueKind,
}
pub fn validate_mapping(mapping: &[Mapping], width: usize, columns: &[Column]) -> Result<()> {
    if mapping.len() != width || width > COLUMN_LIMIT {
        return Err(Error::new("Map every source column or mark it Ignore"));
    }
    let mut names = HashSet::new();
    for name in mapping.iter().filter_map(|m| m.column.as_ref()) {
        if !names.insert(name) {
            return Err(Error::new("Map each destination column only once"));
        }
        if !columns.iter().any(|c| &c.name == name && !c.generated) {
            return Err(Error::new(
                "Choose an existing, non-generated destination column",
            ));
        }
    }
    if names.is_empty() {
        return Err(Error::new("Map at least one destination column"));
    }
    Ok(())
}
pub fn insert(row: Vec<String>, mapping: &[Mapping], options: &ImportOptions) -> Result<Change> {
    if row.len() != mapping.len() {
        return Err(Error::new("CSV column count changed"));
    }
    let mut values = BTreeMap::new();
    for (value, field) in row.into_iter().zip(mapping) {
        let Some(column) = &field.column else {
            continue;
        };
        let value = if options.trim {
            value.trim().to_owned()
        } else {
            value
        };
        let cell = if options.null_value.as_ref() == Some(&value)
            || (options.empty_as_null && value.is_empty())
        {
            Cell::Null
        } else {
            convert_value(value, &field.kind, column)?
        };
        values.insert(column.clone(), cell);
    }
    Ok(Change::Insert { values })
}

fn convert_value(value: String, kind: &ValueKind, column: &str) -> Result<Cell> {
    Ok(match kind {
        ValueKind::Text => Cell::Text(value),
        ValueKind::Number => {
            let digits = value.strip_prefix(['-', '+']).unwrap_or(&value);
            if !(!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
                || value.parse::<f64>().is_ok_and(f64::is_finite))
            {
                return Err(Error::new(format!("Invalid number for column {column}")));
            }
            Cell::Number(value)
        }
        ValueKind::Boolean => match value.to_ascii_lowercase().as_str() {
            "true" | "1" => Cell::Boolean(true),
            "false" | "0" => Cell::Boolean(false),
            _ => {
                return Err(Error::new(format!(
                    "Use true/false or 1/0 for column {column}"
                )));
            }
        },
        ValueKind::Binary => {
            if !value.len().is_multiple_of(2) || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(Error::new(format!(
                    "Use hexadecimal bytes for column {column}"
                )));
            }
            Cell::Binary(value)
        }
        ValueKind::Json => Cell::Json(
            serde_json::from_str(&value)
                .map_err(|_| Error::new(format!("Invalid JSON for column {column}")))?,
        ),
    })
}

fn cell_text(cell: Cell) -> String {
    match cell {
        Cell::Text(value) | Cell::Number(value) | Cell::Binary(value) => value,
        cell => cell.text(),
    }
}

fn insert_cells(row: Vec<Cell>, mapping: &[Mapping], options: &ImportOptions) -> Result<Change> {
    if options.format == ImportFormat::Csv {
        return insert(row.into_iter().map(cell_text).collect(), mapping, options);
    }
    if row.len() != mapping.len() {
        return Err(Error::new("JSON column count changed"));
    }
    let mut values = BTreeMap::new();
    for (cell, field) in row.into_iter().zip(mapping) {
        let Some(column) = &field.column else {
            continue;
        };
        let converted = if cell == Cell::Null {
            Cell::Null
        } else if matches!(field.kind, ValueKind::Json) {
            Cell::Json(match cell {
                Cell::Json(value) => value,
                Cell::Text(value) | Cell::Binary(value) => serde_json::Value::String(value),
                Cell::Boolean(value) => serde_json::Value::Bool(value),
                Cell::Number(value) => serde_json::from_str(&value).map_err(error)?,
                Cell::Null => serde_json::Value::Null,
            })
        } else {
            convert_value(cell_text(cell), &field.kind, column)?
        };
        values.insert(column.clone(), converted);
    }
    Ok(Change::Insert { values })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn streaming_csv_contract() {
        let text = b"\xef\xbb\xbfid,name,note\r\n1,\"a,\"\"b\nsecond\",\\N\r\n2,\"\",last";
        let mut reader = CsvReader::new(&text[..], b',').unwrap();
        assert_eq!(reader.headers().unwrap(), ["id", "name", "note"]);
        assert_eq!(
            reader.next_record().unwrap().unwrap(),
            ["1", "a,\"b\nsecond", "\\N"]
        );
        assert_eq!(reader.next_record().unwrap().unwrap(), ["2", "", "last"]);
        assert!(reader.next_record().unwrap().is_none());
        for invalid in ["a\n\"unclosed", "a\nx\"y", "a\n\"x\"y", "a\n\"x\" y"] {
            let mut r = CsvReader::new(invalid.as_bytes(), b',').unwrap();
            r.headers().unwrap();
            assert!(r.next_record().is_err(), "{invalid}");
        }
        let huge = format!("a\n{}", "x".repeat(RECORD_LIMIT + 1));
        let mut r = CsvReader::new(huge.as_bytes(), b',').unwrap();
        r.headers().unwrap();
        assert!(r.next_record().unwrap_err().message.contains("8 MiB"));
        let many = format!("a\n{}", vec!["x"; COLUMN_LIMIT + 1].join(","));
        let mut r = CsvReader::new(many.as_bytes(), b',').unwrap();
        r.headers().unwrap();
        assert!(r.next_record().is_err());
        let mut r = CsvReader::new(&b"a\n\xff"[..], b',').unwrap();
        r.headers().unwrap();
        assert!(r.next_record().unwrap_err().message.contains("UTF-8"));
        let mut r = CsvReader::new(&b"a,b\n1"[..], b',').unwrap();
        let width = r.headers().unwrap().len();
        let row = r.next_record().unwrap().unwrap();
        assert!(r.check_width(&row, width).is_err());
        let mapping = vec![
            Mapping {
                column: Some("id".into()),
                kind: ValueKind::Number,
            },
            Mapping {
                column: Some("v".into()),
                kind: ValueKind::Text,
            },
        ];
        let options = ImportOptions {
            null_value: Some("\\N".into()),
            ..Default::default()
        };
        let change = insert(
            vec!["18446744073709551615".into(), "\\N".into()],
            &mapping,
            &options,
        )
        .unwrap();
        assert_eq!(
            change.values().unwrap()["id"],
            Cell::Number("18446744073709551615".into())
        );
        assert_eq!(change.values().unwrap()["v"], Cell::Null);
        assert!(insert(vec!["NaN".into(), "".into()], &mapping, &options).is_err());
        assert!(insert(vec!["--1".into(), "".into()], &mapping, &options).is_err());
        let json = "{\"n\":123456789012345678901234567890,\"d\":1.234567890123456789}";
        let change = insert(
            vec![json.into()],
            &[Mapping {
                column: Some("document".into()),
                kind: ValueKind::Json,
            }],
            &ImportOptions::default(),
        )
        .unwrap();
        let restored = change.values().unwrap()["document"].text();
        assert!(restored.contains("123456789012345678901234567890"));
        assert!(restored.contains("1.234567890123456789"));
        struct OneByte<'a>(&'a [u8]);
        impl Read for OneByte<'_> {
            fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
                let length = out.len().min(1);
                self.0.read(&mut out[..length])
            }
        }
        let mut fragmented = CsvReader::new(OneByte(text), b',').unwrap();
        assert_eq!(fragmented.headers().unwrap(), ["id", "name", "note"]);
        assert_eq!(
            fragmented.next_record().unwrap().unwrap()[1],
            "a,\"b\nsecond"
        );
        let boundary = format!("a\n{}", "x".repeat(RECORD_LIMIT));
        let mut r = CsvReader::new(boundary.as_bytes(), b',').unwrap();
        r.headers().unwrap();
        assert_eq!(r.next_record().unwrap().unwrap()[0].len(), RECORD_LIMIT);
        assert!(r.next_record().unwrap().is_none());
        let column = Column {
            name: "id".into(),
            data_type: "BIGINT".into(),
            nullable: false,
            primary_key: true,
            default: None,
            generated: false,
        };
        assert!(validate_mapping(&mapping[..1], 1, std::slice::from_ref(&column)).is_ok());
        assert!(
            validate_mapping(
                &[mapping[0].clone(), mapping[0].clone()],
                2,
                std::slice::from_ref(&column)
            )
            .is_err()
        );
        assert!(
            validate_mapping(
                &mapping[..1],
                1,
                &[Column {
                    generated: true,
                    ..column
                }]
            )
            .is_err()
        );
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("source");
        std::fs::write(&file, text).unwrap();
        let snapshot = Snapshot::copy(&file).unwrap();
        std::fs::write(&file, b"changed").unwrap();
        let preview = snapshot.preview(&ImportOptions::default()).unwrap();
        assert_eq!(preview.headers, ["id", "name", "note"]);
        assert_eq!(preview.rows.len(), 2);
    }
}
