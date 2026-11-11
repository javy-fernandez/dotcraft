//! SQL Server schema from INFORMATION_SCHEMA and sys views, mapped from plain result rows so
//! the mapping is testable without a server.

use crate::database::{Cell, Grid};
use std::collections::BTreeMap;

pub const TABLES: &str = "SELECT TABLE_SCHEMA, TABLE_NAME, TABLE_TYPE FROM INFORMATION_SCHEMA.TABLES ORDER BY TABLE_SCHEMA, TABLE_NAME";
pub const COLUMNS: &str = "SELECT TABLE_SCHEMA, TABLE_NAME, COLUMN_NAME, DATA_TYPE, CHARACTER_MAXIMUM_LENGTH, NUMERIC_PRECISION, NUMERIC_SCALE, IS_NULLABLE \
    FROM INFORMATION_SCHEMA.COLUMNS ORDER BY TABLE_SCHEMA, TABLE_NAME, ORDINAL_POSITION";
pub const KEYS: &str = "SELECT tc.TABLE_SCHEMA, tc.TABLE_NAME, tc.CONSTRAINT_NAME, tc.CONSTRAINT_TYPE, kcu.COLUMN_NAME \
    FROM INFORMATION_SCHEMA.TABLE_CONSTRAINTS tc JOIN INFORMATION_SCHEMA.KEY_COLUMN_USAGE kcu \
    ON kcu.CONSTRAINT_SCHEMA = tc.CONSTRAINT_SCHEMA AND kcu.CONSTRAINT_NAME = tc.CONSTRAINT_NAME \
    WHERE tc.CONSTRAINT_TYPE IN ('PRIMARY KEY', 'UNIQUE') ORDER BY tc.TABLE_SCHEMA, tc.TABLE_NAME, tc.CONSTRAINT_NAME, kcu.ORDINAL_POSITION";
