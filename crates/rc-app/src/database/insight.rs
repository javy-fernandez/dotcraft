//! Bounded, read-only SQLite inspection and local SQL suggestions.
use super::*;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone)]
pub struct Details {
    pub table: String,
    pub columns: Grid,
    pub indexes: Grid,
    pub foreign_keys: Grid,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Relation {
    pub from: String,
    pub to: String,
    pub from_column: String,
    pub to_column: String,
}

#[derive(Debug, Clone)]
pub struct Relations {
    pub tables: Vec<String>,
    pub edges: Vec<Relation>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ChangeKind { Added, Removed, Changed }

#[derive(Debug, Clone)]
pub struct SchemaChange {
    pub name: String,
    pub object_type: String,
    pub kind: ChangeKind,
    pub current: Option<String>,
    pub other: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Comparison {
    pub other: PathBuf,
    pub changes: Vec<SchemaChange>,
    pub unchanged: usize,
}

fn literal(value: &str) -> Result<String, Error> {
    ident(value)?;
    Ok(format!("'{}'", value.replace('\'', "''")))
}

pub(super) fn details(connection: &Connection, name: String, cancel: &AtomicBool, deadline: Instant) -> Result<Details, Error> {
    let exists: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name=?1 AND type IN ('table','view'))", [&name], |row| row.get(0))?;
    if !exists { return Err(Error::Input); }
    let quoted = literal(&name)?;
    let columns = query(connection, &format!("SELECT name, type, [notnull] AS required, dflt_value AS default_value, pk AS primary_key, hidden FROM pragma_table_xinfo({quoted})"), cancel, deadline)?;
    // The table-valued pragma quotes the index name as data, including unusual names.
    let indexes = query(connection, &format!("SELECT i.name, i.[unique], i.origin, i.partial, x.seqno, x.cid, x.name AS column_name FROM pragma_index_list({quoted}) AS i LEFT JOIN pragma_index_info(i.name) AS x ORDER BY i.name, x.seqno"), cancel, deadline)?;
    let foreign_keys = query(connection, &format!("SELECT id, seq, [table] AS target_table, [from] AS source_column, [to] AS target_column, on_update, on_delete FROM pragma_foreign_key_list({quoted}) ORDER BY id, seq"), cancel, deadline)?;
    Ok(Details { table: name, columns, indexes, foreign_keys })
}

pub(super) fn relations(connection: &Connection, cancel: &AtomicBool, deadline: Instant) -> Result<Relations, Error> {
    let names = query(connection, "SELECT name FROM sqlite_schema WHERE type='table' AND name NOT GLOB 'sqlite_*' ORDER BY name LIMIT 201", cancel, deadline)?;
    let mut result = Relations { tables: Vec::new(), edges: Vec::new(), truncated: names.truncated || names.rows.len() > MAX_TABLES };
    let mut bytes = 0usize;
    for row in names.rows.iter().take(MAX_TABLES) {
        let [Cell::Text(name)] = row.as_slice() else { return Err(Error::Input) };
        bytes += name.len();
        if bytes > MAX_BYTES { result.truncated = true; break; }
        result.tables.push(name.clone());
        let grid = query(connection, &format!("SELECT [table], [from], [to] FROM pragma_foreign_key_list({}) ORDER BY id, seq", literal(name)?), cancel, deadline)?;
        result.truncated |= grid.truncated;
        for row in grid.rows {
            if result.edges.len() >= MAX_ROWS { result.truncated = true; break; }
            if let [Cell::Text(target), Cell::Text(source), destination] = row.as_slice() {
                let to_column = match destination { Cell::Text(value) => value.clone(), _ => String::new() };
                bytes += name.len() + target.len() + source.len() + to_column.len();
                if bytes > MAX_BYTES { result.truncated = true; break; }
                result.edges.push(Relation { from: name.clone(), to: target.clone(), from_column: source.clone(), to_column });
            }
        }
    }
    Ok(result)
}

fn snapshot(connection: &Connection, cancel: &AtomicBool, deadline: Instant) -> Result<BTreeMap<(String, String), String>, Error> {
    let grid = query(connection, "SELECT type, name, sql FROM sqlite_schema WHERE name NOT GLOB 'sqlite_*' AND sql IS NOT NULL ORDER BY type, name", cancel, deadline)?;
    // Partial schemas must never be reported as identical or as complete differences.
    if grid.truncated { return Err(Error::Limit); }
    let mut result = BTreeMap::new();
    for row in grid.rows {
        let [Cell::Text(kind), Cell::Text(name), Cell::Text(sql)] = row.as_slice() else { return Err(Error::Input) };
        result.insert((kind.clone(), name.clone()), sql.clone());
    }
    Ok(result)
}

pub(super) fn compare(connection: &Connection, other: PathBuf, cancel: Arc<AtomicBool>, deadline: Instant) -> Result<Comparison, Error> {
    let other = other.canonicalize().map_err(|_| Error::Path)?;
    let second = open(&other, cancel.clone(), deadline)?;
    let current = snapshot(connection, &cancel, deadline)?;
    let compared = snapshot(&second, &cancel, deadline)?;
    let keys: BTreeSet<_> = current.keys().chain(compared.keys()).cloned().collect();
    let mut result = Comparison { other, changes: Vec::new(), unchanged: 0 };
    for (kind, name) in keys {
        check(&cancel, deadline)?;
        let key = (kind.clone(), name.clone());
        let before = current.get(&key);
        let after = compared.get(&key);
        if before == after { result.unchanged += 1; continue; }
        let change = match (before, after) { (None, Some(_)) => ChangeKind::Added, (Some(_), None) => ChangeKind::Removed, _ => ChangeKind::Changed };
        result.changes.push(SchemaChange { name, object_type: kind, kind: change, current: before.cloned(), other: after.cloned() });
    }
    Ok(result)
}

pub(super) fn explain(connection: &Connection, sql: String, cancel: &AtomicBool, deadline: Instant) -> Result<Grid, Error> {
    if sql.len() > MAX_SQL { return Err(Error::Limit); }
    check(cancel, deadline)?;
    let mut batch = rusqlite::Batch::new(connection, &sql);
    let statement = batch.next()?.ok_or(Error::Input)?;
    if batch.next()?.is_some() { return Err(Error::Input); }
    if !statement.readonly() || statement.column_count() == 0 { return Err(Error::ReadOnly); }
    query(connection, &format!("EXPLAIN QUERY PLAN {sql}"), cancel, deadline)
}

#[derive(Debug, Clone, PartialEq)]
pub struct Completion {
    pub label: String,
    pub replacement: String,
    pub start: usize,
    pub end: usize,
}

/// Explicit local suggestions from the loaded schema; no query or filesystem work.
/// Comments and quoted strings/identifiers are deliberately excluded.
pub fn completions(sql: &str, cursor: usize, tables: &[Table], details: Option<&Details>) -> Vec<Completion> {
    if sql.len() > MAX_SQL { return Vec::new(); }
    let chars: Vec<char> = sql.chars().collect();
    let end = cursor.min(chars.len());
    let (mut quote, mut line_comment, mut block_comment) = (None, false, false);
    let mut i = 0;
    while i < end {
        let c = chars[i]; let next = chars.get(i+1).copied();
        if line_comment { if c == '\n' { line_comment = false; } }
        else if block_comment { if c == '*' && next == Some('/') { block_comment = false; i += 1; } }
        else if let Some(q) = quote {
            if c == q { if next == Some(q) && i+1 < end { i += 1; } else { quote = None; } }
        } else if c == '-' && next == Some('-') { line_comment = true; i += 1; }
        else if c == '/' && next == Some('*') { block_comment = true; i += 1; }
        else if matches!(c, '\'' | '"' | '`') { quote = Some(c); }
        else if c == '[' { quote = Some(']'); }
        i += 1;
    }
    if quote.is_some() || line_comment || block_comment { return Vec::new(); }
    let start = chars[..end].iter().rposition(|c| !c.is_alphanumeric() && *c != '_').map_or(0, |i| i+1);
    let replace_end = end + chars[end..].iter().take_while(|c| c.is_alphanumeric() || **c == '_').count();
    let prefix: String = chars[start..end].iter().collect::<String>().to_lowercase();
    let mut words = BTreeMap::new();
    for word in ["SELECT", "FROM", "WHERE", "JOIN", "LEFT JOIN", "INNER JOIN", "ON", "GROUP BY", "ORDER BY", "HAVING", "LIMIT", "OFFSET", "DISTINCT", "AS", "AND", "OR", "IS NULL", "COUNT", "SUM", "AVG", "MIN", "MAX", "EXPLAIN QUERY PLAN"] {
        words.insert(word.to_string(), word.to_string());
    }
    for table in tables.iter().take(MAX_TABLES) {
        if let Ok(quoted) = ident(&table.name) { words.insert(table.name.clone(), quoted); }
    }
    if let Some(details) = details {
        for row in &details.columns.rows {
            if let Some(Cell::Text(name)) = row.first() && let Ok(quoted) = ident(name) { words.insert(name.clone(), quoted); }
        }
    }
    words.into_iter().filter(|(label,_)| label.to_lowercase().starts_with(&prefix)).take(24)
        .map(|(label,replacement)| Completion { label, replacement, start, end: replace_end }).collect()
}

pub fn apply_completion(sql: &mut String, completion: &Completion) -> Option<usize> {
    let bytes: Vec<usize> = sql.char_indices().map(|(i,_)| i).chain(std::iter::once(sql.len())).collect();
    let start = *bytes.get(completion.start)?;
    let end = *bytes.get(completion.end)?;
    if start > end || sql.len() - (end-start) + completion.replacement.len() > MAX_SQL { return None; }
    sql.replace_range(start..end, &completion.replacement);
    Some(completion.start + completion.replacement.chars().count())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("schema.sqlite");
        Connection::open(&path).unwrap().execute_batch("CREATE TABLE \"authors' list\"(id INTEGER PRIMARY KEY, name TEXT NOT NULL); CREATE TABLE books(id INTEGER PRIMARY KEY, author INTEGER REFERENCES \"authors' list\"(id) ON DELETE CASCADE, title TEXT, normalized TEXT GENERATED ALWAYS AS (lower(title)) VIRTUAL); CREATE INDEX title_idx ON books(title); CREATE VIEW titles AS SELECT title FROM books;").unwrap();
        (directory,path)
    }
    fn call(path: &Path, action: Action) -> Result<Reply, Error> { run(path,action,Arc::new(AtomicBool::new(false))) }

