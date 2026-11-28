use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use devtools_symbols::SymbolTable;
use impasto::{AppPaths, ImpastoError, PluginContext, PluginInitError, PluginRegistrar, Runtime};

mod artifact;
mod connection;
mod discovery;

use artifact::ArtifactSummary;
use connection::{Connection, Snapshot};
use discovery::TargetSummary;

#[derive(Debug)]
pub struct DesktopError(String);

impl DesktopError {
    fn new(error: impl ToString) -> Self {
        Self(error.to_string())
    }
}

impl fmt::Display for DesktopError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str(&self.0)
    }
}

impl std::error::Error for DesktopError {}

impl ImpastoError for DesktopError {
    fn scope(&self) -> &'static str {
        "impasto:devtools-desktop"
    }
    fn code(&self) -> &'static str {
        "unavailable"
    }
    fn message(&self) -> String {
        self.0.clone()
    }
}

impl From<std::io::Error> for DesktopError {
    fn from(error: std::io::Error) -> Self {
        Self::new(error)
    }
}

impl From<impasto_file_system::FileError> for DesktopError {
    fn from(error: impasto_file_system::FileError) -> Self {
        Self::new(error)
    }
}

#[derive(Default)]
struct DesktopState {
    connection: Option<Connection>,
    offline_symbols: Option<SymbolTable>,
}

#[impasto::module(specifier = "@app/devtools-core", init = from_context)]
pub struct Desktop {
    runtime: Runtime,
    paths: AppPaths,
    state: Arc<Mutex<DesktopState>>,
}

impl Desktop {
    fn from_context(ctx: &PluginContext) -> Result<Self, PluginInitError> {
        Ok(Self {
            runtime: ctx.runtime().clone(),
            paths: ctx.paths().clone(),
            state: Arc::default(),
        })
    }

    async fn io<T: Send + 'static>(
        &self,
        work: impl FnOnce() -> Result<T, DesktopError> + Send + 'static,
    ) -> Result<T, DesktopError> {
        self.runtime
            .spawn_blocking(work)
            .await
            .map_err(DesktopError::new)?
    }
}

#[impasto::export]
impl Desktop {
    pub fn preferred_target() -> Option<String> {
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            if arg == "--attach" {
                return args.next();
            }
        }
        None
    }

    pub async fn discover(&self) -> Result<Vec<TargetSummary>, DesktopError> {
        self.io(|| {
            Ok(discovery::targets()
                .iter()
                .map(discovery::Target::summary)
                .collect())
        })
        .await
    }

    pub async fn attach(&self, target_id: String) -> Result<Snapshot, DesktopError> {
        let state = Arc::clone(&self.state);
        self.io(move || {
            let started = Instant::now();
            let target = loop {
                if let Some(target) = discovery::targets()
                    .into_iter()
                    .find(|target| target.summary().id == target_id)
                {
                    break target;
                }
                if started.elapsed() > Duration::from_secs(3) {
                    return Err(DesktopError::new("target session is unavailable"));
                }
                std::thread::sleep(Duration::from_millis(100));
            };
            let (connection, snapshot) = Connection::open(&target).map_err(DesktopError::new)?;
            state
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .connection = Some(connection);
            Ok(snapshot)
        })
        .await
    }

    pub async fn snapshot(&self) -> Result<Option<Snapshot>, DesktopError> {
        let state = Arc::clone(&self.state);
        self.io(move || {
            let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
            let result = state
                .connection
                .as_mut()
                .map(Connection::snapshot)
                .transpose();
            if result.is_err() {
                state.connection = None;
            }
            result.map_err(DesktopError::new)
        })
        .await
    }

    pub async fn disconnect(&self) -> Result<(), DesktopError> {
        let state = Arc::clone(&self.state);
        self.io(move || {
            let connection = state
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .connection
                .take();
            drop(connection);
            Ok(())
        })
        .await
    }

    pub async fn import_artifact(
        &self,
        relative_path: String,
    ) -> Result<ArtifactSummary, DesktopError> {
        let paths = self.paths.clone();
        let state = Arc::clone(&self.state);
        self.io(move || {
            impasto_file_system::consume_import(&paths, &relative_path, |path, file| {
                let summary = artifact::read(path, file)?;
                if summary.kind == "JavaScript source map" {
                    let symbols = SymbolTable::load(path).map_err(DesktopError::new)?;
                    state
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .offline_symbols = Some(symbols);
                }
                Ok(summary)
            })
        })
        .await
    }

    pub async fn resolve_stack(&self, stack: String) -> Result<String, DesktopError> {
        let state = Arc::clone(&self.state);
        self.io(move || {
            let state = state.lock().unwrap_or_else(|error| error.into_inner());
            let symbols = state
                .connection
                .as_ref()
                .and_then(|connection| connection.symbols.as_ref())
                .or(state.offline_symbols.as_ref());
            Ok(symbols.map_or_else(|| stack.clone(), |table| table.resolver().remap(&stack)))
        })
        .await
    }
}

pub fn register(registrar: &mut PluginRegistrar) {
    registrar.add(Desktop::plugin());
}
