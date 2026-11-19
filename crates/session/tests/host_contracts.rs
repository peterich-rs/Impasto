//! Public host-contract baseline. These tests lock the behavior phase 1 moves
//! without changing it: one-shot settle, caller-thread execution, first-wins
//! lookup, the `@impasto/` allow-list, owned payloads, and the default HTTP stream.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;

use session::{
    HTTP_BODY_LIMIT, HttpClient, HttpError, HttpFailure, HttpPiece, HttpRequest, HttpResponse,
    ImmediateExec, Invoke, IoEvent, IoJob, Method, MethodExec, Module, ModuleConfig,
    ModuleRegistry, PluginResult, PluginValue, Settle, Work, document_relative, module_not_linked,
    shared, shared_read, shared_write,
};

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
    let registry = ModuleRegistry::new(modules);
    let names = |packages: &[&str]| -> Vec<&str> {
        let owned: Vec<String> = packages
            .iter()
            .map(|package| (*package).to_owned())
            .collect();
        let allowed = registry.restricted(&owned);
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

    let narrowed = registry.restricted(&["maps".into(), "clipboard".into()]);
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

struct FixedHttp {
    response: Result<HttpResponse, HttpError>,
    performs: AtomicUsize,
}

impl HttpClient for FixedHttp {
    fn perform(&self, _request: &HttpRequest) -> Result<HttpResponse, HttpError> {
        self.performs.fetch_add(1, Ordering::Relaxed);
        self.response.clone()
    }
}

fn request() -> HttpRequest {
    HttpRequest {
        token: 7,
        method: "GET".into(),
        url: "https://example.test/item".into(),
        headers: vec![("Accept".into(), "text/plain".into())],
        body: Vec::new(),
        cancel: Arc::new(AtomicBool::new(false)),
        timeout_ms: None,
    }
}

#[test]
fn default_stream_sends_head_then_one_chunk_and_stops() {
    let headers = vec![("Content-Type".into(), "text/plain".into())];
    let client = FixedHttp {
        response: Ok(HttpResponse {
            status: 201,
            headers: headers.clone(),
            body: b"abc".to_vec(),
        }),
        performs: AtomicUsize::new(0),
    };
    let mut seen = Vec::new();
    client
        .stream(&request(), &mut |piece| {
            match piece {
                HttpPiece::Head(status, got) => {
                    seen.push(format!("head:{status}:{}", got.len()));
                    assert_eq!(got, headers.as_slice());
                }
                HttpPiece::Chunk(bytes, done) => {
                    seen.push(format!("chunk:{}:{done}", bytes.len()));
                    assert_eq!(bytes, b"abc");
                }
            }
            true
        })
        .unwrap_or_else(|error| panic!("{error:?}"));
    assert_eq!(seen, ["head:201:1", "chunk:3:true"]);
    assert_eq!(client.performs.load(Ordering::Relaxed), 1);

    let empty = FixedHttp {
        response: Ok(HttpResponse {
            status: 204,
            headers: Vec::new(),
            body: Vec::new(),
        }),
        performs: AtomicUsize::new(0),
    };
    let mut pieces = 0;
    empty
        .stream(&request(), &mut |piece| {
            pieces += 1;
            assert!(matches!(piece, HttpPiece::Head(204, _)));
            true
        })
        .unwrap_or_else(|error| panic!("{error:?}"));
    assert_eq!(pieces, 1);

    let stopped = FixedHttp {
        response: Ok(HttpResponse {
            status: 200,
            headers: Vec::new(),
            body: b"more".to_vec(),
        }),
        performs: AtomicUsize::new(0),
    };
    let mut stopped_at = Vec::new();
    stopped
        .stream(&request(), &mut |piece| {
            stopped_at.push(match piece {
                HttpPiece::Head(_, _) => "head",
                HttpPiece::Chunk(_, _) => "chunk",
            });
            false
        })
        .unwrap_or_else(|error| panic!("{error:?}"));
    assert_eq!(stopped_at, ["head"]);

    let failure = HttpError::new(HttpFailure::Timeout, "deadline");
    let failing = FixedHttp {
        response: Err(failure.clone()),
        performs: AtomicUsize::new(0),
    };
    let mut called = false;
    match failing.stream(&request(), &mut |_piece| {
        called = true;
        true
    }) {
        Err(error) => {
            assert_eq!(error, failure);
            assert_eq!(error.dom_name(), "TimeoutError");
            assert!(!called);
        }
        Ok(()) => panic!("stream must return the perform error"),
    }
}

#[test]
fn failure_names_limits_and_document_paths_stay_exact() {
    let names = [
        (HttpFailure::Network, "TypeError"),
        (HttpFailure::Timeout, "TimeoutError"),
        (HttpFailure::Canceled, "AbortError"),
        (HttpFailure::TooLarge, "TypeError"),
        (HttpFailure::BadUrl, "TypeError"),
        (HttpFailure::BodyNotUtf8, "TypeError"),
    ];
    for (failure, name) in names {
        assert_eq!(failure.dom_name(), name);
        assert_eq!(HttpError::new(failure, "detail").dom_name(), name);
    }
    assert_eq!(HTTP_BODY_LIMIT, 4 * 1024 * 1024);

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
    let request = request();
    let response = HttpResponse {
        status: 200,
        headers: vec![("X-Test".into(), "1".into())],
        body: b"hi".to_vec(),
    };
    let error = HttpError::new(HttpFailure::BadUrl, "unsupported URL");
    let jobs = [
        IoJob::Http(request.clone()),
        IoJob::HttpStream(request),
        IoJob::Abort { token: 7 },
        IoJob::SocketOpen {
            id: 3,
            url: "wss://example.test".into(),
            protocols: vec!["chat".into()],
        },
        IoJob::SocketSend {
            id: 3,
            text: false,
            data: vec![9],
        },
        IoJob::SocketClose { id: 3 },
    ];
    assert!(matches!(jobs[0], IoJob::Http(ref got) if got.token == 7 && got.method == "GET"));
    assert!(
        matches!(jobs[1], IoJob::HttpStream(ref got) if got.url == "https://example.test/item")
    );
    assert!(matches!(jobs[2], IoJob::Abort { token: 7 }));
    assert!(matches!(
        jobs[3],
        IoJob::SocketOpen { id: 3, ref protocols, .. } if protocols == &["chat".to_owned()]
    ));
    assert!(matches!(jobs[4], IoJob::SocketSend { id: 3, text: false, ref data } if data == &[9]));
    assert!(matches!(jobs[5], IoJob::SocketClose { id: 3 }));

    let events = [
        IoEvent::Http {
            token: 7,
            result: Ok(response.clone()),
        },
        IoEvent::Http {
            token: 8,
            result: Err(error.clone()),
        },
        IoEvent::HttpHead {
            token: 7,
            status: 200,
            headers: response.headers,
        },
        IoEvent::HttpChunk {
            token: 7,
            bytes: b"hi".to_vec(),
            done: false,
            error: None,
        },
        IoEvent::HttpChunk {
            token: 7,
            bytes: Vec::new(),
            done: true,
            error: Some(error.clone()),
        },
        IoEvent::Plugin {
            token: 4,
            result: PluginResult::Done(PluginValue::Bytes(vec![1, 2])),
        },
        IoEvent::Socket {
            id: 3,
            event: "message".into(),
            text: true,
            data: b"hi".to_vec(),
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
        IoEvent::Http { token, result } => {
            assert_eq!(*token, 7);
            match result {
                Ok(body) => assert_eq!(body.body, b"hi"),
                Err(error) => panic!("{error:?}"),
            }
        }
        _ => panic!("unexpected contract value"),
    }
    match &events[1] {
        IoEvent::Http {
            token: 8,
            result: Err(got),
        } => assert_eq!(got, &error),
        _ => panic!("unexpected contract value"),
    }
    match &events[2] {
        IoEvent::HttpHead {
            token: 7,
            status: 200,
            headers,
        } => assert_eq!(headers.len(), 1),
        _ => panic!("unexpected contract value"),
    }
    match &events[3] {
        IoEvent::HttpChunk {
            bytes,
            done: false,
            error: None,
            ..
        } => assert_eq!(bytes, b"hi"),
        _ => panic!("unexpected contract value"),
    }
    match &events[4] {
        IoEvent::HttpChunk {
            done: true,
            error: Some(got),
            ..
        } => assert_eq!(got.kind, HttpFailure::BadUrl),
        _ => panic!("unexpected contract value"),
    }
    match &events[5] {
        IoEvent::Plugin {
            token: 4,
            result: PluginResult::Done(PluginValue::Bytes(bytes)),
        } => assert_eq!(bytes, &[1, 2]),
        _ => panic!("unexpected contract value"),
    }
    match &events[6] {
        IoEvent::Socket {
            id: 3,
            event,
            text: true,
            data,
        } => {
            assert_eq!(event, "message");
            assert_eq!(data, b"hi");
        }
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
        settle: Arc::new(|_token| Settle::new(|_result| {})),
        app_state: shared("active".to_owned()),
        launch_url: shared(Some("impasto://open".to_owned())),
    };
    match config.registry.call("Clipboard", "getString", &[]) {
        Ok(PluginValue::Str(text)) => assert_eq!(text, "clip"),
        _ => panic!("unexpected contract value"),
    }
    let narrowed = config.restrict(&[]);
    assert!(narrowed.registry.module("Clipboard").is_none());

    let disconnected = ModuleConfig::disconnected();
    assert_eq!(shared_read(&disconnected.app_state), "active");
    assert!(shared_read(&disconnected.launch_url).is_none());
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

trait DistinctRegistry {}
impl DistinctRegistry for ModuleRegistry {}
impl DistinctRegistry for js_host::ModuleRegistry {}

fn registry_types_differ() {
    fn hold<T: DistinctRegistry>() {}
    hold::<ModuleRegistry>();
    hold::<js_host::ModuleRegistry>();
}

#[test]
#[allow(
    unused_qualifications,
    reason = "the js_host path is the proof that session reexports the same type"
)]
fn session_types_are_host_types() {
    fn same<T>(left: T, right: T) -> (T, T) {
        (left, right)
    }
    let (value, _) = same(PluginValue::Bytes(vec![1]), js_host::PluginValue::Null);
    assert_eq!(value, PluginValue::Bytes(vec![1]));
    let (result, _) = same(
        PluginResult::Failed {
            message: "x".into(),
        },
        js_host::PluginResult::Done(js_host::PluginValue::Null),
    );
    assert!(matches!(result, PluginResult::Failed { .. }));
    let (event, _host_event) = same(
        IoEvent::Socket {
            id: 3,
            event: "open".into(),
            text: true,
            data: vec![9],
        },
        js_host::IoEvent::Http {
            token: 1,
            result: Ok(js_host::HttpResponse {
                status: 200,
                headers: Vec::new(),
                body: Vec::new(),
            }),
        },
    );
    fn into_host(event: IoEvent) -> js_host::IoEvent {
        event
    }
    fn into_session(event: js_host::IoEvent) -> IoEvent {
        event
    }
    let event = into_session(into_host(event));
    match event {
        IoEvent::Socket { id: 3, data, .. } => assert_eq!(data, vec![9]),
        _ => panic!("socket event"),
    }
    registry_types_differ();
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
    let host = registry.as_host_registry();
    let Invoke::Sync(found) = &host
        .method("Ping", "ping")
        .unwrap_or_else(|| panic!("ping"))
        .invoke
    else {
        panic!("sync callback");
    };
    assert!(Arc::ptr_eq(&original, found));
    let filtered = host.filtered(|module| module.specifier == "app:ping");
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
    assert_send::<HttpRequest>();
    assert_send::<HttpResponse>();
    assert_send::<HttpError>();
    assert_send::<IoJob>();
    assert_send::<IoEvent>();
    assert_send::<Method>();
    assert_send::<Module>();
    assert_send::<ImmediateExec>();
    assert_sync::<ImmediateExec>();
    assert_send::<Arc<dyn MethodExec>>();
    assert_sync::<Arc<dyn MethodExec>>();
    assert_send::<Arc<dyn HttpClient>>();
    assert_sync::<Arc<dyn HttpClient>>();
    let piece = HttpPiece::Chunk(b"a", true);
    fn check_piece<T: Send + Sync>(_: &T) {}
    check_piece(&piece);
}
