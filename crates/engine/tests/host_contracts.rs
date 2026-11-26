//! Public host-contract baseline. These tests lock the shared session/module contracts:
//! one-shot settle, caller-thread execution, first-wins lookup, the `@impasto/` allow-list,
//! owned payloads, and shared slots.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;

use engine::session::{
    ImmediateExec, Invoke, IoEvent, Method, MethodExec, Module, ModuleConfig, ModuleRegistry,
    PluginResult, PluginValue, Settle, Work, document_relative, module_not_linked, shared,
    shared_read, shared_write,
};
use javascript::host::StreamEvent;

fn require_err<T: std::fmt::Debug>(result: Result<T, String>) -> String {
    match result {
        Ok(value) => panic!("expected error, got {value:?}"),
        Err(message) => message,
    }
}

fn sample_module(name: &'static str, specifier: &'static str, value: &'static str) -> Module {
    Module {
        name,
        specifier,
        exports: Default::default(),
        methods: vec![Method::sync("getString", move |_| {
            Ok(PluginValue::Str(value.into()))
        })],
    }
}

#[test]
fn settle_delivers_once_on_send_or_drop() {
    let (tx, rx) = mpsc::channel();
    drop(Settle::new(move |result| {
        let _ = tx.send(result);
    }));
    match rx.try_recv() {
        Ok(PluginResult::Failed { message }) => assert_eq!(message, "settle dropped"),
        _ => panic!("unexpected contract value"),
    }
    assert!(rx.try_recv().is_err());

    let (tx, rx) = mpsc::channel();
    Settle::new(move |result| {
        let _ = tx.send(result);
    })
    .send(PluginResult::Done(PluginValue::Null));
    match rx.try_recv() {
        Ok(PluginResult::Done(PluginValue::Null)) => {}
        _ => panic!("unexpected contract value"),
    }
    assert!(rx.try_recv().is_err());

    let (tx, rx) = mpsc::channel();
    let settle = Settle::new(move |result| {
        let _ = tx.send(("drop", result));
    });
    let worker = thread::spawn(move || drop(settle));
    if worker.join().is_err() {
        panic!("drop worker panicked");
    }
    match rx.try_recv() {
        Ok(("drop", PluginResult::Failed { message })) => assert_eq!(message, "settle dropped"),
        _ => panic!("unexpected contract value"),
    }

    let (tx, rx) = mpsc::channel();
    let settle = Settle::new(move |result| {
        let _ = tx.send(result);
    });
    let worker = thread::spawn(move || {
        settle.send(PluginResult::Done(PluginValue::Bool(true)));
    });
    if worker.join().is_err() {
        panic!("send worker panicked");
    }
    match rx.try_recv() {
        Ok(PluginResult::Done(PluginValue::Bool(true))) => {}
        _ => panic!("unexpected contract value"),
    }
    assert!(rx.try_recv().is_err());
}

#[test]
fn immediate_exec_runs_worker_main_and_now_on_the_caller() {
    let caller = thread::current().id();
    let exec = ImmediateExec;
    let (tx, rx) = mpsc::channel();
    let cases = [
        ("worker", PluginValue::Str("w".into())),
        ("main", PluginValue::Str("m".into())),
        ("now", PluginValue::Str("n".into())),
    ];
    exec.worker(
        Settle::new({
            let tx = tx.clone();
            move |result| {
                let _ = tx.send(("worker", result, thread::current().id()));
            }
        }),
        Box::new(|| PluginResult::Done(PluginValue::Str("w".into()))),
    );
    exec.main_thread(
        Settle::new({
            let tx = tx.clone();
            move |result| {
                let _ = tx.send(("main", result, thread::current().id()));
            }
        }),
        Box::new(|| PluginResult::Done(PluginValue::Str("m".into()))),
    );
    exec.now(
        Settle::new(move |result| {
            let _ = tx.send(("now", result, thread::current().id()));
        }),
        PluginResult::Done(PluginValue::Str("n".into())),
    );
    for (expected, value) in cases {
        match rx.try_recv() {
            Ok((name, PluginResult::Done(got), id)) => {
                assert_eq!(name, expected);
                assert_eq!(got, value);
                assert_eq!(id, caller);
            }
            _ => panic!("unexpected contract value"),
        }
    }
}

