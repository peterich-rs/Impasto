//! Scoped SQLite databases with dedicated connection workers.

mod wire;
mod worker;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex, OnceLock, Weak,
    atomic::{AtomicU32, Ordering},
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use impasto::{ImpastoError, ImpastoType, PluginContext, PluginInitError, PluginRegistrar};
use impasto_file_system::{Scope, resolve_path};
use rusqlite::{
    Connection, ErrorCode, OpenFlags, TransactionBehavior,
    hooks::{AuthAction, AuthContext, Authorization},
};

use wire::{Cell, Column, IntMode, Params};
use worker::Worker;

#[derive(Debug)]
struct SqlError(&'static str, &'static str);
impl SqlError {
    fn io() -> Self {
        Self("io", "Could not access the database")
    }
    fn busy() -> Self {
        Self("busy", "Database writer or request queue is busy")
    }
    fn closed() -> Self {
        Self("closed", "Database or transaction is closed")
    }
    fn mismatch() -> Self {
        Self("mismatch", "Invalid database value or options")
    }
    fn scope() -> Self {
        Self("outside-scope", "Database path is outside its scope")
    }
    fn migration() -> Self {
        Self(
            "migration-checksum",
            "Recorded migration differs from the supplied history",
        )
    }
}
impl ImpastoError for SqlError {
    fn scope(&self) -> &'static str {
        "sqlite"
    }
    fn code(&self) -> &'static str {
        self.0
    }
    fn message(&self) -> String {
        self.1.to_owned()
    }
}
impl From<rusqlite::Error> for SqlError {
    fn from(error: rusqlite::Error) -> Self {
        match error {
            rusqlite::Error::SqliteFailure(error, _) => match error.code {
                ErrorCode::DatabaseBusy => Self("busy", "Database is busy"),
                ErrorCode::DatabaseLocked => Self("locked", "Database is locked"),
                ErrorCode::ConstraintViolation => Self("constraint", "SQL constraint failed"),
                ErrorCode::ReadOnly => Self("readonly", "Database is read-only"),
                ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase => {
                    Self("corrupt", "Invalid database or encryption key")
                }
                ErrorCode::TypeMismatch | ErrorCode::ParameterOutOfRange => Self::mismatch(),
                ErrorCode::CannotOpen | ErrorCode::NotFound => {
                    Self("not-found", "Database could not be opened")
                }
                ErrorCode::Unknown | ErrorCode::AuthorizationForStatementDenied => {
                    Self("syntax", "SQL statement could not be prepared")
                }
                _ => Self::io(),
            },
            rusqlite::Error::InvalidQuery | rusqlite::Error::ExecuteReturnedResults => {
                Self("syntax", "Invalid SQL statement for this operation")
            }
            _ => Self::mismatch(),
        }
    }
}

#[derive(ImpastoType)]
struct OpenOptions {
    name: String,
    scope: Option<Scope>,
    read_only: Option<bool>,
    wal: Option<bool>,
    foreign_keys: Option<bool>,
    busy_timeout_ms: Option<u32>,
    read_pool_size: Option<u32>,
    int64_mode: Option<IntMode>,
    encryption_key: Option<Cell>,
}

#[derive(ImpastoType)]
struct ExecuteResult {
    changes: f64,
    last_insert_rowid: Column,
}

#[derive(ImpastoType)]
struct SqlStatement {
    sql: String,
    params: Option<Params>,
}

#[derive(ImpastoType)]
struct Migration {
    version: u32,
    name: String,
    up: Vec<String>,
}

#[derive(ImpastoType)]
struct Applied {
    version: u32,
    name: String,
}

#[derive(ImpastoType)]
struct MigrationReport {
    from_version: u32,
    to_version: u32,
    applied: Vec<Applied>,
}

type Rows = Vec<Vec<(String, Column)>>;

