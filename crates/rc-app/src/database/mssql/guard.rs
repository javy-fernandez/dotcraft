//! The read-only statement guard: one `SELECT` or `WITH … SELECT` statement, no keyword that
//! writes, changes the session or leaves the surrounding transaction. It is a first line of
//! defence; the statement still runs inside a transaction that is always rolled back.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuardError {
    Empty,
    /// The statement starts with this word instead of SELECT or WITH.
    NotSelect(String),
    Multiple,
    /// A keyword that is not allowed anywhere in a read-only query.
    Forbidden(String),
    Unterminated,
}

const FORBIDDEN: &[&str] = &[
    "INSERT", "UPDATE", "DELETE", "MERGE", "DROP", "ALTER", "CREATE", "TRUNCATE", "EXEC", "EXECUTE", "GRANT", "REVOKE", "DENY",
    "INTO", "BACKUP", "RESTORE", "DBCC", "SHUTDOWN", "KILL", "BULK", "OPENROWSET", "OPENDATASOURCE", "OPENQUERY", "USE", "SET",
    "DECLARE", "BEGIN", "COMMIT", "ROLLBACK", "SAVE", "RECONFIGURE", "GO", "WAITFOR", "CHECKPOINT", "ENABLE", "DISABLE", "RECEIVE",
    "SEND", "UPDATETEXT", "WRITETEXT", "READTEXT", "REVERT", "SETUSER", "TRAN", "TRANSACTION",
];

/// Upper-cased words outside comments, strings and quoted identifiers; `;` is kept as a token.
fn words(sql: &str) -> Result<Vec<String>, GuardError> {
    let chars: Vec<char> = sql.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        if c == '-' && next == Some('-') {
            while i < chars.len() && chars[i] != '\n' { i += 1; }
        } else if c == '/' && next == Some('*') {
            // T-SQL block comments nest.
            let mut depth = 0;
            loop {
                if i >= chars.len() { return Err(GuardError::Unterminated); }
                if chars[i] == '/' && chars.get(i + 1) == Some(&'*') { depth += 1; i += 2; }
                else if chars[i] == '*' && chars.get(i + 1) == Some(&'/') { depth -= 1; i += 2; if depth == 0 { break; } }
                else { i += 1; }
            }
        } else if c == '\'' || c == '"' || c == '[' {
            let close = if c == '[' { ']' } else { c };
            i += 1;
            loop {
                if i >= chars.len() { return Err(GuardError::Unterminated); }
                if chars[i] == close {
                    if chars.get(i + 1) == Some(&close) { i += 2; continue; }
                    i += 1;
                    break;
                }
                i += 1;
            }
            out.push(String::from(if c == '\'' { "'" } else { "\"" }));
        } else if c == ';' {
            out.push(";".into());
            i += 1;
        } else if c.is_alphanumeric() || c == '_' || c == '@' || c == '#' || c == '$' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || matches!(chars[i], '_' | '@' | '#' | '$')) { i += 1; }
            let word: String = chars[start..i].iter().collect();
            // `N'…'` Unicode literals: the N is part of the string.
            if word.eq_ignore_ascii_case("n") && chars.get(i) == Some(&'\'') { continue; }
            out.push(word.to_uppercase());
        } else {
            i += 1;
        }
    }
    Ok(out)
}

/// Checks that `sql` is a single read-only query.
pub fn check(sql: &str) -> Result<(), GuardError> {
    let mut words = words(sql)?;
    while words.last().is_some_and(|w| w == ";") { words.pop(); }
    let Some(first) = words.first() else { return Err(GuardError::Empty) };
    if first != "SELECT" && first != "WITH" { return Err(GuardError::NotSelect(first.clone())); }
    if words.iter().any(|w| w == ";") { return Err(GuardError::Multiple); }
    for (i, word) in words.iter().enumerate() {
        if word.starts_with('@') || word.starts_with('#') { continue; }
        if FORBIDDEN.contains(&word.as_str()) { return Err(GuardError::Forbidden(word.clone())); }
        // `NEXT VALUE FOR seq` advances a sequence, which a rollback does not undo.
        if word == "NEXT" && words.get(i + 1).is_some_and(|w| w == "VALUE") { return Err(GuardError::Forbidden("NEXT VALUE FOR".into())); }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_selects_and_ctes() {
        for sql in [
            "SELECT 1",
            "  select * from dbo.Orders where Note = 'DELETE FROM x; DROP TABLE y' ;  ",
            "-- leading comment\nWITH c AS (SELECT Id FROM [Update]) SELECT * FROM c",
            "SELECT [Insert], \"Set\", N'exec' FROM t WITH (NOLOCK) /* drop /* nested */ table */ ORDER BY 1 FOR XML PATH",
            "SELECT update_date, created_by FROM dbo.Audit",
            "SELECT TOP (10) o.Id, COUNT(*) OVER () FROM sales.Orders o JOIN sales.Customers c ON c.Id = o.CustomerId",
            "SELECT @@VERSION, DB_NAME()",
        ] {
            assert_eq!(check(sql), Ok(()), "{sql}");
        }
    }

    #[test]
    fn rejects_writes_batches_and_transaction_control() {
        let cases = [
            ("", GuardError::Empty),
            ("-- only a comment", GuardError::Empty),
            ("DELETE FROM t", GuardError::NotSelect("DELETE".into())),
            ("EXEC sp_who", GuardError::NotSelect("EXEC".into())),
            ("SELECT 1; SELECT 2", GuardError::Multiple),
            ("SELECT 1; COMMIT", GuardError::Multiple),
            ("SELECT * INTO copy FROM t", GuardError::Forbidden("INTO".into())),
            ("WITH x AS (SELECT 1 a) DELETE FROM x", GuardError::Forbidden("DELETE".into())),
            ("WITH x AS (SELECT 1 a) UPDATE x SET a = 2", GuardError::Forbidden("UPDATE".into())),
            ("SELECT 1 COMMIT", GuardError::Forbidden("COMMIT".into())),
            ("SELECT * FROM OPENROWSET('SQLNCLI', 'x', 'y')", GuardError::Forbidden("OPENROWSET".into())),
            ("SELECT NEXT VALUE FOR dbo.Seq", GuardError::Forbidden("NEXT VALUE FOR".into())),
            ("SELECT 'open", GuardError::Unterminated),
            ("SELECT 1 /* open", GuardError::Unterminated),
            ("SELECT [open", GuardError::Unterminated),
            ("SELECT 1\nGO\nSELECT 2", GuardError::Forbidden("GO".into())),
        ];
        for (sql, expected) in cases {
            assert_eq!(check(sql), Err(expected), "{sql}");
        }
    }
}
