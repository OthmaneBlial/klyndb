use klyndb_driver_api::{Error, Result as DriverResult};
use serde::Serialize;
use sqlparser::{
    ast::{Query, SetExpr, Statement, Visit, Visitor},
    dialect::{
        ClickHouseDialect, Dialect, DuckDbDialect, GenericDialect, MsSqlDialect, MySqlDialect,
        PostgreSqlDialect, SQLiteDialect,
    },
    parser::Parser,
    tokenizer::{Location, Token, TokenWithSpan, Tokenizer, Whitespace},
};
use std::ops::ControlFlow;
mod mssql;
pub mod plan;
pub use mssql::MssqlReader;
pub const SQL_LIMIT: usize = 4 * 1024 * 1024;

// Keep executable comments visible: their meaning varies with the server/version.
sqlparser::derive_dialect!(
    ValidatedMySqlDialect,
    MySqlDialect,
    preserve_type_id = true,
    overrides = { supports_multiline_comment_hints = false }
);
sqlparser::derive_dialect!(
    ValidatedDuckDbDialect,
    DuckDbDialect,
    preserve_type_id = true,
    overrides = { supports_nested_comments = true }
);

#[derive(Debug, Serialize)]
pub struct Analysis {
    pub statements: Vec<String>,
    pub warnings: Vec<String>,
    pub read_only: bool,
}
struct Safety {
    warnings: Vec<String>,
    read_only: bool,
}
fn executes_plan(statement: &Statement) -> bool {
    matches!(statement, Statement::Explain { analyze, options, .. }
    if *analyze || options.as_ref().is_some_and(|options| options.iter().any(|option| {
        option.name.value.eq_ignore_ascii_case("ANALYZE") && option.arg.as_ref().is_none_or(|arg|
            !["FALSE", "OFF", "0"].contains(&arg.to_string().trim_matches('\'').to_ascii_uppercase().as_str())
        )
    })))
}
fn read_only_body(body: &SetExpr) -> bool {
    match body {
        SetExpr::Select(s) => s.into.is_none(),
        SetExpr::Query(q) => read_only_body(&q.body),
        SetExpr::SetOperation { left, right, .. } => read_only_body(left) && read_only_body(right),
        SetExpr::Values(_) | SetExpr::Table(_) => true,
        _ => false,
    }
}
impl Visitor for Safety {
    type Break = ();
    fn pre_visit_statement(&mut self, statement: &Statement) -> ControlFlow<()> {
        let warning = match statement {
            Statement::Explain { .. } if executes_plan(statement) => {
                Some("ANALYZE executes the statement, including its writes and side effects")
            }
            Statement::Drop { .. } => Some("DROP removes database objects"),
            Statement::Truncate(_) => Some("TRUNCATE removes all rows"),
            Statement::Delete(delete) if delete.selection.is_none() => {
                Some("DELETE has no WHERE clause")
            }
            Statement::Update(update) if update.selection.is_none() => {
                Some("UPDATE has no WHERE clause")
            }
            _ => None,
        };
        if let Some(warning) = warning {
            self.warnings.push(warning.into());
        }
        self.read_only &= matches!(statement, Statement::Query(_))
            || matches!(statement, Statement::Explain { .. } if !executes_plan(statement))
            || matches!(statement, Statement::ShowVariable { variable } if variable.len() == 1 && variable[0].value.eq_ignore_ascii_case("WARNINGS"));
        ControlFlow::Continue(())
    }
    fn pre_visit_query(&mut self, query: &Query) -> ControlFlow<()> {
        self.read_only &= read_only_body(&query.body);
        ControlFlow::Continue(())
    }
}
fn parse(sql: &str, engine: &str) -> DriverResult<(Vec<Statement>, usize)> {
    if sql.len() > SQL_LIMIT {
        return Err(Error::new(
            "SQL exceeds the 4 MiB editor limit. Import a file instead.",
        ));
    }
    let dialect: &dyn Dialect = match engine {
        "sqlite" => &SQLiteDialect {},
        "postgres" => &PostgreSqlDialect {},
        "mysql" => &ValidatedMySqlDialect::new(),
        "duckdb" => &ValidatedDuckDbDialect::new(),
        "clickhouse" => &ClickHouseDialect {},
        "mssql" => &MsSqlDialect {},
        _ => &GenericDialect {},
    };
    let mut tokens = Tokenizer::new(dialect, sql)
        .tokenize_with_location()
        .map_err(|e| Error::new(format!("SQL could not be validated: {e}")))?;
    if engine == "mysql"
        && tokens.iter().any(|t| {
            matches!(
                &t.token,
                Token::Whitespace(Whitespace::MultiLineComment(comment))
                    if comment.starts_with('!') || comment.starts_with("M!")
            )
        })
    {
        return Err(Error::new(
            "Executable MySQL/MariaDB comments cannot be validated. Write their SQL explicitly.",
        ));
    }
    let end = tokens
        .iter()
        .rev()
        .find(|t| {
            !matches!(
                t.token,
                Token::Whitespace(_) | Token::SemiColon | Token::EOF
            )
        })
        .map_or(sql.len(), |t| byte_offset(sql, t.span.end));
    // MariaDB's runtime plan syntax is ANALYZE FORMAT=JSON, not EXPLAIN ANALYZE.
    // Normalize tokens for validation only; the original SQL is always executed.
    let leading: Vec<_> = tokens
        .iter()
        .filter(|t| !matches!(t.token, Token::Whitespace(_)))
        .take(4)
        .map(|t| t.token.to_string().to_ascii_uppercase())
        .collect();
    if engine == "mysql" && leading == ["ANALYZE", "FORMAT", "=", "JSON"] {
        tokens.insert(0, TokenWithSpan::wrap(Token::make_word("EXPLAIN", None)));
    }
    let statements = Parser::new(dialect)
        .with_tokens_with_locations(tokens)
        .parse_statements()
        .map_err(|e| Error::new(format!("SQL could not be validated: {e}")))?;
    Ok((statements, end))
}
fn byte_offset(sql: &str, target: Location) -> usize {
    let (mut line, mut column) = (1, 1);
    for (offset, c) in sql.char_indices() {
        if line == target.line && column == target.column {
            return offset;
        }
        if c == '\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }
    sql.len()
}
/// Preserve vendor syntax while excluding trailing delimiters/comments before adding plan SQL.
pub fn explain_target<'a>(sql: &'a str, engine: &str) -> DriverResult<&'a str> {
    let (statements, end) = parse(sql, engine)?;
    if statements.len() != 1
        || matches!(
            statements[0],
            Statement::Explain { .. } | Statement::ExplainTable { .. }
        )
    {
        return Err(Error::new(
            "Explain one statement or selection, without an existing EXPLAIN.",
        ));
    }
    Ok(&sql[..end])
}
pub fn sql_server_plan_target(sql: &str) -> DriverResult<()> {
    let (statements, _) = parse(sql, "mssql")?;
    if !matches!(
        statements.as_slice(),
        [Statement::Query(_)
            | Statement::Insert(_)
            | Statement::Update { .. }
            | Statement::Delete(_)]
    ) {
        return Err(Error::new(
            "SQL Server plans support one SELECT, INSERT, UPDATE or DELETE statement",
        ));
    }
    Ok(())
}
pub fn analyze(sql: &str, engine: &str) -> DriverResult<Analysis> {
    let statements = if engine == "mssql" {
        if sql.len() > SQL_LIMIT {
            return Err(Error::new(
                "SQL exceeds the 4 MiB editor limit. Import a file instead.",
            ));
        }
        let mut reader = MssqlReader::new(sql.as_bytes());
        let mut statements = vec![];
        while let Some(batch) = reader.next_batch(|| false)? {
            statements.extend(parse(&batch, engine)?.0);
        }
        statements
    } else {
        parse(sql, engine)?.0
    };
    if statements.is_empty() {
        return Err(Error::new("Enter a SQL statement."));
    }
    Ok(analyze_statements(&statements))
}
/// DuckDB returns a native Count result for DML without RETURNING.
/// Classification avoids confusing an ordinary SELECT column named Count with writes.
pub fn duckdb_count_result(sql: &str) -> DriverResult<bool> {
    let (statements, _) = parse(sql, "duckdb")?;
    Ok(
        matches!(statements.as_slice(), [Statement::Insert(i)] if i.returning.is_none())
            || matches!(statements.as_slice(), [Statement::Update(u)] if u.returning.is_none())
            || matches!(statements.as_slice(), [Statement::Delete(d)] if d.returning.is_none()),
    )
}
/// Native DuckDB column defaults also contain generated expressions. Resolve them from its DDL.
pub fn duckdb_generated_columns(
    sql: &str,
) -> DriverResult<std::collections::BTreeMap<String, bool>> {
    let (statements, _) = parse(sql, "duckdb")?;
    let [Statement::CreateTable(table)] = statements.as_slice() else {
        return Err(Error::new("Expected native DuckDB table DDL"));
    };
    Ok(table
        .columns
        .iter()
        .map(|column| {
            (
                column.name.value.clone(),
                column.options.iter().any(|option| {
                    matches!(
                        option.option,
                        sqlparser::ast::ColumnOption::Generated { .. }
                            | sqlparser::ast::ColumnOption::Identity(_)
                    )
                }),
            )
        })
        .collect())
}
fn analyze_statements(statements: &[Statement]) -> Analysis {
    let mut safety = Safety {
        warnings: vec![],
        read_only: true,
    };
    for statement in statements {
        if matches!(statement, Statement::Explain { .. }) && !executes_plan(statement) {
            continue;
        }
        let _ = statement.visit(&mut safety);
    }
    // Execute original SQL: AST formatting can change vendor syntax and comments.
    Analysis {
        statements: statements.iter().map(ToString::to_string).collect(),
        warnings: safety.warnings,
        read_only: safety.read_only,
    }
}

