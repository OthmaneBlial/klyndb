use super::{COLUMN_LIMIT, ImportFormat, RECORD_LIMIT, error};
use klyndb_driver_api::{Cell, Error, Result};
use serde::{
    Deserialize, Deserializer,
    de::{self, MapAccess, SeqAccess, Visitor},
};
use serde_json::value::RawValue;
use std::{
    collections::HashSet,
    fmt,
    io::{BufReader, Chain, Cursor, Read},
};
use tokio_util::sync::CancellationToken;

/// Frames one bounded object from a top-level JSON array. Serde validates its
/// syntax; the framing layer prevents allocating an unbounded record first.
pub struct JsonReader<R: Read> {
    input: BufReader<Chain<Cursor<Vec<u8>>, R>>,
    format: ImportFormat,
    first: bool,
    done: bool,
    headers: Vec<String>,
    pending: Option<Vec<Cell>>,
    pub record: u64,
    pub(crate) cancel: CancellationToken,
    consumed: u64,
}
impl<R: Read> JsonReader<R> {
    pub fn new(mut input: R, format: ImportFormat) -> Result<Self> {
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
        let mut reader = Self {
            input: BufReader::with_capacity(64 * 1024, Cursor::new(prefix).chain(input)),
            format,
            first: true,
            done: false,
            headers: vec![],
            pending: None,
            record: 0,
            cancel: CancellationToken::new(),
            consumed: 0,
        };
        if reader.nonspace()? != Some(b'[') {
            return Err(Error::new("JSON import requires an array of objects"));
        }
        Ok(reader)
    }
    fn byte(&mut self) -> Result<Option<u8>> {
        if self.consumed.is_multiple_of(4096) && self.cancel.is_cancelled() {
            return Err(Error::new("Import cancelled"));
        }
        let mut byte = [0];
        let read = self.input.read(&mut byte).map_err(error)?;
        self.consumed += read as u64;
        Ok((read != 0).then_some(byte[0]))
    }
    fn nonspace(&mut self) -> Result<Option<u8>> {
        loop {
            match self.byte()? {
                Some(b' ' | b'\t' | b'\r' | b'\n') => {}
                byte => return Ok(byte),
            }
        }
    }
    fn finish(&mut self) -> Result<Option<Vec<Cell>>> {
        if self.nonspace()?.is_some() {
            return Err(Error::new("Trailing data after the JSON array"));
        }
        self.done = true;
        Ok(None)
    }
    pub fn headers(&mut self) -> Result<Vec<String>> {
        if self.headers.is_empty() {
            self.pending = self.read_record()?;
            if self.pending.is_none() {
                return Err(Error::new("The JSON array has no records"));
            }
        }
        Ok(self.headers.clone())
    }
    pub fn next_record(&mut self) -> Result<Option<Vec<Cell>>> {
        if self.cancel.is_cancelled() {
            return Err(Error::new("Import cancelled"));
        }
        if let Some(row) = self.pending.take() {
            return Ok(Some(row));
        }
        self.read_record()
            .map_err(|e| Error::new(format!("JSON record {}: {}", self.record + 1, e.message)))
    }
    fn read_record(&mut self) -> Result<Option<Vec<Cell>>> {
        if self.done {
            return Ok(None);
        }
        let mut start = self.nonspace()?;
        if self.first {
            if start == Some(b']') {
                return self.finish();
            }
            self.first = false;
        } else {
            match start {
                Some(b']') => return self.finish(),
                Some(b',') => start = self.nonspace()?,
                _ => return Err(Error::new("Expected a comma or closing JSON array bracket")),
            }
        }
        if start != Some(b'{') {
            return Err(Error::new("Each JSON record must be an object"));
        }
        let mut bytes = vec![b'{'];
        let mut stack = vec![b'}'];
        let (mut string, mut escaped) = (false, false);
        while !stack.is_empty() {
            let byte = self
                .byte()?
                .ok_or_else(|| Error::new("Unclosed JSON record"))?;
            if bytes.len() >= RECORD_LIMIT {
                return Err(Error::new("A JSON record exceeds 8 MiB"));
            }
            bytes.push(byte);
            if string {
                if escaped {
                    escaped = false;
                } else if byte == b'\\' {
                    escaped = true;
                } else if byte == b'"' {
                    string = false;
                }
            } else {
                match byte {
                    b'"' => string = true,
                    b'{' | b'[' => {
                        if stack.len() >= 128 {
                            return Err(Error::new("JSON nesting exceeds 128 levels"));
                        }
                        stack.push(if byte == b'{' { b'}' } else { b']' });
                    }
                    b'}' | b']' if stack.pop() != Some(byte) => {
                        return Err(Error::new("Mismatched JSON brackets"));
                    }
                    _ => {}
                }
            }
        }
        // Check duplicate keys at every depth before Value can silently overwrite them.
        serde_json::from_slice::<UniqueKeys>(&bytes)
            .map_err(|_| Error::new("Invalid JSON or duplicate object keys"))?;
        let Fields(fields) =
            serde_json::from_slice(&bytes).map_err(|_| Error::new("Invalid JSON record"))?;
        let (headers, row) = if self.format == ImportFormat::KlyndbJson {
            if fields.len() != 2 {
                return Err(Error::new("Klyndb JSON records require columns and values"));
            }
            let get = |name: &str| {
                fields
                    .iter()
                    .find(|(key, _)| key == name)
                    .map(|(_, value)| value.get())
                    .ok_or_else(|| Error::new("Klyndb JSON records require columns and values"))
            };
            let headers: Vec<String> = serde_json::from_str(get("columns")?)
                .map_err(|_| Error::new("Invalid Klyndb JSON columns"))?;
            let row: Vec<Cell> = serde_json::from_str(get("values")?)
                .map_err(|_| Error::new("Invalid Klyndb JSON values"))?;
            if headers.len() != row.len() {
                return Err(Error::new("Klyndb JSON column/value counts differ"));
            }
            (headers, row)
        } else {
            let mut headers = Vec::with_capacity(fields.len());
            let mut row = Vec::with_capacity(fields.len());
            for (name, raw) in fields {
                headers.push(name);
                let value = raw.get();
                row.push(match value.as_bytes()[0] {
                    b'n' => Cell::Null,
                    b'"' => Cell::Text(serde_json::from_str(value).map_err(error)?),
                    b't' | b'f' => Cell::Boolean(value == "true"),
                    b'{' | b'[' => Cell::Json(serde_json::from_str(value).map_err(error)?),
                    _ => Cell::Number(value.to_owned()),
                });
            }
            (headers, row)
        };
        if headers.is_empty()
            || headers.len() > COLUMN_LIMIT
            || headers.iter().any(|h| h.trim().is_empty() || h.len() > 256)
            || headers.iter().map(String::len).sum::<usize>() > 64 * 1024
        {
            return Err(Error::new(
                "JSON records require 1–1000 named fields, at most 256 bytes each and 64 KiB total",
            ));
        }
        let row = if self.headers.is_empty() {
            self.headers = headers;
            row
        } else if self.format == ImportFormat::KlyndbJson {
            if headers != self.headers {
                return Err(Error::new("Klyndb JSON columns changed between records"));
            }
            row
        } else {
            let mut fields: std::collections::BTreeMap<_, _> =
                headers.into_iter().zip(row).collect();
            if fields.len() != self.headers.len()
                || self.headers.iter().any(|h| !fields.contains_key(h))
            {
                return Err(Error::new(
                    "JSON field names changed; every record must have the same fields",
                ));
            }
            self.headers
                .iter()
                .map(|h| {
                    fields
                        .remove(h)
                        .ok_or_else(|| Error::new("Missing JSON field"))
                })
                .collect::<Result<_>>()?
        };
        self.record += 1;
        Ok(Some(row))
    }
}