struct RecordingExec {
    calls: Arc<AtomicUsize>,
}

impl MethodExec for RecordingExec {
    fn worker(&self, settle: Settle, work: Work) {
        self.calls.fetch_add(1, Ordering::Relaxed);
        settle.send(work());
    }

    fn main_thread(&self, settle: Settle, work: Work) {
        self.calls.fetch_add(1, Ordering::Relaxed);
        settle.send(work());
    }

    fn now(&self, settle: Settle, result: PluginResult) {
        self.calls.fetch_add(1, Ordering::Relaxed);
        settle.send(result);
    }
}

#[test]
fn promise_invoke_receives_the_supplied_executor() {
    let calls = Arc::new(AtomicUsize::new(0));
    let method = Method::promise("search", |args, settle, exec| {
        assert_eq!(args, &[PluginValue::Str("q".into())]);
        exec.now(settle, PluginResult::Done(PluginValue::Str("found".into())));
    });
    let Invoke::Promise(function) = &method.invoke else {
        panic!("promise constructor stores a promise invoke");
    };
    let (tx, rx) = mpsc::channel();
    let exec = RecordingExec {
        calls: Arc::clone(&calls),
    };
    function(
        &[PluginValue::Str("q".into())],
        Settle::new(move |result| {
            let _ = tx.send(result);
        }),
        &exec,
    );
    match rx.try_recv() {
        Ok(PluginResult::Done(PluginValue::Str(text))) => assert_eq!(text, "found"),
        _ => panic!("unexpected contract value"),
    }
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}

#[test]
fn registry_first_wins_keeps_order_and_exact_errors() {
    let first = sample_module("Clipboard", "@impasto/clipboard", "one");
    let second = sample_module("Clipboard", "@impasto/other", "two");
    let duplicate_specifier = sample_module("Clipboard", "@impasto/clipboard", "three");
    let maps = Module {
        name: "Maps",
        exports: Default::default(),
        specifier: "@impasto/maps",
        methods: vec![
            Method::sync("ping", |_| Ok(PluginValue::Str("pong".into()))),
            Method::sync("ping", |_| Ok(PluginValue::Str("later".into()))),
            Method::promise("search", |_, settle, exec| {
                exec.now(settle, PluginResult::Done(PluginValue::Null));
            }),
        ],
    };
    let registry = ModuleRegistry::new(vec![first, second, duplicate_specifier, maps]);
    match registry.call("Clipboard", "getString", &[]) {
        Ok(PluginValue::Str(text)) => assert_eq!(text, "one"),
        _ => panic!("unexpected contract value"),
    }
    assert_eq!(
        registry
            .by_specifier("@impasto/clipboard")
            .map(|module| module.name),
        Some("Clipboard")
    );
    assert_eq!(
        registry
            .by_specifier("@impasto/other")
            .map(|module| module.specifier),
        Some("@impasto/other")
    );
    assert!(registry.by_specifier("@impasto/missing").is_none());
    assert_eq!(
        registry.module("Clipboard").map(|module| module.specifier),
        Some("@impasto/clipboard")
    );
    assert!(registry.module("Camera").is_none());
    match registry.method("Maps", "ping") {
        Some(method) => assert_eq!(method.name, "ping"),
        None => panic!("first ping method"),
    }
    match registry.call("Maps", "ping", &[]) {
        Ok(PluginValue::Str(text)) => assert_eq!(text, "pong"),
        _ => panic!("unexpected contract value"),
    }
    assert_eq!(
        require_err(registry.call("Maps", "search", &[])),
        "module \"Maps.search\" is not sync"
    );
    assert_eq!(
        require_err(registry.call("Camera", "take", &[])),
        "module \"Camera.take\" is not linked"
    );
    assert_eq!(
        require_err(registry.call("Maps", "missing", &[])),
        "module \"Maps.missing\" is not linked"
    );
    assert_eq!(
        module_not_linked("Camera", None),
        "module \"Camera\" is not linked"
    );
}

