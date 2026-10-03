use klyndb_driver_api::{Error, Result};
use sqlparser::{
    dialect::MsSqlDialect,
    tokenizer::{Token, Tokenizer},
};
use std::io::{BufRead, BufReader, Read};

#[derive(Default)]
enum Lexical {
    #[default]
    Normal,
    Quote(u8),
    Comment(usize),
}

/// Native T-SQL batches: semicolons do not end variable scope; standalone GO does.
pub struct MssqlReader<R: Read> {
    input: BufReader<R>,
    state: Lexical,
    line: u64,
}
impl<R: Read> MssqlReader<R> {
    pub fn new(input: R) -> Self {
        Self {
            input: BufReader::new(input),
            state: Lexical::Normal,
            line: 0,
        }
    }
    pub fn next_batch(&mut self, cancelled: impl Fn() -> bool) -> Result<Option<String>> {
        let mut batch = vec![];
        loop {
            if cancelled() {
                return Err(Error::new("SQL import cancelled"));
            }
            let mut line = vec![];
            let bytes = (&mut self.input)
                .take((super::SQL_LIMIT + 1) as u64)
                .read_until(b'\n', &mut line)
                .map_err(|e| Error::new(e.to_string()))?;
            if bytes == 0 {
                if !matches!(self.state, Lexical::Normal) {
                    return Err(Error::new(format!(
                        "Unterminated T-SQL quote/comment near line {}",
                        self.line
                    )));
                }
                return if batch.is_empty() {
                    Ok(None)
                } else {
                    text(batch).map(Some)
                };
            }
            self.line += 1;
            let original =
                std::str::from_utf8(&line).map_err(|_| Error::new("SQL files must use UTF-8"))?;
            let original = if self.line == 1 {
                original.trim_start_matches('\u{feff}')
            } else {
                original
            };
            if matches!(self.state, Lexical::Normal)
                && let Ok(tokens) = Tokenizer::new(&MsSqlDialect {}, original).tokenize()
            {
                let tokens: Vec<_> = tokens
                    .into_iter()
                    .filter(|t| !matches!(t, Token::Whitespace(_) | Token::EOF))
                    .collect();
                if matches!(tokens.first(), Some(Token::Word(w)) if w.quote_style.is_none() && w.value.eq_ignore_ascii_case("GO"))
                {
                    if tokens.len() != 1 {
                        return Err(Error::new(format!(
                            "Unsupported GO command near line {}. Use standalone GO without a repeat count or semicolon.",
                            self.line
                        )));
                    }
                    if batch.is_empty() {
                        continue;
                    }
                    return text(batch).map(Some);
                }
            }
            if batch.len() + original.len() > super::SQL_LIMIT {
                return Err(Error::new(
                    "A SQL Server batch exceeds 4 MiB. Separate independent batches with standalone GO lines.",
                ));
            }
            let line = original.as_bytes();
            let mut i = 0;
            while i < line.len() {
                let byte = line[i];
                let next = line.get(i + 1).copied();
                match &mut self.state {
                    Lexical::Quote(close) if byte == *close => {
                        if next == Some(*close) {
                            i += 1;
                        } else {
                            self.state = Lexical::Normal;
                        }
                    }
                    Lexical::Quote(_) => {}
                    Lexical::Comment(depth) => {
                        if byte == b'/' && next == Some(b'*') {
                            *depth += 1;
                            i += 1;
                        } else if byte == b'*' && next == Some(b'/') {
                            *depth -= 1;
                            i += 1;
                            if *depth == 0 {
                                self.state = Lexical::Normal;
                            }
                        }
                    }
                    Lexical::Normal => match (byte, next) {
                        (b'-', Some(b'-')) => break,
                        (b'/', Some(b'*')) => {
                            self.state = Lexical::Comment(1);
                            i += 1;
                        }
                        (b'\'' | b'"', _) => self.state = Lexical::Quote(byte),
                        (b'[', _) => self.state = Lexical::Quote(b']'),
                        _ => {}
                    },
                }
                i += 1;
            }
            batch.extend_from_slice(line);
        }
    }
}
fn text(bytes: Vec<u8>) -> Result<String> {
    String::from_utf8(bytes).map_err(|_| Error::new("SQL files must use UTF-8"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_batch_scope_and_boundaries() {
        let sql = "\u{feff}DECLARE @n int=42; SELECT @n;\r\nGO -- end\r\nSELECT N'one;\nGO\n雪', [a]];b]; /* outer\n/* GO */\nGO\n*/ SELECT 2;\n/* note */ gO /* end */\nSELECT 3";
        let mut reader = MssqlReader::new(sql.as_bytes());
        assert!(
            reader
                .next_batch(|| false)
                .unwrap()
                .unwrap()
                .contains("SELECT @n")
        );
        let second = reader.next_batch(|| false).unwrap().unwrap();
        assert!(second.contains("GO\n雪"));
        assert!(second.contains("*/ SELECT 2"));
        assert_eq!(reader.next_batch(|| false).unwrap().unwrap(), "SELECT 3");
        assert!(reader.next_batch(|| false).unwrap().is_none());
        for sql in [
            "SELECT 1\nGO 2\n",
            "SELECT 1\nGO;",
            "SELECT 'unclosed",
            "/* unclosed",
        ] {
            assert!(
                MssqlReader::new(sql.as_bytes())
                    .next_batch(|| false)
                    .is_err()
            );
        }
        let huge = "x".repeat(super::super::SQL_LIMIT + 1);
        assert!(
            MssqlReader::new(huge.as_bytes())
                .next_batch(|| false)
                .is_err()
        );
        assert!(
            MssqlReader::new(b"SELECT 1".as_slice())
                .next_batch(|| true)
                .is_err()
        );
    }
}