struct Fields<'a>(Vec<(String, &'a RawValue)>);
impl<'de> Deserialize<'de> for Fields<'de> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct FieldVisitor;
        impl<'de> Visitor<'de> for FieldVisitor {
            type Value = Fields<'de>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a JSON object")
            }
            fn visit_map<M: MapAccess<'de>>(
                self,
                mut map: M,
            ) -> std::result::Result<Self::Value, M::Error> {
                let mut fields = vec![];
                while let Some(key) = map.next_key::<String>()? {
                    if fields.len() >= COLUMN_LIMIT {
                        return Err(de::Error::custom("Too many JSON fields"));
                    }
                    fields.push((key, map.next_value()?));
                }
                Ok(Fields(fields))
            }
        }
        deserializer.deserialize_map(FieldVisitor)
    }
}

struct UniqueKeys;
impl<'de> Deserialize<'de> for UniqueKeys {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct Check;
        impl<'de> Visitor<'de> for Check {
            type Value = UniqueKeys;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("JSON with unique object keys")
            }
            fn visit_map<M: MapAccess<'de>>(
                self,
                mut map: M,
            ) -> std::result::Result<Self::Value, M::Error> {
                let mut keys = HashSet::new();
                while let Some(key) = map.next_key::<String>()? {
                    if keys.len() >= COLUMN_LIMIT || !keys.insert(key) {
                        return Err(de::Error::custom("Duplicate or excessive JSON keys"));
                    }
                    map.next_value::<UniqueKeys>()?;
                }
                Ok(UniqueKeys)
            }
            fn visit_seq<S: SeqAccess<'de>>(
                self,
                mut seq: S,
            ) -> std::result::Result<Self::Value, S::Error> {
                while seq.next_element::<UniqueKeys>()?.is_some() {}
                Ok(UniqueKeys)
            }
            fn visit_bool<E: de::Error>(self, _: bool) -> std::result::Result<Self::Value, E> {
                Ok(UniqueKeys)
            }
            fn visit_i64<E: de::Error>(self, _: i64) -> std::result::Result<Self::Value, E> {
                Ok(UniqueKeys)
            }
            fn visit_u64<E: de::Error>(self, _: u64) -> std::result::Result<Self::Value, E> {
                Ok(UniqueKeys)
            }
            fn visit_f64<E: de::Error>(self, _: f64) -> std::result::Result<Self::Value, E> {
                Ok(UniqueKeys)
            }
            fn visit_str<E: de::Error>(self, _: &str) -> std::result::Result<Self::Value, E> {
                Ok(UniqueKeys)
            }
            fn visit_unit<E: de::Error>(self) -> std::result::Result<Self::Value, E> {
                Ok(UniqueKeys)
            }
        }
        deserializer.deserialize_any(Check)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ImportOptions, Mapping, Snapshot, ValueKind, insert_cells};
    #[test]
    fn bounded_json_precision_nulls_and_framing() {
        let input = r#"[{"n":18446744073709551615,"v":null,"j":{"d":1.234567890123456789},"s":"a,\"b\n😀"},{"s":"","j":[true,null],"v":"null","n":2}]"#;
        let mut reader = JsonReader::new(input.as_bytes(), ImportFormat::Json).unwrap();
        assert_eq!(reader.headers().unwrap(), ["n", "v", "j", "s"]);
        let row = reader.next_record().unwrap().unwrap();
        assert_eq!(row[0], Cell::Number("18446744073709551615".into()));
        assert_eq!(row[1], Cell::Null);
        assert!(row[2].text().contains("1.234567890123456789"));
        assert_eq!(row[3], Cell::Text("a,\"b\n😀".into()));
        let second = reader.next_record().unwrap().unwrap();
        assert_eq!(second[1], Cell::Text("null".into()));
        assert!(reader.next_record().unwrap().is_none());
        let options = ImportOptions {
            format: ImportFormat::Json,
            trim: true,
            empty_as_null: true,
            null_value: Some("null".into()),
            ..Default::default()
        };
        let mapping = [
            ValueKind::Number,
            ValueKind::Text,
            ValueKind::Json,
            ValueKind::Text,
        ]
        .into_iter()
        .enumerate()
        .map(|(i, kind)| Mapping {
            column: Some(i.to_string()),
            kind,
        })
        .collect::<Vec<_>>();
        let change = insert_cells(row, &mapping, &options).unwrap();
        assert_eq!(change.values().unwrap()["1"], Cell::Null);
        assert_eq!(
            insert_cells(second, &mapping, &options)
                .unwrap()
                .values()
                .unwrap()["3"],
            Cell::Text("".into())
        );
        let mapping = [Mapping {
            column: Some("doc".into()),
            kind: ValueKind::Json,
        }];
        assert_eq!(
            insert_cells(vec![Cell::Text("plain".into())], &mapping, &options)
                .unwrap()
                .values()
                .unwrap()["doc"],
            Cell::Json(serde_json::json!("plain"))
        );
        for invalid in [
            "{}",
            "[1]",
            "[]",
            "[{}]",
            "[{\"a\":1},]",
            "[{\"a\":1}] false",
            "[{\"a\":1} {\"a\":2}]",
            "[{\"a\":1},{\"b\":2}]",
            "[{\"a\":1},{\"a\":2,\"b\":3}]",
            "[{\"a\":1,\"a\":2}]",
            "[{\"a\":1,\"\\u0061\":2}]",
            "[{\"a\":{\"v\":1,\"v\":2}}]",
            "[{\"a\":\"unclosed}]",
            "[{\"a\": [1}}]",
            "[{\"a\":NaN}]",
            "[{\"a\":01}]",
            "[{\"a\":1}",
        ] {
            let result = (|| -> Result<()> {
                let mut r = JsonReader::new(invalid.as_bytes(), ImportFormat::Json)?;
                r.headers()?;
                while r.next_record()?.is_some() {}
                Ok(())
            })();
            assert!(result.is_err(), "{invalid}");
        }
        let huge = format!("[{{\"a\":\"{}\"}}]", "x".repeat(RECORD_LIMIT));
        assert!(
            JsonReader::new(huge.as_bytes(), ImportFormat::Json)
                .unwrap()
                .headers()
                .unwrap_err()
                .message
                .contains("8 MiB")
        );
        let many = format!(
            "[{{{}}}]",
            (0..=COLUMN_LIMIT)
                .map(|i| format!("\"f{i}\":0"))
                .collect::<Vec<_>>()
                .join(",")
        );
        assert!(
            JsonReader::new(many.as_bytes(), ImportFormat::Json)
                .unwrap()
                .headers()
                .is_err()
        );
        let deep = format!("[{{\"a\":{}0{}}}]", "[".repeat(128), "]".repeat(128));
        assert!(
            JsonReader::new(deep.as_bytes(), ImportFormat::Json)
                .unwrap()
                .headers()
                .is_err()
        );
        let invalid_utf8 = b"[{\"a\":\"\xff\"}]";
        assert!(
            JsonReader::new(&invalid_utf8[..], ImportFormat::Json)
                .unwrap()
                .headers()
                .is_err()
        );
        struct OneByte<'a>(&'a [u8]);
        impl Read for OneByte<'_> {
            fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
                let length = out.len().min(1);
                self.0.read(&mut out[..length])
            }
        }
        let bom = format!("\u{feff}{input}");
        let mut fragmented = JsonReader::new(OneByte(bom.as_bytes()), ImportFormat::Json).unwrap();
        fragmented.headers().unwrap();
        assert_eq!(
            fragmented.next_record().unwrap().unwrap()[0].text(),
            "18446744073709551615"
        );
        fragmented.cancel.cancel();
        assert!(fragmented.next_record().is_err());
        let native = r#"[{"columns":["a","a","doc"],"values":[{"kind":"number","value":"18446744073709551615"},{"kind":"null"},{"kind":"json","value":"plain"}]}]"#;
        let mut native = JsonReader::new(native.as_bytes(), ImportFormat::KlyndbJson).unwrap();
        assert_eq!(native.headers().unwrap(), ["a", "a", "doc"]);
        assert_eq!(
            native.next_record().unwrap().unwrap()[2],
            Cell::Json(serde_json::json!("plain"))
        );
        assert!(native.next_record().unwrap().is_none());
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("input.json");
        std::fs::write(&file, input).unwrap();
        let snapshot = Snapshot::copy(&file).unwrap();
        std::fs::write(&file, b"changed").unwrap();
        let preview = snapshot.preview(&options).unwrap();
        assert_eq!(preview.rows[0][1], None);
        assert_eq!(preview.rows[1][1], Some("null".into()));
        assert_eq!(preview.rows[1][3], Some("".into()));
    }
}
