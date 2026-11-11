//! The tiberius side: TCP + TLS (rustls) connection, read-only execution and value mapping.

use super::{MsError, ServerInfo, conn::{Auth, ConnSpec}, plan, schema};
use crate::database::{Cell, Grid, MAX_BYTES, MAX_ROWS};
use futures_util::TryStreamExt;
use tiberius::{AuthMethod, Client, ColumnData, Config, EncryptionLevel, QueryStream, SqlBrowser};
use tokio::net::TcpStream;
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};

pub type Conn = Client<Compat<TcpStream>>;

fn server_error(e: tiberius::error::Error) -> MsError {
    let text = match &e {
        tiberius::error::Error::Server(token) => format!("{} (error {}, line {})", token.message(), token.code(), token.line()),
        other => other.to_string(),
    };
    MsError::Server(text.chars().take(2048).collect())
}

fn config(spec: &ConnSpec) -> Result<Config, MsError> {
    let mut config = Config::new();
    config.host(&spec.host);
    if let Some(port) = spec.port { config.port(port); }
    if let Some(instance) = &spec.instance { config.instance_name(instance); }
    if !spec.database.is_empty() { config.database(&spec.database); }
    config.application_name("DotCraft");
    // ApplicationIntent=ReadOnly: routes to readable secondaries in availability groups.
    config.readonly(true);
    config.encryption(if spec.no_encrypt { EncryptionLevel::Off } else { EncryptionLevel::Required });
    if spec.trust_cert { config.trust_cert(); }
    match spec.auth {
        Auth::Sql => config.authentication(AuthMethod::sql_server(&spec.user, &spec.password)),
        #[cfg(windows)]
        Auth::Integrated => config.authentication(AuthMethod::Integrated),
        #[cfg(not(windows))]
        Auth::Integrated => return Err(MsError::IntegratedUnavailable),
    }
    Ok(config)
}

async fn open(config: Config) -> Result<Conn, tiberius::error::Error> {
    // Asks SQL Browser (UDP 1434) for a named instance's port; otherwise connects to host:port.
    let tcp = TcpStream::connect_named(&config).await?;
    tcp.set_nodelay(true)?;
    Client::connect(config, tcp.compat_write()).await
}

pub async fn connect(spec: &ConnSpec) -> Result<(Conn, ServerInfo), MsError> {
    let mut cfg = config(spec)?;
    let mut client = match open(cfg.clone()).await {
        // Azure SQL gateways redirect to the node that holds the database.
        Err(tiberius::error::Error::Routing { host, port }) => {
            cfg.host(&host);
            cfg.port(port);
            open(cfg).await
        }
        other => other,
    }
    .map_err(|e| match e {
        tiberius::error::Error::Io { message, .. } => MsError::Connect(message),
        tiberius::error::Error::Tls(message) => MsError::Tls(message),
        other => server_error(other),
    })?;
    drain(client.simple_query("SET LOCK_TIMEOUT 10000; SET IMPLICIT_TRANSACTIONS OFF; SET TRANSACTION ISOLATION LEVEL READ COMMITTED;").await.map_err(server_error)?).await?;
    let grid = read(client.simple_query(
        "SELECT CAST(SERVERPROPERTY('ProductVersion') AS nvarchar(128)), CAST(SERVERPROPERTY('Edition') AS nvarchar(128)), DB_NAME(), SUSER_SNAME(), \
         CAST(ISNULL(IS_ROLEMEMBER('db_owner'), 0) | ISNULL(IS_ROLEMEMBER('db_datawriter'), 0) | ISNULL(IS_ROLEMEMBER('db_ddladmin'), 0) | ISNULL(IS_SRVROLEMEMBER('sysadmin'), 0) AS int)",
    ).await.map_err(server_error)?).await?;
    let row = grid.rows.first().cloned().unwrap_or_default();
    let text = |i: usize| match row.get(i) { Some(Cell::Text(s)) => s.clone(), _ => String::new() };
    let info = ServerInfo { version: text(0), edition: text(1), database: text(2), login: text(3), can_write: matches!(row.get(4), Some(Cell::Integer(n)) if *n != 0) };
    Ok((client, info))
}

async fn drain(stream: QueryStream<'_>) -> Result<(), MsError> {
    stream.into_results().await.map(|_| ()).map_err(server_error)
}

