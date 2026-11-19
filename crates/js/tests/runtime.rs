use std::sync::atomic::Ordering;

use js::{ModuleStatus, RuntimeBuilder, RuntimeCounts};
use js_host::{HttpResponse, IoEvent, IoJob};

#[test]
fn headless_script_promise_and_timer_share_the_runtime() {
    let mut runtime = RuntimeBuilder::new().build().expect("runtime");
    runtime
        .eval_script(
            "globalThis.flag = 0; Promise.resolve().then(function () { globalThis.flag = 1; }); setTimeout(function () { globalThis.flag = 2; }, 0);",
            "entry.js",
        )
        .expect("eval");
    let first = runtime.drive(0).expect("drive");
    assert_eq!(first.timers_ran, 1);
    let second = runtime.drive(1).expect("drive");
    assert_eq!(second.timers_ran, 0);
    runtime.shutdown();
    let closed = runtime.eval_script("1", "again.js");
    assert!(closed.is_err());
}

#[test]
fn headless_fetch_submits_an_owned_job_and_settles_on_drive() {
    let mut runtime = RuntimeBuilder::new().build().expect("runtime");
    runtime
        .eval_script(
            "fetch('https://example.test/item').then(function (response) { return response.text(); }).then(function (text) { globalThis.body = text; });",
            "fetch.js",
        )
        .expect("fetch");
    let jobs = runtime.take_jobs();
    let token = match jobs.first() {
        Some(IoJob::Http(request)) => {
            assert_eq!(request.url, "https://example.test/item");
            request.token
        }
        other => panic!("expected an HTTP job, got {other:?}"),
    };
    runtime
        .enqueue(IoEvent::Http {
            token,
            result: Ok(HttpResponse {
                status: 200,
                headers: Vec::new(),
                body: b"ok".to_vec(),
            }),
        })
        .expect("enqueue");
    runtime.drive(0).expect("drive");
    runtime
        .eval_script(
            "if (globalThis.body !== 'ok') throw new Error(String(globalThis.body));",
            "check.js",
        )
        .expect("body");
    runtime.shutdown();
    assert!(
        runtime
            .enqueue(IoEvent::Http {
                token: 1,
                result: Ok(HttpResponse {
                    status: 200,
                    headers: Vec::new(),
                    body: Vec::new(),
                }),
            })
            .is_err()
    );
}

#[test]
fn headless_abort_marks_the_request_and_queues_abort() {
    let mut runtime = RuntimeBuilder::new().build().expect("runtime");
    runtime
        .eval_script(
            "var ctrl = new AbortController(); fetch('https://example.test/late', { signal: ctrl.signal }); ctrl.abort();",
            "abort.js",
        )
        .expect("abort");
    let jobs = runtime.take_jobs();
    let (flag, token) = match jobs.first() {
        Some(IoJob::Http(request)) => (std::sync::Arc::clone(&request.cancel), request.token),
        other => panic!("expected an HTTP job, got {other:?}"),
    };
    assert!(flag.load(Ordering::Relaxed));
    assert!(matches!(jobs.get(1), Some(IoJob::Abort { token: got }) if *got == token));
    runtime.shutdown();
    runtime.shutdown();
}

#[test]
fn headless_refuses_scene_and_unknown_modules() {
    let mut runtime = RuntimeBuilder::new().build().expect("runtime");
    for name in ["scene", "reactive", "#reactive", "missing"] {
        match runtime.start_module(name) {
            Err(error) => assert!(
                error.message.contains("not registered"),
                "{name}: {}",
                error.message
            ),
            Ok(ticket) => panic!("{name} returned {ticket:?}"),
        }
    }
}

#[test]
fn memory_module_import_and_native_factory_share_one_loader() {
    let mut runtime = RuntimeBuilder::new()
        .module(
            "app.js",
            "import { answer } from 'answer'; import { extra } from './lib.js'; globalThis.value = answer + extra;",
        )
        .module("lib.js", "export const extra = 1;")
        .native("answer", Box::new(AnswerFactory))
        .build()
        .expect("runtime");
    let ticket = runtime.start_module("app.js").expect("start");
    assert_eq!(
        runtime.module_status(ticket).expect("status"),
        ModuleStatus::Fulfilled
    );
    runtime
        .eval_script(
            "if (globalThis.value !== 8) throw new Error(String(globalThis.value));",
            "check.js",
        )
        .expect("value");
}

