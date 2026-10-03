use crate::State;
use klyndb_driver_api::{Cell, Error, KeyEntry, KeyInspection, KeyScan, KeyValue, Result};
use redis::Value;

fn invalid() -> Error {
    Error::new("Unexpected Redis response shape")
}
pub(crate) fn cursor(position: &str) -> Result<u64> {
    if position.is_empty() {
        return Ok(0);
    }
    if position.len() > 20 || !position.bytes().all(|c| c.is_ascii_digit()) {
        return Err(Error::new("Invalid Redis page cursor"));
    }
    position
        .parse()
        .map_err(|_| Error::new("Invalid Redis page cursor"))
}
pub(crate) fn key_bytes(key: &Cell) -> Result<Vec<u8>> {
    let bytes = match key {
        Cell::Text(text) if text.len() <= 65536 => text.as_bytes().to_vec(),
        Cell::Binary(hex) if hex.len() <= 131072 => {
            hex::decode(hex).map_err(|_| Error::new("Invalid binary key"))?
        }
        _ => {
            return Err(Error::new(
                "Keys must be text or hexadecimal bytes, at most 64 KiB",
            ));
        }
    };
    Ok(bytes)
}
fn cell(bytes: Vec<u8>) -> Cell {
    match String::from_utf8(bytes) {
        Ok(text) => Cell::Text(text),
        Err(error) => Cell::Binary(hex::encode(error.into_bytes())),
    }
}
fn array(value: Value) -> Result<Vec<Value>> {
    match value {
        Value::Array(values) => Ok(values),
        _ => Err(invalid()),
    }
}
fn text(value: Value) -> Result<String> {
    match value {
        Value::BulkString(bytes) => String::from_utf8(bytes).map_err(|_| invalid()),
        Value::SimpleString(text) => Ok(text),
        _ => Err(invalid()),
    }
}
fn integer(value: Value) -> Result<i64> {
    match value {
        Value::Int(n) => Ok(n),
        _ => Err(invalid()),
    }
}
fn bounded<T: serde::Serialize>(result: T) -> Result<T> {
    if serde_json::to_vec(&result).map_err(|_| invalid())?.len() > 4 * 1024 * 1024 {
        return Err(Error::new(
            "Redis reply exceeds the 4 MiB display limit; request a smaller range",
        ));
    }
    Ok(result)
}
pub(crate) fn value(value: Value) -> Result<KeyValue> {
    fn convert(value: Value, depth: usize, remaining: &mut usize) -> Result<KeyValue> {
        if depth > 32 || *remaining == 0 {
            return Err(Error::new(
                "Redis reply exceeds the display depth or 10,000-value limit",
            ));
        }
        *remaining -= 1;
        Ok(match value {
            Value::Nil => KeyValue::Cell(Cell::Null),
            Value::Int(n) => KeyValue::Cell(Cell::Number(n.to_string())),
            Value::BulkString(bytes) => KeyValue::Cell(cell(bytes)),
            Value::SimpleString(text) => KeyValue::Cell(Cell::Text(text)),
            Value::Okay => KeyValue::Cell(Cell::Text("OK".into())),
            Value::Array(values) => KeyValue::Array(
                values
                    .into_iter()
                    .map(|v| convert(v, depth + 1, remaining))
                    .collect::<Result<_>>()?,
            ),
            _ => {
                return Err(Error::new(
                    "This Redis response type is not supported by the RESP2 console",
                ));
            }
        })
    }
    bounded(convert(value, 0, &mut 10000)?)
}
fn scanned(value: Value) -> Result<(String, Vec<Value>)> {
    let mut parts = array(value)?.into_iter();
    let next = text(parts.next().ok_or_else(invalid)?)?;
    cursor(&next)?;
    let values = array(parts.next().ok_or_else(invalid)?)?;
    if parts.next().is_some() {
        return Err(invalid());
    }
    Ok((next, values))
}
pub(crate) async fn scan(state: &mut State, pattern: &str, position: u64) -> Result<KeyScan> {
    let (cursor, keys) = scanned(
        state
            .request(
                redis::cmd("SCAN")
                    .arg(position)
                    .arg("MATCH")
                    .arg(pattern)
                    .arg("COUNT")
                    .arg(100),
            )
            .await?,
    )?;
    // ponytail: COUNT is a server hint; reject oversized steps rather than silently losing keys behind a cursor.
    if keys.len() > 1000 {
        return Err(Error::new(
            "Redis returned more than 1,000 keys in one scan step; narrow the search pattern",
        ));
    }
    let mut bytes = Vec::with_capacity(keys.len());
    let mut metadata = redis::pipe();
    for key in keys {
        let Value::BulkString(key) = key else {
            return Err(invalid());
        };
        key_bytes(&cell(key.clone()))?;
        metadata.cmd("TYPE").arg(&key).cmd("PTTL").arg(&key);
        bytes.push(key);
    }
    if bytes.is_empty() {
        return Ok(KeyScan {
            cursor,
            keys: vec![],
        });
    }
    let mut values = state.pipeline(&metadata).await?.into_iter();
    let keys = bytes
        .into_iter()
        .map(|key| {
            Ok(KeyEntry {
                key: cell(key),
                data_type: text(values.next().ok_or_else(invalid)?)?,
                ttl_ms: integer(values.next().ok_or_else(invalid)?)?.to_string(),
            })
        })
        .collect::<Result<_>>()?;
    if values.next().is_some() {
        return Err(invalid());
    }
    bounded(KeyScan { cursor, keys })
}
pub(crate) async fn inspect(
    state: &mut State,
    key: Vec<u8>,
    position: &str,
) -> Result<KeyInspection> {
    let mut metadata = redis::pipe();
    metadata.cmd("TYPE").arg(&key).cmd("PTTL").arg(&key);
    let mut values = state.pipeline(&metadata).await?.into_iter();
    let entry = KeyEntry {
        key: cell(key.clone()),
        data_type: text(values.next().ok_or_else(invalid)?)?,
        ttl_ms: integer(values.next().ok_or_else(invalid)?)?.to_string(),
    };
    let mut next = None;
    let mut length = 0;
    let result = match entry.data_type.as_str() {
        "none" => Value::Nil,
        kind @ ("string" | "list" | "hash" | "set" | "zset" | "stream") => {
            let size_command = match kind {
                "string" => "STRLEN",
                "list" => "LLEN",
                "hash" => "HLEN",
                "set" => "SCARD",
                "zset" => "ZCARD",
                _ => "XLEN",
            };
            length = integer(state.request(redis::cmd(size_command).arg(&key)).await?)?;
            if length < 0 {
                return Err(invalid());
            }
            match kind {
                "string" | "list" => {
                    let offset = cursor(position)?;
                    let count = if kind == "string" { 65536 } else { 100 };
                    let end = offset
                        .checked_add(count - 1)
                        .filter(|n| *n <= i64::MAX as u64)
                        .ok_or_else(|| Error::new("Invalid value page position"))?;
                    let result = state
                        .request(
                            redis::cmd(if kind == "string" {
                                "GETRANGE"
                            } else {
                                "LRANGE"
                            })
                            .arg(&key)
                            .arg(offset)
                            .arg(end),
                        )
                        .await?;
                    let returned = match &result {
                        Value::BulkString(v) if kind == "string" => v.len(),
                        Value::Array(v) if kind == "list" => v.len(),
                        _ => return Err(invalid()),
                    } as u64;
                    if returned > 0 && offset + returned < length as u64 {
                        next = Some((offset + returned).to_string());
                    }
                    result
                }
                "stream" => {
                    let start = if position.is_empty() {
                        "-".into()
                    } else {
                        let (time, sequence) = position
                            .split_once('-')
                            .ok_or_else(|| Error::new("Invalid stream page position"))?;
                        cursor(time)?;
                        cursor(sequence)?;
                        if time.is_empty() || sequence.is_empty() {
                            return Err(Error::new("Invalid stream page position"));
                        }
                        format!("({position}")
                    };
                    let rows = array(
                        state
                            .request(
                                redis::cmd("XRANGE")
                                    .arg(&key)
                                    .arg(start)
                                    .arg("+")
                                    .arg("COUNT")
                                    .arg(100),
                            )
                            .await?,
                    )?;
                    if rows.len() == 100 {
                        let last = array(rows.last().ok_or_else(invalid)?.clone())?;
                        next = Some(text(last.first().ok_or_else(invalid)?.clone())?);
                    }
                    Value::Array(rows)
                }
                _ => {
                    let command = match kind {
                        "hash" => "HSCAN",
                        "set" => "SSCAN",
                        _ => "ZSCAN",
                    };
                    let (native_next, values) = scanned(
                        state
                            .request(
                                redis::cmd(command)
                                    .arg(&key)
                                    .arg(cursor(position)?)
                                    .arg("COUNT")
                                    .arg(100),
                            )
                            .await?,
                    )?;
                    if native_next != "0" {
                        next = Some(native_next);
                    }
                    Value::Array(values)
                }
            }
        }
        _ => {
            return Err(Error::new(
                "This Redis module key type does not have a native inspector yet",
            ));
        }
    };
    bounded(KeyInspection {
        entry,
        length: length.to_string(),
        position: position.into(),
        next,
        value: value(result)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_cells_binary_keys_and_reply_limits() {
        let bytes = vec![0, 255, b'a'];
        assert_eq!(key_bytes(&cell(bytes.clone())).unwrap(), bytes);
        assert!(key_bytes(&Cell::Number("1".into())).is_err());
        assert!(cursor("-1").is_err());
        assert!(cursor("+1").is_err());
        assert_eq!(cursor("18446744073709551615").unwrap(), u64::MAX);
        assert!(cursor("18446744073709551616").is_err());
        let KeyValue::Cell(Cell::Number(number)) = value(Value::Int(i64::MAX)).unwrap() else {
            panic!("integer type lost")
        };
        assert_eq!(number, "9223372036854775807");
        assert!(value(Value::Array(vec![Value::Nil; 10000])).is_err());
        assert!(value(Value::BulkString(vec![0; 1024 * 1024])).is_err());
        let mut deep = Value::Nil;
        for _ in 0..33 {
            deep = Value::Array(vec![deep]);
        }
        assert!(value(deep).is_err());
    }
}
