//! First-party host modules. Constructors return [`session::HostModule`].

pub mod app_state;
pub mod clipboard;
pub mod fs;
pub mod linking;
pub mod memory;
pub mod permissions;

pub use app_state::module as app_state;
pub use clipboard::{ClipboardHost, module as clipboard};
pub use fs::{StorageHost, async_storage, module as file_system};
pub use linking::{LinkingHost, module as linking};
pub use permissions::{PermissionsHost, module as permissions};

use session::PluginValue;

pub(crate) fn str_arg(args: &[PluginValue]) -> String {
    match args.first() {
        Some(PluginValue::Str(value)) => value.clone(),
        _ => String::new(),
    }
}

pub(crate) fn string_list(value: Option<&PluginValue>) -> Vec<String> {
    let Some(PluginValue::List(items)) = value else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| match item {
            PluginValue::Str(text) => Some(text.clone()),
            _ => None,
        })
        .collect()
}

pub(crate) fn string_pairs(value: Option<&PluginValue>) -> Vec<(String, String)> {
    let Some(PluginValue::List(rows)) = value else {
        return Vec::new();
    };
    rows.iter()
        .filter_map(|row| {
            let PluginValue::List(pair) = row else {
                return None;
            };
            match (pair.first(), pair.get(1)) {
                (Some(PluginValue::Str(key)), Some(PluginValue::Str(value))) => {
                    Some((key.clone(), value.clone()))
                }
                _ => None,
            }
        })
        .collect()
}