#[test]
fn duplicate_module_name_fails_at_build() {
    match RuntimeBuilder::new()
        .module("app.js", "export const a = 1;")
        .module("app.js", "export const a = 2;")
        .build()
    {
        Err(error) => assert!(
            error.message.contains("already registered"),
            "{}",
            error.message
        ),
        Ok(_) => panic!("duplicate module was accepted"),
    }
}

#[test]
fn top_level_await_stays_pending_until_drive() {
    let mut runtime = RuntimeBuilder::new()
        .module(
            "slow.js",
            "globalThis.seen = 0; await new Promise(function (resolve) { globalThis.release = resolve; }); globalThis.seen = 1; export const done = true;",
        )
        .build()
        .expect("runtime");
    let ticket = runtime.start_module("slow.js").expect("start");
    assert_eq!(
        runtime.module_status(ticket).expect("pending"),
        ModuleStatus::Pending
    );
    runtime
        .eval_script(
            "if (globalThis.seen !== 0) throw new Error('ran early'); globalThis.release();",
            "release.js",
        )
        .expect("release");
    assert_eq!(
        runtime.module_status(ticket).expect("still pending"),
        ModuleStatus::Pending
    );
    runtime.drive(0).expect("drive");
    assert_eq!(
        runtime.module_status(ticket).expect("fulfilled"),
        ModuleStatus::Fulfilled
    );
    runtime
        .eval_script(
            "if (globalThis.seen !== 1) throw new Error(String(globalThis.seen));",
            "seen.js",
        )
        .expect("seen");
    let mut other = RuntimeBuilder::new().build().expect("other");
    assert!(other.module_status(ticket).is_err());
}

#[test]
fn sync_http_client_settles_on_drive_without_a_second_enqueue() {
    let mut runtime = RuntimeBuilder::new()
        .http_client(std::sync::Arc::new(SyncOk))
        .build()
        .expect("runtime");
    runtime
        .eval_script(
            "fetch('https://example.test/sync').then(function (response) { return response.text(); }).then(function (text) { globalThis.body = text; });",
            "sync.js",
        )
        .expect("fetch");
    assert!(matches!(runtime.take_jobs().first(), Some(IoJob::Http(_))));
    runtime.drive(0).expect("drive");
    runtime
        .eval_script(
            "if (globalThis.body !== 'sync') throw new Error(String(globalThis.body));",
            "check.js",
        )
        .expect("body");
}

#[test]
fn job_round_stops_at_32_and_timer_batch_stops_at_64() {
    let mut runtime = RuntimeBuilder::new().build().expect("runtime");
    let mut jobs = String::from("var n = 0;");
    for _ in 0..40 {
        jobs.push_str("Promise.resolve().then(function () { n += 1; });");
    }
    runtime.eval_script(&jobs, "jobs.js").expect("queue");
    let error = runtime.drive(0).expect_err("limit");
    assert!(
        error.message.contains("job round limit"),
        "{}",
        error.message
    );

    let mut runtime = RuntimeBuilder::new().build().expect("runtime");
    let mut timers = String::from("globalThis.ran = 0;");
    for _ in 0..65 {
        timers.push_str("setTimeout(function () { globalThis.ran += 1; }, 0);");
    }
    runtime.eval_script(&timers, "timers.js").expect("timers");
    assert_eq!(runtime.drive(0).expect("first").timers_ran, 64);
    assert_eq!(runtime.drive(1).expect("second").timers_ran, 1);
    runtime
        .eval_script(
            "if (globalThis.ran !== 65) throw new Error(String(globalThis.ran));",
            "check.js",
        )
        .expect("ran");
}