#[test]
fn restricted_matches_impasto_prefix_exactly() {
    let modules = vec![
        sample_module("Clipboard", "@impasto/clipboard", "clip"),
        sample_module("Extra", "@impasto/clipboard-extra", "extra"),
        sample_module("Nested", "@impasto/clipboard/extra", "nested"),
        sample_module("Clip", "@impasto/clip", "short"),
        sample_module("Case", "@impasto/Clipboard", "case"),
        sample_module("Bare", "impasto/clipboard", "bare"),
        sample_module("OtherPrefix", "@Impasto/clipboard", "other"),
        sample_module("Root", "@impasto/", "root"),
        sample_module("Maps", "@impasto/maps", "maps"),
        sample_module("Plain", "clipboard", "plain"),
    ];
    let config = ModuleConfig {
        registry: Arc::new(ModuleRegistry::new(modules)),
        ..ModuleConfig::disconnected()
    };
    let names = |packages: &[&str]| -> Vec<&str> {
        let owned: Vec<String> = packages
            .iter()
            .map(|package| (*package).to_owned())
            .collect();
        let allowed = config.clone().restrict(&owned).registry;
        [
            "Clipboard",
            "Extra",
            "Nested",
            "Clip",
            "Case",
            "Bare",
            "OtherPrefix",
            "Root",
            "Maps",
            "Plain",
        ]
        .into_iter()
        .filter(|name| allowed.module(name).is_some())
        .collect()
    };
    assert!(names(&[]).is_empty());
    assert_eq!(names(&["clipboard"]), vec!["Clipboard"]);
    assert_eq!(names(&["Clipboard"]), vec!["Case"]);
    assert_eq!(names(&["clipboard-extra"]), vec!["Extra"]);
    assert_eq!(names(&["clip"]), vec!["Clip"]);
    assert_eq!(names(&["clipboard/extra"]), vec!["Nested"]);
    assert_eq!(names(&[""]), vec!["Root"]);
    assert!(names(&["clipboard "]).is_empty());
    assert_eq!(
        names(&["maps", "clipboard", "clipboard"]),
        vec!["Clipboard", "Maps"]
    );
    assert_eq!(names(&["nope"]), Vec::<&str>::new());

    let narrowed = config
        .restrict(&["maps".into(), "clipboard".into()])
        .registry;
    match narrowed.call("Clipboard", "getString", &[]) {
        Ok(PluginValue::Str(text)) => assert_eq!(text, "clip"),
        _ => panic!("unexpected contract value"),
    }
    assert!(narrowed.module("Maps").is_some());
    assert!(narrowed.module("Extra").is_none());
    assert_eq!(
        require_err(narrowed.call("Extra", "getString", &[])),
        "module \"Extra.getString\" is not linked"
    );
}