pub(crate) fn done_null() -> session::PluginResult {
    session::PluginResult::Done(PluginValue::Null)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use session::{
        HostMethod, HostModule, HttpClient, Invoke, MethodExec, ModuleRegistry, PluginResult,
        PluginValue, Settle, Work,
    };

    use super::*;
    use crate::modules::memory::{DeniedPermissions, MemoryClipboard, MemoryStorage, NullLinking};

    struct RecordingExec {
        worker: AtomicUsize,
        now: AtomicUsize,
    }

    impl MethodExec for RecordingExec {
        fn worker(&self, settle: Settle, work: Work) {
            self.worker.fetch_add(1, Ordering::SeqCst);
            settle.send(work());
        }

        fn main_thread(&self, settle: Settle, work: Work) {
            settle.send(work());
        }

        fn now(&self, settle: Settle, result: PluginResult) {
            self.now.fetch_add(1, Ordering::SeqCst);
            settle.send(result);
        }
    }

    fn run_promise(invoke: &Invoke, args: &[PluginValue], exec: &dyn MethodExec) -> PluginResult {
        let Invoke::Promise(f) = invoke else {
            panic!("expected promise");
        };
        let (tx, rx) = std::sync::mpsc::channel();
        f(
            args,
            Settle::new(move |result| {
                let _ = tx.send(result);
            }),
            exec,
        );
        match rx.recv() {
            Ok(result) => result,
            Err(_) => panic!("settle was not sent"),
        }
    }

    #[test]
    fn host_module_table_includes_unknown_names() {
        let clip = MemoryClipboard::new("");
        let storage = MemoryStorage::new("/tmp/file-system");
        let launch = session::shared(None);
        let maps = HostModule {
            name: "Maps",
            specifier: "@impasto/maps/host",
            methods: vec![HostMethod::sync("ping", |_| {
                Ok(PluginValue::Str("pong".into()))
            })],
        };
        let registry = ModuleRegistry::new(vec![
            clipboard(clip),
            linking(Arc::new(NullLinking), launch),
            file_system(Arc::clone(&storage) as _),
            async_storage(storage),
            permissions(Arc::new(DeniedPermissions)),
            maps,
        ]);
        match registry.call("Maps", "ping", &[]) {
            Ok(PluginValue::Str(text)) => assert_eq!(text, "pong"),
            other => panic!("{other:?}"),
        }
        match registry.call("Clipboard", "getString", &[]) {
            Ok(PluginValue::Str(text)) => assert!(text.is_empty()),
            other => panic!("{other:?}"),
        }
        match registry.call("Clipboard", "setString", &[PluginValue::Str("hi".into())]) {
            Ok(PluginValue::Null) => {}
            other => panic!("{other:?}"),
        }
        match registry.method("FileSystem", "documentDirectory") {
            Some(method) => match &method.invoke {
                Invoke::Sync(f) => match f(&[]) {
                    Ok(PluginValue::Str(path)) => assert_eq!(path, "/tmp/file-system"),
                    other => panic!("{other:?}"),
                },
                Invoke::Promise(_) => panic!("documentDirectory must be sync"),
            },
            None => panic!("documentDirectory"),
        }
    }

    #[test]
    fn filesystem_rejects_escape_on_the_js_thread() {
        let disk = Arc::new(AtomicBool::new(false));
        struct Probe(Arc<AtomicBool>);
        impl StorageHost for Probe {
            fn document_dir(&self) -> Result<String, String> {
                Ok("/tmp/file-system".into())
            }
            fn pick_document(&self) -> Result<Option<String>, String> {
                self.0.store(true, Ordering::SeqCst);
                Ok(None)
            }
        }
        let module = file_system(Arc::new(Probe(Arc::clone(&disk))));
        let read = match module.methods.iter().find(|method| method.name == "read") {
            Some(method) => method,
            None => panic!("read"),
        };
        let exec = RecordingExec {
            worker: AtomicUsize::new(0),
            now: AtomicUsize::new(0),
        };
        let result = run_promise(
            &read.invoke,
            &[PluginValue::Str("../etc/passwd".into())],
            &exec,
        );
        match result {
            PluginResult::Failed { message } => {
                assert!(message.contains("outside"), "{message}");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(exec.worker.load(Ordering::SeqCst), 0);
        assert_eq!(exec.now.load(Ordering::SeqCst), 1);
        assert!(!disk.load(Ordering::SeqCst));
    }

    #[test]
    fn worker_does_not_block_http() {
        use std::io::{Read, Write};
        use std::time::{Duration, Instant};

        struct SlowExec {
            handle: tokio::runtime::Handle,
        }
        impl MethodExec for SlowExec {
            fn worker(&self, settle: Settle, work: Work) {
                drop(self.handle.spawn_blocking(move || settle.send(work())));
            }
            fn main_thread(&self, settle: Settle, work: Work) {
                settle.send(work());
            }
            fn now(&self, settle: Settle, result: PluginResult) {
                settle.send(result);
            }
        }

        let handle = match crate::rt::handle() {
            Ok(handle) => handle,
            Err(error) => panic!("{error}"),
        };
        let exec = SlowExec { handle };
        let (tx, rx) = std::sync::mpsc::channel();
        exec.worker(
            Settle::new(move |result| {
                let _ = tx.send(result);
            }),
            Box::new(|| {
                std::thread::sleep(Duration::from_millis(150));
                PluginResult::Done(PluginValue::Str("slow".into()))
            }),
        );

        let listener = match std::net::TcpListener::bind("127.0.0.1:0") {
            Ok(listener) => listener,
            Err(error) => panic!("{error}"),
        };
        let port = match listener.local_addr() {
            Ok(addr) => addr.port(),
            Err(error) => panic!("{error}"),
        };
        let server = std::thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            let body = b"ok";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.write_all(body);
        });
        let client = match crate::http::OpenWireClient::new() {
            Ok(client) => client,
            Err(error) => panic!("{error}"),
        };
        let started = Instant::now();
        let result = client.perform(&session::HttpRequest {
            token: 1,
            method: "GET".to_owned(),
            url: format!("http://127.0.0.1:{port}/"),
            headers: Vec::new(),
            body: Vec::new(),
            cancel: Arc::new(AtomicBool::new(false)),
            timeout_ms: None,
        });
        let http_ms = started.elapsed();
        let _ = server.join();
        match result {
            Ok(response) => assert_eq!(response.body, b"ok"),
            Err(error) => panic!("{}", error.message),
        }
        assert!(
            http_ms < Duration::from_millis(150),
            "http waited on worker: {http_ms:?}"
        );
        match rx.recv_timeout(Duration::from_secs(2)) {
            Ok(PluginResult::Done(PluginValue::Str(text))) => assert_eq!(text, "slow"),
            other => panic!("{other:?}"),
        }
    }
}