#[test]
fn socket_completion_and_shutdown_drop_roots() {
    let mut runtime = RuntimeBuilder::new().build().expect("runtime");
    let weak = runtime.downgrade();
    let other = RuntimeBuilder::new().build().expect("other");
    assert!(weak.same_runtime(&runtime));
    assert!(!weak.same_runtime(&other));
    runtime
        .eval_script(
            "var socket = new WebSocket('ws://example.test/sock'); socket.onopen = function () { globalThis.opened = 1; }; socket.onmessage = function (event) { globalThis.message = String(event.data); };",
            "socket.js",
        )
        .expect("socket");
    let id = match runtime.take_jobs().first() {
        Some(IoJob::SocketOpen { id, url, .. }) => {
            assert_eq!(url, "ws://example.test/sock");
            *id
        }
        other => panic!("expected a socket job, got {other:?}"),
    };
    assert!(runtime.counts().timers == 0);
    runtime
        .enqueue(IoEvent::Socket {
            id,
            event: "open".into(),
            text: true,
            data: Vec::new(),
        })
        .expect("open");
    runtime
        .enqueue(IoEvent::Socket {
            id,
            event: "message".into(),
            text: true,
            data: b"hi".to_vec(),
        })
        .expect("message");
    runtime.drive(0).expect("drive");
    runtime
        .eval_script(
            "if (globalThis.opened !== 1 || globalThis.message !== 'hi') throw new Error(String(globalThis.opened) + ':' + String(globalThis.message));",
            "check.js",
        )
        .expect("socket state");
    let failed = runtime.eval_script("throw new Error('boom');", "bad.js");
    assert!(failed.is_err());
    runtime
        .eval_script("globalThis.after = 1;", "after.js")
        .expect("still open");
    runtime.shutdown();
    assert!(!weak.is_open());
    assert_eq!(runtime.counts(), RuntimeCounts::default());
    runtime.shutdown();
    assert!(
        runtime
            .enqueue(IoEvent::Socket {
                id,
                event: "close".into(),
                text: true,
                data: Vec::new(),
            })
            .is_err()
    );
}

struct SyncOk;

impl js_host::HttpClient for SyncOk {
    fn perform(&self, request: &js_host::HttpRequest) -> Result<HttpResponse, js_host::HttpError> {
        assert_eq!(request.url, "https://example.test/sync");
        Ok(HttpResponse {
            status: 200,
            headers: Vec::new(),
            body: b"sync".to_vec(),
        })
    }
}

#[test]
fn sync_client_streams_head_and_chunk_through_the_shared_delivery() {
    let mut runtime = RuntimeBuilder::new()
        .http_client(std::sync::Arc::new(StreamOk))
        .build()
        .expect("runtime");
    runtime
        .eval_script(
            "fetchStream('https://example.test/stream').then(function (response) { globalThis.status = response.status; return response.getReader().read(); }).then(function (part) { globalThis.text = new TextDecoder().decode(part.value); });",
            "stream.js",
        )
        .expect("fetchStream");
    runtime.drive(0).expect("drive");
    runtime
        .eval_script(
            "if (globalThis.status !== 200 || globalThis.text !== 'abc') throw new Error(String(globalThis.status) + ':' + String(globalThis.text));",
            "check.js",
        )
        .expect("stream body");
}

struct StreamOk;

impl js_host::HttpClient for StreamOk {
    fn perform(&self, _request: &js_host::HttpRequest) -> Result<HttpResponse, js_host::HttpError> {
        Ok(HttpResponse {
            status: 201,
            headers: Vec::new(),
            body: Vec::new(),
        })
    }

    fn stream(
        &self,
        request: &js_host::HttpRequest,
        sink: &mut dyn FnMut(js_host::HttpPiece<'_>) -> bool,
    ) -> Result<(), js_host::HttpError> {
        assert_eq!(request.url, "https://example.test/stream");
        if sink(js_host::HttpPiece::Head(200, &[])) {
            sink(js_host::HttpPiece::Chunk(b"abc", true));
        }
        Ok(())
    }
}

struct AnswerFactory;

impl js::NativeFactory for AnswerFactory {
    fn load<'js>(
        &self,
        ctx: &rquickjs::Ctx<'js>,
        name: &str,
    ) -> rquickjs::Result<rquickjs::Module<'js, rquickjs::module::Declared>> {
        rquickjs::Module::declare_def::<Answer, _>(ctx.clone(), name)
    }
}

struct Answer;

impl rquickjs::module::ModuleDef for Answer {
    fn declare(decl: &rquickjs::module::Declarations) -> rquickjs::Result<()> {
        decl.declare("answer")?;
        Ok(())
    }

    fn evaluate<'js>(
        _ctx: &rquickjs::Ctx<'js>,
        exports: &rquickjs::module::Exports<'js>,
    ) -> rquickjs::Result<()> {
        exports.export("answer", 7)?;
        Ok(())
    }
}