    #[test]
    fn sqlite_inspection_reports_columns_indexes_generated_columns_and_foreign_keys() {
        let (_dir,path) = fixture();
        let before = std::fs::read(&path).unwrap();
        let Reply::Details(details) = call(&path,Action::Inspect("books".into())).unwrap() else { panic!() };
        assert_eq!(details.columns.rows.len(),4);
        assert!(details.columns.rows.iter().any(|row| row[0] == Cell::Text("normalized".into()) && row[5] == Cell::Integer(2)));
        assert!(details.indexes.rows.iter().any(|row| row[0] == Cell::Text("title_idx".into()) && row[6] == Cell::Text("title".into())));
        assert_eq!(details.foreign_keys.rows[0][2], Cell::Text("authors' list".into()));
        assert_eq!(details.foreign_keys.rows[0][3], Cell::Text("author".into()));
        assert_eq!(details.foreign_keys.rows[0][6], Cell::Text("CASCADE".into()));
        assert!(call(&path, Action::Inspect("authors' list".into())).is_ok());
        assert!(call(&path, Action::Inspect("books'); DROP TABLE books;--".into())).is_err());
        for sql in ["PRAGMA writable_schema=1", "PRAGMA journal_mode=WAL", "PRAGMA foreign_keys=OFF", "ATTACH ':memory:' AS other", "DELETE FROM books"] {
            assert!(call(&path,Action::Query(sql.into())).is_err(),"{sql}");
        }
        assert_eq!(std::fs::read(path).unwrap(),before);
    }

