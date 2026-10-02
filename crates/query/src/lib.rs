use klyndb_driver_api::{Error, Result};
use serde::Serialize;
use sqlparser::{
    ast::{Query, SetExpr, Statement, Visit, Visitor},
    dialect::{Dialect, GenericDialect, PostgreSqlDialect, SQLiteDialect},
    parser::Parser,
};
use std::ops::ControlFlow;

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
        self.read_only &= matches!(
            statement,
            Statement::Query(_) | Statement::Explain { analyze: false, .. }
        );
        ControlFlow::Continue(())
    }
    fn pre_visit_query(&mut self, query: &Query) -> ControlFlow<()> {
        self.read_only &= read_only_body(&query.body);
        ControlFlow::Continue(())
    }
}
pub fn analyze(sql: &str, engine: &str) -> Result<Analysis> {
    if sql.len() > 4 * 1024 * 1024 {
        return Err(Error::new(
            "SQL exceeds the 4 MiB editor limit. Import a file instead.",
        ));
    }
    let dialect: &dyn Dialect = match engine {
        "sqlite" => &SQLiteDialect {},
        "postgres" => &PostgreSqlDialect {},
        _ => &GenericDialect {},
    };
    let statements = Parser::parse_sql(dialect, sql)
        .map_err(|e| Error::new(format!("SQL could not be validated: {e}")))?;
    if statements.is_empty() {
        return Err(Error::new("Enter a SQL statement."));
    }
    let mut safety = Safety {
        warnings: vec![],
        read_only: true,
    };
    let _ = statements.visit(&mut safety);
    // Execute original SQL: AST formatting can change vendor syntax and comments.
    Ok(Analysis {
        statements: statements.iter().map(ToString::to_string).collect(),
        warnings: safety.warnings,
        read_only: safety.read_only,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
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