/// Reads the first result set, bounded by [`MAX_ROWS`] and [`MAX_BYTES`].
async fn read(mut stream: QueryStream<'_>) -> Result<Grid, MsError> {
    let columns: Vec<String> = stream.columns().await.map_err(server_error)?.map(|c| c.iter().map(|c| c.name().to_string()).collect()).unwrap_or_default();
    let mut grid = Grid { columns, ..Grid::default() };
    let mut bytes: usize = grid.columns.iter().map(String::len).sum();
    let mut rows = stream.into_row_stream();
    while let Some(row) = rows.try_next().await.map_err(server_error)? {
        if grid.truncated { continue; }
        if grid.rows.len() >= MAX_ROWS { grid.truncated = true; continue; }
        let cells: Vec<Cell> = row.into_iter().map(cell).collect();
        bytes = bytes.saturating_add(cells.iter().map(|c| match c { Cell::Text(s) => s.len().max(16), Cell::Blob(b) => b.len().max(16), _ => 16 }).sum());
        if bytes > MAX_BYTES { grid.truncated = true; continue; }
        grid.rows.push(cells);
    }
    Ok(grid)
}

/// Runs one guarded statement inside a transaction that is always rolled back. `SET ROWCOUNT`
/// stops the server after one row more than the grid shows.
pub async fn query(client: &mut Conn, sql: &str) -> Result<Grid, MsError> {
    drain(client.simple_query(format!("SET ROWCOUNT {}; SET XACT_ABORT ON; BEGIN TRANSACTION;", MAX_ROWS + 1)).await.map_err(server_error)?).await?;
    let result = match client.simple_query(sql).await {
        Ok(stream) => read(stream).await,
        Err(e) => Err(server_error(e)),
    };
    let rollback = drain(client.simple_query("IF @@TRANCOUNT > 0 ROLLBACK TRANSACTION; SET ROWCOUNT 0; SET XACT_ABORT OFF;").await.map_err(server_error)?).await;
    let grid = result?;
    rollback?;
    Ok(grid)
}

/// The estimated plan: the statement is compiled, not executed.
pub async fn plan(client: &mut Conn, sql: &str) -> Result<plan::Plan, MsError> {
    drain(client.simple_query("SET SHOWPLAN_XML ON").await.map_err(server_error)?).await?;
    let result = match client.simple_query(sql).await {
        Ok(stream) => stream.into_first_result().await.map_err(server_error),
        Err(e) => Err(server_error(e)),
    };
    drain(client.simple_query("SET SHOWPLAN_XML OFF").await.map_err(server_error)?).await?;
    let xml = result?.into_iter().next().and_then(|row| row.into_iter().next()).and_then(|c| match cell(c) { Cell::Text(s) => Some(s), _ => None }).ok_or(MsError::Server("the server returned no plan".into()))?;
    plan::parse(&xml).map_err(MsError::Server)
}

pub async fn schema(client: &mut Conn, max_tables: usize) -> Result<schema::Schema, MsError> {
    let tables = query_all(client, schema::TABLES).await?;
    let columns = query_all(client, schema::COLUMNS).await?;
    let keys = query_all(client, schema::KEYS).await?;
    let fks = query_all(client, schema::FOREIGN_KEYS).await?;
    Ok(schema::map(&tables, &columns, &keys, &fks, max_tables))
}

/// Metadata queries are not bounded by the result grid's row limit.
async fn query_all(client: &mut Conn, sql: &str) -> Result<Grid, MsError> {
    let rows = client.simple_query(sql).await.map_err(server_error)?.into_first_result().await.map_err(server_error)?;
    Ok(Grid { columns: Vec::new(), rows: rows.into_iter().take(50_000).map(|row| row.into_iter().map(cell).collect()).collect(), truncated: false })
}

// ---- values ------------------------------------------------------------------------------

/// Proleptic Gregorian date for days since 1970-01-01 (H. Hinnant's `civil_from_days`).
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

const DAYS_0001_TO_1970: i64 = 719_162;
const DAYS_1900_TO_1970: i64 = 25_567;

fn date(days_since_1970: i64) -> String {
    let (y, m, d) = civil(days_since_1970);
    format!("{y:04}-{m:02}-{d:02}")
}