fn authorize(context: AuthContext<'_>) -> Authorization {
    match context.action {
        AuthAction::Attach { .. }
        | AuthAction::Detach { .. }
        | AuthAction::Transaction { .. }
        | AuthAction::Savepoint { .. } => Authorization::Deny,
        AuthAction::Pragma { pragma_name, .. }
            if [
                "key",
                "rekey",
                "writable_schema",
                "temp_store_directory",
                "data_store_directory",
            ]
            .iter()
            .any(|name| pragma_name.eq_ignore_ascii_case(name)) =>
        {
            Authorization::Deny
        }
        _ => Authorization::Allow,
    }
}

fn user_sql<T>(
    conn: &Connection,
    f: impl FnOnce(&Connection) -> Result<T, SqlError>,
) -> Result<T, SqlError> {
    conn.authorizer(Some(authorize));
    let result = f(conn);
    conn.authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    result
}

fn execute(
    conn: &Connection,
    statement: SqlStatement,
    mode: IntMode,
) -> Result<ExecuteResult, SqlError> {
    user_sql(conn, |conn| {
        let mut stmt = conn.prepare(&statement.sql)?;
        statement.params.unwrap_or_default().bind(&mut stmt)?;
        let changes = stmt.raw_execute()?;
        Ok(ExecuteResult {
            changes: f64::from(u32::try_from(changes).map_err(|_| SqlError::mismatch())?),
            last_insert_rowid: Column::rowid(conn.last_insert_rowid(), mode),
        })
    })
}

fn query(
    conn: &Connection,
    sql: &str,
    params: Params,
    mode: IntMode,
    reader: bool,
) -> Result<Option<Rows>, SqlError> {
    user_sql(conn, |conn| {
        let mut stmt = conn.prepare(sql)?;
        if reader && !stmt.readonly() {
            return Ok(None);
        }
        let names = stmt
            .column_names()
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        params.bind(&mut stmt)?;
        let mut iter = stmt.raw_query();
        let mut result = Vec::new();
        while let Some(row) = iter.next()? {
            let cells = names
                .iter()
                .enumerate()
                .map(|(index, name)| {
                    Ok((name.clone(), Column::from_sql(row.get_ref(index)?, mode)?))
                })
                .collect::<Result<_, SqlError>>()?;
            result.push(cells);
        }
        Ok(Some(result))
    })
}

fn path(ctx: &PluginContext, name: &str, scope: Option<Scope>) -> Result<PathBuf, SqlError> {
    if name.is_empty() || name == ":memory:" {
        return Err(SqlError::mismatch());
    }
    resolve_path(
        ctx,
        scope.unwrap_or(Scope::ApplicationSupport),
        &format!("databases/{name}"),
    )
    .map_err(|_| SqlError::scope())
}

fn configure(conn: &Connection, options: &OpenOptions) -> Result<(), SqlError> {
    if let Some(key) = &options.encryption_key {
        configure_key(conn, key)?;
        conn.query_row("SELECT count(*) FROM sqlite_master", [], |_| Ok(()))?;
    }
    conn.busy_timeout(Duration::from_millis(u64::from(
        options.busy_timeout_ms.unwrap_or(5_000),
    )))?;
    conn.pragma_update(None, "foreign_keys", options.foreign_keys.unwrap_or(true))?;
    conn.pragma_update(None, "trusted_schema", false)?;
    Ok(())
}

#[cfg(feature = "sqlcipher")]
fn configure_key(conn: &Connection, key: &Cell) -> Result<(), SqlError> {
    use rusqlite::types::Value;
    use zeroize::Zeroizing;

    match &key.0 {
        Value::Text(passphrase) if !passphrase.is_empty() => {
            conn.pragma_update(None, "key", passphrase)?
        }
        Value::Blob(bytes) if bytes.len() == 32 => {
            let key = Zeroizing::new(format!(
                "x'{}'",
                Zeroizing::new(hex::encode(bytes)).as_str()
            ));
            conn.pragma_update(None, "key", key.as_str())?;
        }
        _ => return Err(SqlError::mismatch()),
    }
    Ok(())
}