#[test]
fn plugin_values_keep_bytes_order_and_duplicate_keys() {
    let value = PluginValue::Map(vec![
        ("n".into(), PluginValue::Null),
        ("b".into(), PluginValue::Bool(false)),
        ("f".into(), PluginValue::F64(-0.0)),
        ("s".into(), PluginValue::Str("text".into())),
        ("bytes".into(), PluginValue::Bytes(vec![0xff, 0x00, b'a'])),
        (
            "list".into(),
            PluginValue::List(vec![PluginValue::Str("a".into()), PluginValue::F64(1.0)]),
        ),
        (
            "k".into(),
            PluginValue::Map(vec![("k".into(), PluginValue::F64(1.0))]),
        ),
        ("k".into(), PluginValue::F64(2.0)),
    ]);
    let cloned = value.clone();
    assert_eq!(value, cloned);
    let PluginValue::Map(pairs) = value else {
        panic!("map");
    };
    assert_eq!(pairs.len(), 8);
    assert_eq!(pairs[4].0, "bytes");
    assert_eq!(pairs[4].1, PluginValue::Bytes(vec![0xff, 0x00, b'a']));
    assert_ne!(pairs[4].1, PluginValue::Str("\u{ff}\0a".into()));
    assert_eq!(pairs[6].0, "k");
    assert_eq!(pairs[7].0, "k");
    match &pairs[7].1 {
        PluginValue::F64(number) => assert_eq!(*number, 2.0),
        _ => panic!("unexpected contract value"),
    }
    match &pairs[2].1 {
        PluginValue::F64(number) => assert_eq!(*number, 0.0),
        _ => panic!("unexpected contract value"),
    }
}

#[test]
fn document_paths_stay_exact() {
    for path in ["a", "a/b.bin", "a.b", "a-b_c"] {
        match document_relative(path) {
            Ok(kept) => assert_eq!(kept, path),
            Err(error) => panic!("{path}: {error}"),
        }
    }
    for path in [
        "", "/", "\\a", "a\\b", "C:foo", "..", "a/../b", "a/./b", "/abs", "a//b", ".", "foo:bar",
    ] {
        match document_relative(path) {
            Err(message) => {
                assert_eq!(message, "path is outside the document directory");
            }
            Ok(ok) => panic!("{path} should fail, got {ok}"),
        }
    }
}

#[test]
fn io_variants_keep_their_payloads() {
    let events = [
        IoEvent::Plugin {
            token: 4,
            result: PluginResult::Done(PluginValue::Bytes(vec![1, 2])),
        },
        IoEvent::NativeStream {
            id: 7,
            event: StreamEvent::Item(PluginValue::Bytes(b"hi".to_vec())),
        },
        IoEvent::NativeStream {
            id: 7,
            event: StreamEvent::Failed("BadUrl".into()),
        },
    ];
    let (tx, rx) = mpsc::channel();
    if tx.send(events).is_err() {
        panic!("send events");
    }
    let events = match rx.try_recv() {
        Ok(events) => events,
        Err(error) => panic!("{error}"),
    };
    match &events[0] {
        IoEvent::Plugin {
            token: 4,
            result: PluginResult::Done(PluginValue::Bytes(bytes)),
        } => assert_eq!(bytes, &[1, 2]),
        _ => panic!("unexpected contract value"),
    }
    match &events[1] {
        IoEvent::NativeStream {
            id: 7,
            event: StreamEvent::Item(PluginValue::Bytes(bytes)),
        } => assert_eq!(bytes, b"hi"),
        _ => panic!("unexpected contract value"),
    }
    match &events[2] {
        IoEvent::NativeStream {
            id: 7,
            event: StreamEvent::Failed(reason),
        } => assert_eq!(reason, "BadUrl"),
        _ => panic!("unexpected contract value"),
    }
}

#[test]
fn module_config_literal_and_shared_slots_keep_their_defaults() {
    let registry = Arc::new(ModuleRegistry::new(vec![sample_module(
        "Clipboard",
        "@impasto/clipboard",
        "clip",
    )]));
    let config = ModuleConfig {
        registry: Arc::clone(&registry),
        exec: Arc::new(ImmediateExec),
        post_delivery: None,
        transport_clock: None,
        script_execution: Default::default(),
        settle: Arc::new(|_token| Settle::new(|_result| {})),
    };
    match config.registry.call("Clipboard", "getString", &[]) {
        Ok(PluginValue::Str(text)) => assert_eq!(text, "clip"),
        _ => panic!("unexpected contract value"),
    }
    let narrowed = config.restrict(&[]);
    assert!(narrowed.registry.module("Clipboard").is_none());

    let disconnected = ModuleConfig::disconnected();
    (disconnected.settle)(1).send(PluginResult::Failed {
        message: "ignored".into(),
    });
    drop((disconnected.settle)(2));

    let slot = shared(String::from("kept"));
    let probe = Arc::clone(&slot);
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = probe
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        panic!("hold the guard");
    }));
    assert!(panicked.is_err());
    assert_eq!(shared_read(&slot), "kept");
    shared_write(&slot, String::from("next"));
    assert_eq!(shared_read(&slot), "next");
}