/// Validate file statements with the same safety rules as the editor.
/// Client commands and COPY streams need separate protocols, not simple SQL execution.
pub fn analyze_script(sql: &str, engine: &str) -> DriverResult<Option<Analysis>> {
    let (statements, _) = parse(sql, engine)?;
    if statements.is_empty() {
        return Ok(None);
    }
    if engine != "mssql" && statements.len() != 1 {
        return Err(Error::new(
            "SQL file framing produced more than one statement",
        ));
    }
    // Lexical settings can make server statement boundaries disagree with the file reader.
    for statement in &statements {
        if matches!(statement, Statement::Set(_)) {
            let normalized = statement.to_string().to_ascii_lowercase();
            if normalized.contains("sql_mode")
                || normalized.contains("standard_conforming_strings")
                || engine == "mssql"
                    && normalized.contains("quoted_identifier")
                    && normalized != "set quoted_identifier on"
            {
                return Err(Error::new(
                    "Changing SQL lexical settings inside imports is unsupported. Remove sql_mode/standard_conforming_strings/QUOTED_IDENTIFIER assignments.",
                ));
            }
        }
    }
    if matches!(
        statements[0],
        Statement::Copy {
            target: sqlparser::ast::CopyTarget::Stdin | sqlparser::ast::CopyTarget::Stdout,
            ..
        }
    ) {
        return Err(Error::new(
            "COPY STDIN/STDOUT dumps are not supported. Use SQL INSERT statements or CSV import instead.",
        ));
    }
    Ok(Some(analyze_statements(&statements)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mssql_preserves_batch_and_checks_native_writes() {
        let sql =
            "DECLARE @n bigint = 9223372036854775807; SELECT TOP (2) @n AS [exact]; SELECT N'é;雪'";
        assert_eq!(analyze(sql, "mssql").unwrap().statements.len(), 3);
        assert!(analyze("SELECT TOP (1) * FROM [dbo].[items] ORDER BY [id] OFFSET 1 ROWS FETCH NEXT 2 ROWS ONLY", "mssql").unwrap().read_only);
        assert!(
            !analyze("SELECT 1 INTO [dbo].[copy]", "mssql")
                .unwrap()
                .read_only
        );
        assert_eq!(
            analyze("UPDATE [dbo].[items] SET [label]=N'new'", "mssql")
                .unwrap()
                .warnings
                .len(),
            1
        );
    }
    #[test]
    fn explain_analyze_options_require_execution_confirmation() {
        let actual = analyze("EXPLAIN (ANALYZE, FORMAT JSON) SELECT 1", "postgres").unwrap();
        assert!(!actual.read_only);
        assert!(actual.warnings.iter().any(|w| w.contains("executes")));
        for option in ["FALSE", "OFF", "0"] {
            assert!(
                analyze(
                    &format!("EXPLAIN (ANALYZE {option}) UPDATE t SET x=1"),
                    "postgres"
                )
                .unwrap()
                .read_only
            );
        }
        let actual = analyze(
            "ANALYZE FORMAT=JSON UPDATE t SET x=1; SHOW WARNINGS",
            "mysql",
        )
        .unwrap();
        assert!(!actual.read_only);
        assert_eq!(actual.warnings.len(), 2);
        assert!(
            analyze(
                "EXPLAIN FORMAT=JSON UPDATE t SET x=1; SHOW WARNINGS",
                "mysql"
            )
            .unwrap()
            .read_only
        );
        assert_eq!(
            explain_target("SELECT 'é;--'\r\n /* hint */ + 1;;; -- trailing\n", "mysql").unwrap(),
            "SELECT 'é;--'\r\n /* hint */ + 1"
        );
        assert!(explain_target("SELECT 1; SELECT 2", "postgres").is_err());
        assert!(explain_target("EXPLAIN SELECT 1", "sqlite").is_err());
    }
    #[test]
    fn mysql_executable_comments_cannot_bypass_validation() {
        for sql in [
            "SELECT 1; /*!COMMIT */; /*!UPDATE t SET x=1 */",
            "SELECT 1; /*!50000 DROP TABLE t */",
            "SELECT 1; /*M! COMMIT */; /*M! UPDATE t SET x=1 */",
            "SELECT 1 /*M!100100 INTO OUTFILE '/tmp/hidden' */",
        ] {
            assert!(analyze(sql, "mysql").is_err(), "accepted: {sql}");
        }
        assert!(analyze("SELECT '/*! DROP */', '/*M! COMMIT */'; /* ordinary */ -- /*!\nSELECT /*+ MAX_EXECUTION_TIME(1000) */ 2", "mysql").unwrap().read_only);
        assert!(
            analyze("SELECT 1 /* ! ordinary */", "mysql")
                .unwrap()
                .read_only
        );
        assert!(
            analyze("SELECT 1 /*! ordinary on SQLite */", "sqlite")
                .unwrap()
                .read_only
        );
    }
    #[test]
    fn safety_checks_all_statements_and_ignores_literals() {
        assert_eq!(analyze("SELECT 'DROP table'; -- DROP\nUPDATE t SET x=1; DELETE FROM t WHERE x=2; DROP TABLE t", "sqlite").unwrap().warnings.len(), 2);
        assert_eq!(
            analyze("SELECT ';'; SELECT 2", "postgres")
                .unwrap()
                .statements
                .len(),
            2
        );
        assert!(analyze("", "sqlite").is_err());
        assert!(analyze("SELECT 'into'", "postgres").unwrap().read_only);
        assert!(!analyze("SELECT 1 INTO t", "postgres").unwrap().read_only);
        assert!(
            !analyze(
                "WITH d AS (DELETE FROM t RETURNING *) SELECT * FROM d",
                "postgres"
            )
            .unwrap()
            .read_only
        );
    }
}