fn time(increments: u64, scale: u8) -> String {
    let unit = 10u64.pow(u32::from(scale));
    let secs = increments / unit;
    let frac = increments % unit;
    let base = format!("{:02}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60);
    if scale == 0 { base } else { format!("{base}.{frac:0width$}", width = scale as usize) }
}

pub fn cell(value: ColumnData<'static>) -> Cell {
    match value {
        ColumnData::U8(Some(v)) => Cell::Integer(v.into()),
        ColumnData::I16(Some(v)) => Cell::Integer(v.into()),
        ColumnData::I32(Some(v)) => Cell::Integer(v.into()),
        ColumnData::I64(Some(v)) => Cell::Integer(v),
        ColumnData::F32(Some(v)) => Cell::Real(v.into()),
        ColumnData::F64(Some(v)) => Cell::Real(v),
        ColumnData::Bit(Some(v)) => Cell::Integer(v.into()),
        ColumnData::String(Some(v)) => Cell::Text(v.into_owned()),
        ColumnData::Guid(Some(v)) => Cell::Text(v.to_string().to_uppercase()),
        ColumnData::Binary(Some(v)) => Cell::Blob(v.into_owned()),
        ColumnData::Numeric(Some(v)) => Cell::Text(v.to_string()),
        ColumnData::Xml(Some(v)) => Cell::Text(v.to_string()),
        ColumnData::DateTime(Some(v)) => {
            // 1/300 s ticks since midnight.
            let ms = u64::from(v.seconds_fragments()) * 10 / 3;
            Cell::Text(format!("{} {}", date(i64::from(v.days()) - DAYS_1900_TO_1970), time(ms, 3)))
        }
        ColumnData::SmallDateTime(Some(v)) => {
            let minutes = u64::from(v.seconds_fragments());
            Cell::Text(format!("{} {:02}:{:02}", date(i64::from(v.days()) - DAYS_1900_TO_1970), minutes / 60, minutes % 60))
        }
        ColumnData::Date(Some(v)) => Cell::Text(date(i64::from(v.days()) - DAYS_0001_TO_1970)),
        ColumnData::Time(Some(v)) => Cell::Text(time(v.increments(), v.scale())),
        ColumnData::DateTime2(Some(v)) => Cell::Text(format!("{} {}", date(i64::from(v.date().days()) - DAYS_0001_TO_1970), time(v.time().increments(), v.time().scale()))),
        ColumnData::DateTimeOffset(Some(v)) => {
            // Stored as UTC; shown in the value's own offset.
            let dt = v.datetime2();
            let offset = i64::from(v.offset());
            let unit = 10i64.pow(u32::from(dt.time().scale()));
            let total = i64::from(dt.date().days() as i32) * 86_400 * unit + dt.time().increments() as i64 + offset * 60 * unit;
            let day = total.div_euclid(86_400 * unit);
            let rest = total.rem_euclid(86_400 * unit) as u64;
            let sign = if offset < 0 { '-' } else { '+' };
            Cell::Text(format!("{} {} {sign}{:02}:{:02}", date(day - DAYS_0001_TO_1970), time(rest, dt.time().scale()), offset.abs() / 60, offset.abs() % 60))
        }
        _ => Cell::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tiberius::time::{Date, DateTime, DateTime2, DateTimeOffset, SmallDateTime, Time};

    #[test]
    fn maps_sql_server_values_to_cells() {
        assert_eq!(cell(ColumnData::I32(Some(7))), Cell::Integer(7));
        assert_eq!(cell(ColumnData::Bit(Some(true))), Cell::Integer(1));
        assert_eq!(cell(ColumnData::String(None)), Cell::Null);
        assert_eq!(cell(ColumnData::Numeric(Some(tiberius::numeric::Numeric::new_with_scale(6485, 2)))), Cell::Text("64.85".into()));
        // 2026-11-01 is 739920 days after 0001-01-01.
        let date_value = Date::new(739_920);
        assert_eq!(cell(ColumnData::Date(Some(date_value))), Cell::Text("2026-11-01".into()));
        assert_eq!(cell(ColumnData::Time(Some(Time::new(453_000_000, 7)))), Cell::Text("00:00:45.3000000".into()));
        assert_eq!(cell(ColumnData::DateTime2(Some(DateTime2::new(date_value, Time::new(3_723, 0))))), Cell::Text("2026-11-01 01:02:03".into()));
        // 1900-01-01 + 300 ticks = one second.
        assert_eq!(cell(ColumnData::DateTime(Some(DateTime::new(0, 300)))), Cell::Text("1900-01-01 00:00:01.000".into()));
        assert_eq!(cell(ColumnData::SmallDateTime(Some(SmallDateTime::new(1, 61)))), Cell::Text("1900-01-02 01:01".into()));
        let utc = DateTime2::new(date_value, Time::new(23 * 3600, 0));
        assert_eq!(cell(ColumnData::DateTimeOffset(Some(DateTimeOffset::new(utc, 120)))), Cell::Text("2026-11-02 01:00:00 +02:00".into()));
    }
}