fn assert_send<T: Send>() {}
fn assert_sync<T: Sync>() {}

#[test]
#[allow(
    unused_qualifications,
    reason = "the js_host path is the proof that session reexports the same type"
)]
fn session_types_are_host_types() {
    fn same<T>(left: T, right: T) -> (T, T) {
        (left, right)
    }
    let (value, _) = same(
        PluginValue::Bytes(vec![1]),
        javascript::host::PluginValue::Null,
    );
    assert_eq!(value, PluginValue::Bytes(vec![1]));
    let (result, _) = same(
        PluginResult::Failed {
            message: "x".into(),
        },
        javascript::host::PluginResult::Done(javascript::host::PluginValue::Null),
    );
    assert!(matches!(result, PluginResult::Failed { .. }));
    let (event, _host_event) = same(
        IoEvent::NativeStream {
            id: 3,
            event: StreamEvent::Item(PluginValue::Bytes(vec![9])),
        },
        javascript::host::IoEvent::NativeStream {
            id: 1,
            event: StreamEvent::Finished,
        },
    );
    fn into_host(event: IoEvent) -> javascript::host::IoEvent {
        event
    }
    fn into_session(event: javascript::host::IoEvent) -> IoEvent {
        event
    }
    let event = into_session(into_host(event));
    match event {
        IoEvent::NativeStream {
            id: 3,
            event: StreamEvent::Item(PluginValue::Bytes(data)),
        } => assert_eq!(data, vec![9]),
        _ => panic!("stream event"),
    }
    let _ = same(
        ModuleRegistry::new(vec![]),
        javascript::host::ModuleRegistry::new(vec![]),
    );
}

#[test]
fn session_registry_shares_callback_arcs() {
    let method = Method::sync("ping", |_| Ok(PluginValue::Str("pong".into())));
    let Invoke::Sync(original) = method.invoke.clone() else {
        panic!("sync callback");
    };
    let registry = ModuleRegistry::new(vec![Module {
        name: "Ping",
        exports: Default::default(),
        specifier: "app:ping",
        methods: vec![method],
    }]);
    let Invoke::Sync(found) = &registry
        .method("Ping", "ping")
        .unwrap_or_else(|| panic!("ping"))
        .invoke
    else {
        panic!("sync callback");
    };
    assert!(Arc::ptr_eq(&original, found));
    let filtered = registry.filtered(|module| module.specifier == "app:ping");
    let Invoke::Sync(kept) = &filtered
        .method("Ping", "ping")
        .unwrap_or_else(|| panic!("filtered ping"))
        .invoke
    else {
        panic!("sync callback");
    };
    assert!(Arc::ptr_eq(&original, kept));
    let config = ModuleConfig {
        registry: Arc::new(registry),
        ..ModuleConfig::disconnected()
    };
    match config.registry.call("Ping", "ping", &[]) {
        Ok(PluginValue::Str(text)) => assert_eq!(text, "pong"),
        _ => panic!("unexpected contract value"),
    }
}

#[test]
fn moved_contracts_keep_their_thread_bounds() {
    assert_send::<PluginValue>();
    assert_send::<PluginResult>();
    assert_send::<Settle>();
    assert_send::<Work>();
    assert_send::<IoEvent>();
    assert_send::<Method>();
    assert_send::<Module>();
    assert_send::<ImmediateExec>();
    assert_sync::<ImmediateExec>();
    assert_send::<Arc<dyn MethodExec>>();
    assert_sync::<Arc<dyn MethodExec>>();
}