    #[test]
    fn sqlite_schema_comparison_finds_changed_added_and_removed_objects_without_writes() {
        let (dir,path) = fixture();
        let other = dir.path().join("other.sqlite");
        std::fs::copy(&path,&other).unwrap();
        Connection::open(&other).unwrap().execute_batch("ALTER TABLE books ADD COLUMN isbn TEXT; DROP INDEX title_idx; CREATE INDEX author_idx ON books(author);").unwrap();
        let original = (std::fs::read(&path).unwrap(),std::fs::read(&other).unwrap());
        let Reply::Comparison(diff) = call(&path,Action::Compare(other.clone())).unwrap() else { panic!() };
        assert!(diff.unchanged >= 2);
        assert!(diff.changes.iter().any(|entry| entry.name=="books" && entry.kind==ChangeKind::Changed && entry.other.as_ref().unwrap().contains("isbn")));
        assert!(diff.changes.iter().any(|entry| entry.name=="title_idx" && entry.kind==ChangeKind::Removed));
        assert!(diff.changes.iter().any(|entry| entry.name=="author_idx" && entry.kind==ChangeKind::Added));
        let Reply::Comparison(equal) = call(&path,Action::Compare(path.clone())).unwrap() else { panic!() };
        assert!(equal.changes.is_empty());
        assert_eq!(original,(std::fs::read(&path).unwrap(),std::fs::read(&other).unwrap()));
        let missing=dir.path().join("missing.sqlite");
        assert_eq!(call(&path,Action::Compare(missing.clone())).unwrap_err(),Error::Path);
        assert!(!missing.exists());
    }