pub const FOREIGN_KEYS: &str = "SELECT OBJECT_SCHEMA_NAME(fk.parent_object_id), OBJECT_NAME(fk.parent_object_id), fk.name, \
    COL_NAME(fkc.parent_object_id, fkc.parent_column_id), OBJECT_SCHEMA_NAME(fk.referenced_object_id), OBJECT_NAME(fk.referenced_object_id), \
    COL_NAME(fkc.referenced_object_id, fkc.referenced_column_id) \
    FROM sys.foreign_keys fk JOIN sys.foreign_key_columns fkc ON fkc.constraint_object_id = fk.object_id \
    ORDER BY 1, 2, fk.name, fkc.constraint_column_id";

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Column {
    pub name: String,
    /// `nvarchar(100)`, `decimal(18,2)`, `int`…
    pub data_type: String,
    pub nullable: bool,
    pub primary_key: bool,
    /// `schema.table.column` this column references.
    pub references: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Key {
    pub name: String,
    /// `PRIMARY KEY`, `UNIQUE` or `FOREIGN KEY`.
    pub kind: String,
    pub columns: Vec<String>,
    /// `schema.table` for foreign keys.
    pub target: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Table {
    pub schema: String,
    pub name: String,
    pub view: bool,
    pub columns: Vec<Column>,
    pub keys: Vec<Key>,
}

impl Table {
    pub fn qualified(&self) -> String { format!("{}.{}", self.schema, self.name) }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Schema {
    pub tables: Vec<Table>,
    pub truncated: bool,
}

impl Schema {
    /// Schema names in order, with their tables and views.
    pub fn by_schema(&self) -> BTreeMap<&str, Vec<&Table>> {
        let mut map: BTreeMap<&str, Vec<&Table>> = BTreeMap::new();
        for table in &self.tables { map.entry(table.schema.as_str()).or_default().push(table); }
        map
    }
}

fn text(cell: Option<&Cell>) -> String {
    match cell {
        Some(Cell::Text(s)) => s.clone(),
        Some(Cell::Integer(i)) => i.to_string(),
        _ => String::new(),
    }
}

fn int(cell: Option<&Cell>) -> Option<i64> {
    match cell { Some(Cell::Integer(i)) => Some(*i), Some(Cell::Text(s)) => s.parse().ok(), _ => None }
}

/// `nvarchar` + length → `nvarchar(100)` / `nvarchar(max)`; decimals keep precision and scale.
pub fn type_name(data_type: &str, length: Option<i64>, precision: Option<i64>, scale: Option<i64>) -> String {
    let lower = data_type.to_ascii_lowercase();
    match lower.as_str() {
        "char" | "varchar" | "nchar" | "nvarchar" | "binary" | "varbinary" => match length {
            Some(-1) => format!("{lower}(max)"),
            Some(n) => format!("{lower}({n})"),
            None => lower,
        },
        "decimal" | "numeric" => match (precision, scale) {
            (Some(p), Some(s)) => format!("{lower}({p},{s})"),
            _ => lower,
        },
        _ => lower,
    }
}

/// Builds the schema from the rows of [`TABLES`], [`COLUMNS`], [`KEYS`] and [`FOREIGN_KEYS`].
pub fn map(tables: &Grid, columns: &Grid, keys: &Grid, foreign_keys: &Grid, max_tables: usize) -> Schema {
    let mut schema = Schema { truncated: tables.truncated || tables.rows.len() > max_tables, ..Schema::default() };
    let mut index: BTreeMap<(String, String), usize> = BTreeMap::new();
    for row in tables.rows.iter().take(max_tables) {
        let table = Table { schema: text(row.first()), name: text(row.get(1)), view: text(row.get(2)).eq_ignore_ascii_case("VIEW"), ..Table::default() };
        index.insert((table.schema.clone(), table.name.clone()), schema.tables.len());
        schema.tables.push(table);
    }
    for row in &columns.rows {
        let Some(&i) = index.get(&(text(row.first()), text(row.get(1)))) else { continue };
        schema.tables[i].columns.push(Column {
            name: text(row.get(2)),
            data_type: type_name(&text(row.get(3)), int(row.get(4)), int(row.get(5)), int(row.get(6))),
            nullable: text(row.get(7)).eq_ignore_ascii_case("YES"),
            ..Column::default()
        });
    }
    fn add_key(table: &mut Table, name: String, kind: &str, column: String, target: Option<String>) {
        match table.keys.iter_mut().find(|k| k.name == name) {
            Some(key) => key.columns.push(column),
            None => table.keys.push(Key { name, kind: kind.into(), columns: vec![column], target }),
        }
    }
    for row in &keys.rows {
        let Some(&i) = index.get(&(text(row.first()), text(row.get(1)))) else { continue };
        add_key(&mut schema.tables[i], text(row.get(2)), &text(row.get(3)), text(row.get(4)), None);
    }
    for row in &foreign_keys.rows {
        let Some(&i) = index.get(&(text(row.first()), text(row.get(1)))) else { continue };
        let target = format!("{}.{}", text(row.get(4)), text(row.get(5)));
        add_key(&mut schema.tables[i], text(row.get(2)), "FOREIGN KEY", text(row.get(3)), Some(target.clone()));
        let column = text(row.get(3));
        if let Some(c) = schema.tables[i].columns.iter_mut().find(|c| c.name == column) {
            c.references = Some(format!("{target}.{}", text(row.get(6))));
        }
    }
    for table in &mut schema.tables {
        let pk: Vec<String> = table.keys.iter().filter(|k| k.kind == "PRIMARY KEY").flat_map(|k| k.columns.clone()).collect();
        for column in &mut table.columns { column.primary_key = pk.contains(&column.name); }
    }
    schema
}

/// `[schema].[name]` with `]` doubled.
pub fn quote(schema: &str, name: &str) -> String {
    format!("[{}].[{}]", schema.replace(']', "]]"), name.replace(']', "]]"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(rows: &[&[Cell]]) -> Grid {
        Grid { columns: Vec::new(), rows: rows.iter().map(|r| r.to_vec()).collect(), truncated: false }
    }
    fn t(s: &str) -> Cell { Cell::Text(s.into()) }

    #[test]
    fn maps_tables_columns_keys_and_references_from_fake_rows() {
        let tables = grid(&[&[t("dbo"), t("Customers"), t("BASE TABLE")], &[t("dbo"), t("Orders"), t("BASE TABLE")], &[t("sales"), t("TopCustomers"), t("VIEW")]]);
        let columns = grid(&[
            &[t("dbo"), t("Customers"), t("Id"), t("int"), Cell::Null, Cell::Integer(10), Cell::Integer(0), t("NO")],
            &[t("dbo"), t("Customers"), t("Email"), t("nvarchar"), Cell::Integer(256), Cell::Null, Cell::Null, t("YES")],
            &[t("dbo"), t("Orders"), t("Id"), t("int"), Cell::Null, Cell::Integer(10), Cell::Integer(0), t("NO")],
            &[t("dbo"), t("Orders"), t("CustomerId"), t("int"), Cell::Null, Cell::Integer(10), Cell::Integer(0), t("NO")],
            &[t("dbo"), t("Orders"), t("Total"), t("decimal"), Cell::Null, Cell::Integer(18), Cell::Integer(2), t("NO")],
            &[t("dbo"), t("Orders"), t("Notes"), t("nvarchar"), Cell::Integer(-1), Cell::Null, Cell::Null, t("YES")],
            &[t("dbo"), t("Missing"), t("X"), t("int"), Cell::Null, Cell::Null, Cell::Null, t("NO")],
        ]);
        let keys = grid(&[
            &[t("dbo"), t("Customers"), t("PK_Customers"), t("PRIMARY KEY"), t("Id")],
            &[t("dbo"), t("Customers"), t("UQ_Email"), t("UNIQUE"), t("Email")],
            &[t("dbo"), t("Orders"), t("PK_Orders"), t("PRIMARY KEY"), t("Id")],
        ]);
        let fks = grid(&[&[t("dbo"), t("Orders"), t("FK_Orders_Customers"), t("CustomerId"), t("dbo"), t("Customers"), t("Id")]]);
        let schema = map(&tables, &columns, &keys, &fks, 200);
        assert!(!schema.truncated);
        assert_eq!(schema.tables.len(), 3);
        let customers = &schema.tables[0];
        assert_eq!(customers.columns.iter().map(|c| (c.name.as_str(), c.data_type.as_str(), c.nullable, c.primary_key)).collect::<Vec<_>>(),
            [("Id", "int", false, true), ("Email", "nvarchar(256)", true, false)]);
        assert_eq!(customers.keys.len(), 2);
        let orders = &schema.tables[1];
        assert_eq!(orders.columns[2].data_type, "decimal(18,2)");
        assert_eq!(orders.columns[3].data_type, "nvarchar(max)");
        assert_eq!(orders.columns[1].references.as_deref(), Some("dbo.Customers.Id"));
        assert_eq!(orders.keys[1], Key { name: "FK_Orders_Customers".into(), kind: "FOREIGN KEY".into(), columns: vec!["CustomerId".into()], target: Some("dbo.Customers".into()) });
        assert!(schema.tables[2].view);
        assert_eq!(schema.by_schema().keys().copied().collect::<Vec<_>>(), ["dbo", "sales"]);
        assert!(map(&tables, &columns, &keys, &fks, 2).truncated);
    }

    #[test]
    fn quotes_identifiers() {
        assert_eq!(quote("dbo", "Odd]Name"), "[dbo].[Odd]]Name]");
    }
}