#[cfg(not(feature = "sqlcipher"))]
fn configure_key(_: &Connection, _: &Cell) -> Result<(), SqlError> {
    Err(SqlError(
        "encryption-not-enabled",
        "Enable the sqlcipher feature to open an encrypted database",
    ))
}

type OpenDatabases = Mutex<BTreeMap<PathBuf, Weak<()>>>;
static OPEN_DATABASES: OnceLock<OpenDatabases> = OnceLock::new();

fn canonical_destination(file: &Path) -> Result<PathBuf, SqlError> {
    for ancestor in file.ancestors() {
        match ancestor.canonicalize() {
            Ok(parent) => {
                let suffix = file.strip_prefix(ancestor).map_err(|_| SqlError::scope())?;
                return Ok(if suffix.as_os_str().is_empty() {
                    parent
                } else {
                    parent.join(suffix)
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(SqlError::io()),
        }
    }
    Err(SqlError::scope())
}

fn sidecars(file: &Path) -> Result<[PathBuf; 3], SqlError> {
    let result = ["-wal", "-shm", "-journal"].map(|suffix| {
        let mut name = file.as_os_str().to_owned();
        name.push(suffix);
        PathBuf::from(name)
    });
    for sidecar in &result {
        match sidecar.symlink_metadata() {
            Ok(metadata) if !metadata.is_file() => return Err(SqlError::scope()),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(SqlError::io()),
        }
    }
    Ok(result)
}

fn reserve_path(file: &Path) -> Result<Arc<()>, SqlError> {
    let mut open = OPEN_DATABASES
        .get_or_init(Mutex::default)
        .lock()
        .map_err(|_| SqlError::closed())?;
    if let Some(live) = open.get(file).and_then(Weak::upgrade) {
        return Ok(live);
    }
    open.retain(|_, lease| lease.strong_count() > 0);
    let lease = Arc::new(());
    open.insert(file.to_owned(), Arc::downgrade(&lease));
    Ok(lease)
}

fn open(ctx: &PluginContext, options: OpenOptions) -> Result<Database, SqlError> {
    if !cfg!(feature = "sqlcipher") && options.encryption_key.is_some() {
        return Err(SqlError(
            "encryption-not-enabled",
            "Enable the sqlcipher feature to open an encrypted database",
        ));
    }
    let mode = options.int64_mode.unwrap_or_default();
    if options.name == ":memory:" {
        if options.read_only == Some(true) {
            return Err(SqlError::mismatch());
        }
        let writer = Connection::open_in_memory()?;
        configure(&writer, &options)?;
        return Ok(Database {
            worker: Worker::new(writer, Vec::new(), None)?,
            mode,
        });
    }
    let path = canonical_destination(&path(ctx, &options.name, options.scope)?)?;
    let lease = reserve_path(&path)?;
    sidecars(&path)?;
    let writable = !options.read_only.unwrap_or(false);
    if writable {
        std::fs::create_dir_all(path.parent().ok_or_else(SqlError::scope)?)
            .map_err(|_| SqlError::io())?;
    }
    let flags = if writable {
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE
    } else {
        OpenFlags::SQLITE_OPEN_READ_ONLY
    };
    let writer = Connection::open_with_flags(&path, flags | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
    configure(&writer, &options)?;
    let wal = if options.wal.unwrap_or(true) {
        if writable {
            writer.query_row("PRAGMA journal_mode=WAL", [], |row| row.get::<_, String>(0))?
        } else {
            writer.pragma_query_value(None, "journal_mode", |row| row.get::<_, String>(0))?
        }
    } else {
        String::new()
    };
    let readers = if wal.eq_ignore_ascii_case("wal") {
        if writable {
            writer.pragma_update(None, "synchronous", "NORMAL")?;
        }
        let count = options.read_pool_size.unwrap_or(2);
        if count > 16 {
            return Err(SqlError::mismatch());
        }
        (0..count)
            .map(|_| {
                let conn = Connection::open_with_flags(
                    &path,
                    OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
                )?;
                configure(&conn, &options)?;
                Ok(conn)
            })
            .collect::<Result<_, SqlError>>()?
    } else {
        Vec::new()
    };
    Ok(Database {
        worker: Worker::new(writer, readers, Some(lease))?,
        mode,
    })
}

#[impasto::object]
struct Database {
    worker: Arc<Worker>,
    mode: IntMode,
}

#[impasto::export]
impl Database {
    #[impasto::method(blocking)]
    pub fn execute(&self, sql: String, params: Option<Params>) -> Result<ExecuteResult, SqlError> {
        let mode = self.mode;
        self.worker.write(None, move |conn| {
            execute(conn, SqlStatement { sql, params }, mode)
        })
    }
    #[impasto::method(blocking)]
    pub fn query(&self, sql: String, params: Option<Params>) -> Result<Rows, SqlError> {
        let mode = self.mode;
        let read_sql = sql.clone();
        let read_params = params.clone();
        let result = self.worker.read(move |conn| {
            query(conn, &read_sql, read_params.unwrap_or_default(), mode, true)
        })?;
        if let Some(result) = result {
            return Ok(result);
        }
        self.worker.write(None, move |conn| {
            query(conn, &sql, params.unwrap_or_default(), mode, false)
                .map(Option::unwrap_or_default)
        })
    }
    #[impasto::method(blocking)]
    pub fn batch(&self, statements: Vec<SqlStatement>) -> Result<Vec<ExecuteResult>, SqlError> {
        let mode = self.mode;
        self.worker.write(None, move |conn| {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let result = statements
                .into_iter()
                .map(|statement| execute(&tx, statement, mode))
                .collect::<Result<_, _>>()?;
            tx.commit()?;
            Ok(result)
        })
    }
    #[impasto::method(resource)]
    pub async fn begin_transaction(&self) -> Result<SqlTransaction, SqlError> {
        // A lease waits on its own worker, independent of the JavaScript queue.
        let worker = Arc::clone(&self.worker);
        let mode = self.mode;
        tokio::task::spawn_blocking(move || {
            Ok(SqlTransaction {
                lease: worker.begin()?,
                worker,
                mode,
                sequence: AtomicU32::new(0),
            })
        })
        .await
        .map_err(|_| SqlError::closed())?
    }
    #[impasto::method(blocking)]
    pub fn migrate(&self, migrations: Vec<Migration>) -> Result<MigrationReport, SqlError> {
        self.worker
            .write(None, move |conn| migrate(conn, migrations))
    }
    #[impasto::method(blocking)]
    pub fn checkpoint(&self, mode: Option<String>) -> Result<(), SqlError> {
        let mode = match mode.as_deref().unwrap_or("passive") {
            "passive" => "PRAGMA wal_checkpoint(PASSIVE)",
            "full" => "PRAGMA wal_checkpoint(FULL)",
            "truncate" => "PRAGMA wal_checkpoint(TRUNCATE)",
            "restart" => "PRAGMA wal_checkpoint(RESTART)",
            _ => return Err(SqlError::mismatch()),
        };
        self.worker.write(None, move |conn| {
            let blocked = conn.query_row(mode, [], |row| row.get::<_, u32>(0))?;
            if blocked != 0 {
                return Err(SqlError("busy", "Checkpoint could not finish"));
            }
            Ok(())
        })
    }
    #[impasto::method(blocking)]
    pub fn dispose(&self) -> Result<(), SqlError> {
        self.worker.close()
    }
}

#[impasto::object]
struct SqlTransaction {
    worker: Arc<Worker>,
    lease: u64,
    mode: IntMode,
    sequence: AtomicU32,
}

impl Drop for SqlTransaction {
    fn drop(&mut self) {
        self.worker.drop_lease(self.lease);
    }
}

#[impasto::export]
impl SqlTransaction {
    #[impasto::method(blocking)]
    pub fn execute(&self, sql: String, params: Option<Params>) -> Result<ExecuteResult, SqlError> {
        let mode = self.mode;
        self.worker.write(Some(self.lease), move |conn| {
            execute(conn, SqlStatement { sql, params }, mode)
        })
    }
    #[impasto::method(blocking)]
    pub fn query(&self, sql: String, params: Option<Params>) -> Result<Rows, SqlError> {
        let mode = self.mode;
        self.worker.write(Some(self.lease), move |conn| {
            query(conn, &sql, params.unwrap_or_default(), mode, false)
                .map(Option::unwrap_or_default)
        })
    }
    #[impasto::method(blocking)]
    pub fn savepoint(&self) -> Result<u32, SqlError> {
        let id = self.sequence.fetch_add(1, Ordering::Relaxed);
        self.worker.write(Some(self.lease), move |conn| {
            conn.execute_batch(&format!("SAVEPOINT impasto_{id}"))?;
            Ok(id)
        })
    }
    #[impasto::method(blocking)]
    pub fn release_savepoint(&self, id: u32, rollback: bool) -> Result<(), SqlError> {
        self.worker.write(Some(self.lease), move |conn| {
            if rollback {
                conn.execute_batch(&format!("ROLLBACK TO impasto_{id}"))?;
            }
            Ok(conn.execute_batch(&format!("RELEASE impasto_{id}"))?)
        })
    }
    #[impasto::method(blocking)]
    pub fn finish(&self, commit: bool) -> Result<(), SqlError> {
        self.worker.end(self.lease, commit)
    }
}

fn migrate(conn: &mut Connection, migrations: Vec<Migration>) -> Result<MigrationReport, SqlError> {
    if migrations
        .iter()
        .any(|m| m.version == 0 || m.name.trim().is_empty() || m.up.is_empty())
        || migrations
            .windows(2)
            .any(|items| items[0].version >= items[1].version)
    {
        return Err(SqlError::migration());
    }
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute_batch("CREATE TABLE IF NOT EXISTS _impasto_migrations(version INTEGER PRIMARY KEY, name TEXT NOT NULL, checksum TEXT NOT NULL, applied_at_ms INTEGER NOT NULL)")?;
    let history = {
        let mut stmt =
            tx.prepare("SELECT version, name, checksum FROM _impasto_migrations ORDER BY version")?;
        stmt.query_map([], |row| {
            Ok((
                row.get::<_, u32>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?
    };
    if history.len() > migrations.len() {
        return Err(SqlError::migration());
    }
    let from_version = history.last().map_or(0, |entry| entry.0);
    let mut applied = Vec::new();
    for (index, migration) in migrations.into_iter().enumerate() {
        let sql = migration
            .up
            .iter()
            .map(|sql| sql.replace("\r\n", "\n"))
            .collect::<Vec<_>>();
        let mut normalized = Vec::new();
        for sql in &sql {
            let sql = sql.trim();
            normalized.extend_from_slice(&(sql.len() as u64).to_le_bytes());
            normalized.extend_from_slice(sql.as_bytes());
        }
        let checksum = hex::encode(impasto_crypto::sha256(&normalized));
        let version = migration.version;
        let name = migration.name;
        if let Some(stored) = history.get(index) {
            if *stored != (version, name, checksum) {
                return Err(SqlError::migration());
            }
            continue;
        }
        for statement in sql {
            user_sql(&tx, |conn| Ok(conn.execute_batch(&statement)?))?;
        }
        let applied_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|value| i64::try_from(value.as_millis()).ok())
            .unwrap_or_default();
        tx.execute(
            "INSERT INTO _impasto_migrations VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![version, name, checksum, applied_at],
        )?;
        tx.pragma_update(None, "user_version", version)?;
        applied.push(Applied { version, name });
    }
    let to_version = applied.last().map_or(from_version, |entry| entry.version);
    tx.commit()?;
    Ok(MigrationReport {
        from_version,
        to_version,
        applied,
    })
}

#[impasto::module(name = "Sqlite", specifier = "@impasto/sqlite/native", init = init)]
struct Sqlite(PluginContext);
impl Sqlite {
    fn init(ctx: &PluginContext) -> Result<Self, PluginInitError> {
        Ok(Self(ctx.clone()))
    }
}
#[impasto::export]
impl Sqlite {
    pub fn is_sqlite_host_available(&self) -> bool {
        self.0.is_hosted()
    }
    #[impasto::method(resource)]
    pub async fn open_database(&self, options: OpenOptions) -> Result<Database, SqlError> {
        let ctx = self.0.clone();
        tokio::task::spawn_blocking(move || open(&ctx, options))
            .await
            .map_err(|_| SqlError::closed())?
    }
    #[impasto::method(blocking)]
    pub fn delete_database(&self, name: String, scope: Option<Scope>) -> Result<(), SqlError> {
        let file = canonical_destination(&path(&self.0, &name, scope)?)?;
        let open = OPEN_DATABASES
            .get_or_init(Mutex::default)
            .lock()
            .map_err(|_| SqlError::closed())?;
        if open
            .get(&file)
            .is_some_and(|lease| lease.strong_count() > 0)
        {
            return Err(SqlError::busy());
        }
        let sidecars = sidecars(&file)?;
        remove(&file)?;
        for sidecar in sidecars {
            remove(&sidecar)?;
        }
        drop(open);
        Ok(())
    }
}
fn remove(path: &Path) -> Result<(), SqlError> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(SqlError::io()),
    }
}

/// Registers the native SQLite bindings.
pub fn register(registrar: &mut PluginRegistrar) {
    registrar.add(Sqlite::plugin());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verifies_applied_migrations_and_rolls_back_failed_steps() -> Result<(), SqlError> {
        let mut connection = Connection::open_in_memory()?;
        let table = || Migration {
            version: 1,
            name: "notes".into(),
            up: vec!["CREATE TABLE notes (name TEXT NOT NULL)".into()],
        };
        assert_eq!(migrate(&mut connection, vec![table()])?.to_version, 1);
        assert!(migrate(&mut connection, vec![table()])?.applied.is_empty());
        let failed = Migration {
            version: 2,
            name: "atomic".into(),
            up: vec![
                "INSERT INTO notes VALUES ('temporary')".into(),
                "SELECT FROM".into(),
            ],
        };
        assert!(migrate(&mut connection, vec![table(), failed]).is_err());
        let count =
            connection.query_row("SELECT count(*) FROM notes", [], |row| row.get::<_, u32>(0))?;
        assert_eq!(count, 0);
        let changed = Migration {
            up: vec!["SELECT 1".into()],
            ..table()
        };
        let code = migrate(&mut connection, vec![changed])
            .err()
            .map(|error| error.code());
        assert_eq!(code, Some("migration-checksum"));
        Ok(())
    }

    #[test]
    fn wal_readers_see_committed_state_while_the_writer_owns_a_transaction() -> Result<(), SqlError>
    {
        let file = tempfile::NamedTempFile::new().map_err(|_| SqlError::io())?;
        let writer = Connection::open(file.path())?;
        writer
            .execute_batch("PRAGMA journal_mode = WAL; CREATE TABLE notes (name TEXT NOT NULL)")?;
        let read = Connection::open_with_flags(file.path(), OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let pool = Worker::new(writer, vec![read], None)?;
        let lease = pool.begin()?;
        pool.write(Some(lease), |connection| {
            connection.execute("INSERT INTO notes VALUES ('temporary')", [])?;
            Ok(())
        })?;
        let count = |connection: &mut Connection| {
            Ok(connection
                .query_row("SELECT count(*) FROM notes", [], |row| row.get::<_, u32>(0))?)
        };
        assert_eq!(pool.read(count)?, 0);
        pool.drop_lease(lease);
        assert_eq!(pool.write(None, count)?, 0);
        pool.close()?;
        assert!(pool.read(count).is_err());
        Ok(())
    }
}