    #[test]
    fn sqlite_schema_comparison_refuses_partial_snapshots() {
        let (_dir,path) = fixture();
        let connection=Connection::open(&path).unwrap();
        for i in 0..MAX_ROWS { connection.execute_batch(&format!("CREATE TABLE t{i}(x)")).unwrap(); }
        assert_eq!(call(&path,Action::Compare(path.clone())).unwrap_err(),Error::Limit);
    }

    #[test]
    fn sqlite_relations_follow_declared_keys_and_cap_large_graphs() {
        let (_dir,path) = fixture();
        let Reply::Relations(graph) = call(&path,Action::Relations).unwrap() else { panic!() };
        assert_eq!(graph.tables.len(),2);
        assert_eq!(graph.edges,[Relation { from:"books".into(),to:"authors' list".into(),from_column:"author".into(),to_column:"id".into() }]);
        assert!(!graph.truncated);
        let connection=Connection::open(&path).unwrap();
        for i in 0..MAX_TABLES { connection.execute_batch(&format!("CREATE TABLE t{i}(x REFERENCES books(id))")).unwrap(); }
        let Reply::Relations(graph) = call(&path,Action::Relations).unwrap() else { panic!() };
        assert_eq!(graph.tables.len(),MAX_TABLES);
        assert!(graph.truncated);
    }

    #[test]
    fn sqlite_query_plan_uses_indexes_without_executing_or_accepting_writes() {
        let (_dir,path) = fixture();
        let original=std::fs::read(&path).unwrap();
        let Reply::Plan(plan)=call(&path,Action::Explain("SELECT * FROM books WHERE title='a'".into())).unwrap() else { panic!() };
        assert!(plan.rows.iter().flatten().any(|cell|matches!(cell,Cell::Text(value) if value.contains("title_idx"))));
        for sql in ["DELETE FROM books", "SELECT 1; SELECT 2", "PRAGMA user_version=1", "SELECT load_extension('missing')"] {
            assert!(call(&path,Action::Explain(sql.into())).is_err());
        }
        assert_eq!(original,std::fs::read(&path).unwrap());
    }

    #[test]
    fn sqlite_completion_uses_inspected_columns_and_quoted_names_at_unicode_carets() {
        let (_dir,path) = fixture();
        let Reply::Schema{tables,..}=call(&path,Action::Schema).unwrap() else {panic!()};
        let Reply::Details(details)=call(&path,Action::Inspect("books".into())).unwrap() else {panic!()};
        let options=completions("SELECT tit",10,&tables,Some(&details));
        let choice=options.iter().find(|choice|choice.label=="title").unwrap();
        let mut sql="SELECT tit".to_string();
        assert_eq!(apply_completion(&mut sql,choice),Some(14));
        assert_eq!(sql,"SELECT \"title\"");
        let mut sql="SELECT '🦀' FROM auth".to_string();
        let choice=completions(&sql,sql.chars().count(),&tables,None).into_iter().find(|choice|choice.label=="authors' list").unwrap();
        apply_completion(&mut sql,&choice).unwrap();
        assert_eq!(sql,"SELECT '🦀' FROM \"authors' list\"");
        for sql in ["-- SELECT tit", "SELECT 'tit", "/* tit", "SELECT [tit", "SELECT \"tit"] {
            assert!(completions(sql,sql.chars().count(),&tables,Some(&details)).is_empty(),"{sql}");
        }
        assert!(completions("sel",3,&tables,None).iter().any(|c| c.replacement=="SELECT"));
        let choice = completions("SELECT title FROM books", 3, &tables, None).into_iter().find(|c|c.replacement=="SELECT").unwrap();
        let mut middle = "SELECT title FROM books".to_string();
        apply_completion(&mut middle,&choice).unwrap();
        assert_eq!(middle,"SELECT title FROM books");
        assert!(completions("",0,&tables,None).len()<=24);
    }

    #[test]
    fn sqlite_inspection_actions_respect_cancellation_and_expired_deadlines() {
        let (_dir,path)=fixture();
        for action in [Action::Inspect("books".into()),Action::Relations,Action::Compare(path.clone()),Action::Explain("SELECT 1".into())] {
            assert_eq!(run(&path,action.clone(),Arc::new(AtomicBool::new(true))).unwrap_err(),Error::Cancelled);
            assert_eq!(run_until(&path,action,Arc::new(AtomicBool::new(false)),Instant::now()).unwrap_err(),Error::TimedOut);
        }
    }
}
