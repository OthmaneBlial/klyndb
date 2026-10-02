use crate::error;
use klyndb_driver_api::{Error, Result};
use serde::Serialize;
use std::io::{BufRead, BufReader, Chain, Cursor, Read};
use tokio_util::sync::CancellationToken;

#[derive(Serialize)]
pub struct SqlPreview {
    pub statements: u64,
    pub sample: Vec<String>,
    pub warnings: Vec<String>,
}

enum Lexical {
    Normal,
    Quote { close: u8, escape: bool },
    LineComment,
    BlockComment(usize),
    Dollar(Vec<u8>),
}

/// Incremental UTF-8 SQL framing. Keep original text; validate each complete unit
/// with the editor parser before returning it. Memory is bounded by SQL_LIMIT.
pub struct SqlReader<R: Read> {
    input: BufReader<Chain<Cursor<Vec<u8>>, R>>,
    engine: String,
    cancel: CancellationToken,
    statement: u64,
    line: u64,
    warnings: Vec<String>,
}
impl<R: Read> SqlReader<R> {
    pub fn new(mut input: R, engine: &str) -> Result<Self> {
        if !["sqlite", "postgres", "mysql", "duckdb"].contains(&engine) {
            return Err(Error::new(
                "SQL file imports are unavailable for this engine",
            ));
        }
        let mut prefix = vec![];
        input
            .by_ref()
            .take(3)
            .read_to_end(&mut prefix)
            .map_err(error)?;
        if prefix == b"\xef\xbb\xbf" {
            prefix.clear();
        }
        Ok(Self {
            input: BufReader::new(Cursor::new(prefix).chain(input)),
            engine: engine.into(),
            cancel: CancellationToken::new(),
            statement: 0,
            line: 1,
            warnings: vec![],
        })
    }
    pub fn set_cancel(&mut self, cancel: CancellationToken) {
        self.cancel = cancel;
    }
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }
    fn peek(&mut self) -> Result<Option<u8>> {
        if self.cancel.is_cancelled() {
            return Err(Error::new("SQL import cancelled"));
        }
        Ok(self.input.fill_buf().map_err(error)?.first().copied())
    }
    fn byte(&mut self, sql: &mut Vec<u8>) -> Result<Option<u8>> {
        let Some(byte) = self.peek()? else {
            return Ok(None);
        };
        if sql.len() == klyndb_query::SQL_LIMIT {
            return Err(Error::new(
                "A SQL file statement exceeds 4 MiB. Split large INSERT statements into smaller batches.",
            ));
        }
        self.input.consume(1);
        self.line += u64::from(byte == b'\n');
        sql.push(byte);
        Ok(Some(byte))
    }
    pub fn next_statement(&mut self) -> Result<Option<String>> {
        let start = self.line;
        self.next().map_err(|e| {
            Error::new(format!(
                "SQL statement {} near line {start}: {}",
                self.statement + 1,
                e.message
            ))
        })
    }
    fn next(&mut self) -> Result<Option<String>> {
        let mut sql = vec![];
        let mut state = Lexical::Normal;
        let mut has_sql = false;
        loop {
            let Some(byte) = self.byte(&mut sql)? else {
                if !matches!(state, Lexical::Normal | Lexical::LineComment) {
                    return Err(Error::new("Unterminated SQL string, identifier or comment"));
                }
                if !has_sql {
                    std::str::from_utf8(&sql)
                        .map_err(|_| Error::new("SQL files must use UTF-8"))?;
                    return Ok(None);
                }
                return self.validate(sql);
            };
            match &mut state {
                Lexical::LineComment => {
                    if byte == b'\n' {
                        state = Lexical::Normal;
                    }
                }
                Lexical::BlockComment(depth) => {
                    if byte == b'*' && self.peek()? == Some(b'/') {
                        self.byte(&mut sql)?;
                        *depth -= 1;
                        if *depth == 0 {
                            state = Lexical::Normal;
                        }
                    } else if matches!(self.engine.as_str(), "postgres" | "duckdb")
                        && byte == b'/'
                        && self.peek()? == Some(b'*')
                    {
                        self.byte(&mut sql)?;
                        *depth += 1;
                    }
                }
                Lexical::Quote { close, escape } => {
                    if *escape && byte == b'\\' {
                        if self.byte(&mut sql)?.is_none() {
                            return Err(Error::new("Unterminated SQL escape"));
                        }
                    } else if byte == *close {
                        if *close != b']' && self.peek()? == Some(*close) {
                            self.byte(&mut sql)?;
                        } else {
                            state = Lexical::Normal;
                        }
                    }
                }
                Lexical::Dollar(tag) => {
                    if byte == b'$' && sql.ends_with(tag) {
                        state = Lexical::Normal;
                    }
                }
                Lexical::Normal => {
                    match byte {
                        b'/' if self.peek()? == Some(b'*') => {
                            self.byte(&mut sql)?;
                            let executable = self.engine == "mysql"
                                && (self.peek()? == Some(b'!')
                                    || self.peek()? == Some(b'M') && {
                                        self.byte(&mut sql)?;
                                        self.peek()? == Some(b'!')
                                    });
                            if executable {
                                return Err(Error::new(
                                    "Executable MySQL/MariaDB comments cannot be validated. Write their SQL explicitly.",
                                ));
                            }
                            state = Lexical::BlockComment(1);
                        }
                        b'-' if self.peek()? == Some(b'-') => {
                            self.byte(&mut sql)?;
                            if self.engine != "mysql"
                                || self
                                    .peek()?
                                    .is_none_or(|b| b.is_ascii_whitespace() || b.is_ascii_control())
                            {
                                state = Lexical::LineComment;
                            } else {
                                has_sql = true;
                            }
                        }
                        b'#' if self.engine == "mysql" => state = Lexical::LineComment,
                        b'\'' | b'"' => {
                            has_sql = true;
                            let escape = self.engine == "mysql" && matches!(byte, b'\'' | b'"')
                                || matches!(self.engine.as_str(), "postgres" | "duckdb")
                                    && byte == b'\''
                                    && sql.len() >= 2
                                    && matches!(sql[sql.len() - 2], b'e' | b'E')
                                    && (sql.len() == 2 || !identifier(sql[sql.len() - 3]));
                            state = Lexical::Quote {
                                close: if byte == b'[' && self.engine == "sqlite" {
                                    b']'
                                } else {
                                    byte
                                },
                                escape,
                            };
                        }
                        b'`' if matches!(self.engine.as_str(), "sqlite" | "mysql") => {
                            has_sql = true;
                            state = Lexical::Quote {
                                close: b'`',
                                escape: false,
                            };
                        }
                        b'[' if self.engine == "sqlite" => {
                            has_sql = true;
                            state = Lexical::Quote {
                                close: b']',
                                escape: false,
                            };
                        }
                        b'$' if matches!(self.engine.as_str(), "postgres" | "duckdb")
                            && (sql.len() == 1 || !identifier(sql[sql.len() - 2])) =>
                        {
                            has_sql = true;
                            let offset = sql.len() - 1;
                            while self
                                .peek()?
                                .is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_')
                            {
                                self.byte(&mut sql)?;
                            }
                            if self.peek()? == Some(b'$') {
                                self.byte(&mut sql)?;
                                state = Lexical::Dollar(sql[offset..].to_vec());
                            }
                        }
                        b';' => {
                            if has_sql {
                                return self.validate(sql);
                            }
                            // Empty statements/comments need no buffer and do not consume the statement limit together.
                            std::str::from_utf8(&sql)
                                .map_err(|_| Error::new("SQL files must use UTF-8"))?;
                            sql.clear();
                        }
                        b if !b.is_ascii_whitespace() => has_sql = true,
                        _ => {}
                    }
                }
            }
        }
    }
    fn validate(&mut self, sql: Vec<u8>) -> Result<Option<String>> {
        let sql = String::from_utf8(sql).map_err(|_| Error::new("SQL files must use UTF-8"))?;
        let Some(analysis) = klyndb_query::analyze_script(&sql, &self.engine)? else {
            return Ok(None);
        };
        self.warnings = analysis.warnings;
        self.statement += 1;
        Ok(Some(sql))
    }
}
fn identifier(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'$') || b >= 128
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sql_file_framing_contract() {
        struct OneByte<'a>(&'a [u8]);
        impl Read for OneByte<'_> {
            fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
                let n = out.len().min(1);
                self.0.read(&mut out[..n])
            }
        }
        let pg = "\u{feff}-- ;\n SELECT 'é;''--', E'escaped\\\';still', $tag$a;'/*$tag$; /* a /* ; */ b */ SELECT 2; -- trailing";
        let mut reader = SqlReader::new(OneByte(pg.as_bytes()), "postgres").unwrap();
        let first = reader.next_statement().unwrap().unwrap();
        assert!(first.contains("$tag$a;'/*$tag$"));
        assert!(
            reader
                .next_statement()
                .unwrap()
                .unwrap()
                .contains("SELECT 2")
        );
        assert!(reader.next_statement().unwrap().is_none());
        for (engine, script, count) in [
            (
                "mysql",
                "# ;\nSELECT 'a\\\';b', `a;b` FROM t; -- ;\nSELECT 2",
                2,
            ),
            (
                "sqlite",
                "SELECT 'a;''b', [semi;colon] FROM t; SELECT 2; /* end */",
                2,
            ),
            ("postgres", "SELECT foo$bar FROM t; SELECT $$one;two$$", 2),
            ("postgres", "SELECT 'backslash\\'; SELECT 2", 2),
        ] {
            let mut reader = SqlReader::new(script.as_bytes(), engine).unwrap();
            let mut seen = 0;
            while reader.next_statement().unwrap().is_some() {
                seen += 1;
            }
            assert_eq!(seen, count, "{script}");
        }
        for script in [
            "SELECT 'unclosed",
            "/* unclosed",
            "SELECT $tag$unclosed",
            "SELECT 1; /*!50000 DROP TABLE t */;",
        ] {
            let engine = if script.contains("/*!") {
                "mysql"
            } else {
                "postgres"
            };
            let mut reader = SqlReader::new(script.as_bytes(), engine).unwrap();
            loop {
                match reader.next_statement() {
                    Ok(Some(_)) => {}
                    Err(_) => break,
                    Ok(None) => panic!("accepted {script}"),
                }
            }
        }
        let too_large = format!("SELECT '{}'", "x".repeat(klyndb_query::SQL_LIMIT));
        assert!(
            SqlReader::new(too_large.as_bytes(), "sqlite")
                .unwrap()
                .next_statement()
                .unwrap_err()
                .message
                .contains("4 MiB")
        );
        let cancel = CancellationToken::new();
        cancel.cancel();
        let mut reader = SqlReader::new(&b"SELECT 1"[..], "sqlite").unwrap();
        reader.set_cancel(cancel);
        assert!(
            reader
                .next_statement()
                .unwrap_err()
                .message
                .contains("cancelled")
        );
        assert!(
            SqlReader::new(&b"SELECT '\xff'"[..], "sqlite")
                .unwrap()
                .next_statement()
                .is_err()
        );
    }
}
