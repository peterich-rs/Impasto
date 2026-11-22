#![cfg(feature = "runtime")]

use std::sync::atomic::Ordering;

use javascript::host::{HttpResponse, IoEvent, IoJob};
use javascript::runtime::{ModuleStatus, Runtime, RuntimeBuilder, RuntimeCounts};

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
fn user_interaction_remains_pending_after_the_transport_deadline() {
    use javascript::host::{Method, Module, ModuleRegistry, PluginResult, PluginValue};
    use javascript::runtime::{ModuleCatalog, ModuleOutcome};

    let (send, receive) = std::sync::mpsc::channel();
    let module = Module {
        name: "Picker",
        specifier: "@impasto/test-picker",
        exports: Default::default(),
        methods: vec![
            Method::promise("pick", move |_, settle, _| {
                send.send(settle).expect("retain picker reply");
            })
            .awaits_user(),
        ],
    };
    let catalog = ModuleCatalog::new(ModuleRegistry::try_new(vec![module]).expect("registry"))
        .expect("catalog");
    let mut runtime = RuntimeBuilder::new().modules(catalog).module(
        "app:picker",
        "import {pick} from '@impasto/test-picker'; if (await pick() !== 'picked.txt') throw Error('wrong result');",
    ).build().expect("runtime");
    let ticket = runtime.start_module("app:picker").expect("start picker");
    runtime.drive(0).expect("initial clock");
    runtime
        .drive(120_000)
        .expect("leave the picker open for two minutes");
    assert!(
        runtime
            .take_module_result(ticket)
            .expect("pending picker")
            .is_none()
    );
    receive
        .try_recv()
        .expect("picker started")
        .send(PluginResult::Done(PluginValue::Str("picked.txt".into())));
    runtime.drive(120_001).expect("deliver picked document");
    assert_eq!(
        runtime.take_module_result(ticket).expect("picked result"),
        Some(ModuleOutcome::Fulfilled)
    );
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
        Some(IoJob::HttpStream(request)) => {
            assert_eq!(request.url, "https://example.test/item");
            request.token
        }
        other => panic!("expected an HTTP job, got {other:?}"),
    };
    runtime
        .enqueue(IoEvent::HttpHead {
            token,
            status: 200,
            headers: Vec::new(),
        })
        .expect("response head");
    runtime
        .enqueue(IoEvent::HttpChunk {
            token,
            bytes: b"ok".to_vec(),
            done: true,
            error: None,
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
        Some(IoJob::HttpStream(request)) => (std::sync::Arc::clone(&request.cancel), request.token),
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
    let other = RuntimeBuilder::new().build().expect("other");
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
    assert!(runtime.take_jobs().is_empty());
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

impl javascript::host::HttpClient for SyncOk {
    fn perform(
        &self,
        request: &javascript::host::HttpRequest,
    ) -> Result<HttpResponse, javascript::host::HttpError> {
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
            "fetch('https://example.test/stream').then(function (response) { globalThis.status = response.status; return response.body.getReader().read(); }).then(function (part) { globalThis.text = new TextDecoder().decode(part.value); });",
            "stream.js",
        )
        .expect("fetch");
    runtime.drive(0).expect("drive");
    runtime
        .eval_script(
            "if (globalThis.status !== 200 || globalThis.text !== 'abc') throw new Error(String(globalThis.status) + ':' + String(globalThis.text));",
            "check.js",
        )
        .expect("stream body");
}

struct StreamOk;

impl javascript::host::HttpClient for StreamOk {
    fn perform(
        &self,
        _request: &javascript::host::HttpRequest,
    ) -> Result<HttpResponse, javascript::host::HttpError> {
        Ok(HttpResponse {
            status: 201,
            headers: Vec::new(),
            body: Vec::new(),
        })
    }

    fn stream(
        &self,
        request: &javascript::host::HttpRequest,
        sink: &mut dyn FnMut(javascript::host::HttpPiece<'_>) -> bool,
    ) -> Result<(), javascript::host::HttpError> {
        assert_eq!(request.url, "https://example.test/stream");
        if sink(javascript::host::HttpPiece::Head(200, &[])) {
            sink(javascript::host::HttpPiece::Chunk(b"abc", true));
        }
        Ok(())
    }
}

struct AnswerFactory;

impl javascript::runtime::NativeFactory for AnswerFactory {
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

#[test]
fn timer_extra_arguments_and_yielding_follow_turns() {
    let mut runtime = RuntimeBuilder::new()
        .drive_policy(javascript::runtime::DrivePolicy::Yielding)
        .limits(javascript::runtime::RuntimeLimits {
            job_batch: 2,
            ..javascript::runtime::RuntimeLimits::default()
        })
        .build()
        .expect("runtime");
    runtime
        .eval_script(
            r#"
        let observed = [];
        if (performance.now() !== 0) throw new Error('startup clock');
        for (let i = 0; i < 5; i++) Promise.resolve().then(() => observed.push(i));
        const id = setTimeout((...extra) => {
            observed.push(extra.join(','));
            setTimeout(() => observed.push('next'), 0);
        }, -4, 1, 2, 3, 4, 5, 6, 7, 8, 9);
        clearInterval(id);
    "#,
            "timer.js",
        )
        .expect("script");
    let first = runtime.drive(300).expect("first");
    assert_eq!((first.timers_ran, first.jobs_ran), (1, 2));
    assert!(first.schedule.jobs_pending && first.schedule.due_next_turn);
    let second = runtime.drive(305).expect("second");
    assert_eq!((second.timers_ran, second.jobs_ran), (1, 2));
    runtime.drive(310).expect("last");
    runtime
        .eval_script(
            r#"
        if (String(observed) !== '1,2,3,4,5,6,7,8,0,1,next,2,3,4' || performance.now() !== 10)
            throw new Error(String(observed));
    "#,
            "check.js",
        )
        .expect("trace");
}

#[test]
fn abort_resolves_locally_and_ignores_a_late_success() {
    let mut runtime = RuntimeBuilder::new().build().expect("runtime");
    runtime
        .eval_script(
            r#"
        const abort = new AbortController();
        let result;
        fetch('https://example.test/abort', {signal: abort.signal}).then(
            () => result = 'incorrect success',
            error => result = error,
        );
    "#,
            "request",
        )
        .expect("request");
    let jobs = runtime.take_jobs();
    let Some(IoJob::HttpStream(request)) = jobs.first() else {
        panic!("expected HTTP request")
    };
    runtime
        .eval_script("abort.abort();", "abort")
        .expect("abort");
    assert!(request.cancel.load(Ordering::Acquire));
    assert_eq!(runtime.counts().requests, 0);
    assert_eq!(runtime.counts().promises, 0);
    runtime
        .enqueue(IoEvent::HttpHead {
            token: request.token,
            status: 204,
            headers: Vec::new(),
        })
        .expect("late event");
    runtime.drive(500).expect("drive");
    runtime
        .eval_script(
            "if (!(result instanceof DOMException) || result.name !== 'AbortError') throw new Error(result);",
            "check",
        )
        .expect("abort result");
    assert_eq!(runtime.counts().discarded_events, 1);
}

#[test]
fn missing_stream_terminal_times_out_from_the_first_clock_sample() {
    let mut runtime = RuntimeBuilder::new()
        .limits(javascript::runtime::RuntimeLimits {
            request_timeout_ms: 50,
            ..javascript::runtime::RuntimeLimits::default()
        })
        .build()
        .expect("runtime");
    runtime
        .eval_script(
            r#"
        let error;
        fetch('https://example.test/timeout').catch(value => error = value.message);
    "#,
            "entry",
        )
        .expect("fetch");
    let jobs = runtime.take_jobs();
    assert!(matches!(jobs.first(), Some(IoJob::HttpStream(_))));
    let first = runtime.drive(20_000).expect("clock origin");
    assert_eq!(first.schedule.in_flight, 1);
    assert_eq!(first.schedule.next_deadline_ms, Some(20_050));
    assert!(!first.schedule.runnable());
    runtime.drive(20_050).expect("time out");
    assert_eq!(runtime.counts().requests, 0);
    assert_eq!(runtime.counts().promises, 0);
    runtime
        .eval_script(
            "if (error !== 'TimeoutError') throw new Error(error);",
            "check",
        )
        .expect("timeout");
}

#[test]
fn ticket_result_is_consumed_and_observation_can_be_abandoned() {
    let mut runtime = RuntimeBuilder::new()
        .module(
            "bad",
            "await Promise.resolve(); throw new Error('module failure')",
        )
        .module(
            "abandon",
            "await new Promise(resolve => { globalThis.finish = resolve }); globalThis.done = true",
        )
        .build()
        .expect("runtime");
    let rejected = runtime.start_module("bad").expect("start");
    let pending = runtime.start_module("abandon").expect("start pending");
    assert_eq!(runtime.take_module_result(pending).expect("poll"), None);
    runtime.abandon_module(pending).expect("abandon");
    runtime.eval_script("finish()", "release").expect("release");
    runtime.drive(20).expect("drive");
    let outcome = runtime.take_module_result(rejected).expect("consume");
    let Some(javascript::runtime::ModuleOutcome::Rejected(error)) = outcome else {
        panic!("{outcome:?}")
    };
    assert!(error.message.contains("module failure"));
    assert_eq!(error.module.as_deref(), Some("bad"));
    assert_eq!(
        runtime
            .take_module_result(rejected)
            .expect_err("taken")
            .kind,
        javascript::runtime::ErrorKind::UnknownTicket
    );
    assert_eq!(runtime.counts().tickets, 0);
    runtime
        .eval_script(
            "if (!done) throw new Error('abandoned module stopped');",
            "check",
        )
        .expect("TLA still completes");
}

#[test]
fn a_full_data_queue_keeps_space_for_the_terminal() {
    let mut runtime = RuntimeBuilder::new()
        .limits(javascript::runtime::RuntimeLimits {
            queue_events: 1,
            stream_events: 1,
            queue_bytes: 4,
            stream_bytes: 4,
            ..javascript::runtime::RuntimeLimits::default()
        })
        .build()
        .expect("runtime");
    // The outbound admission counts URLs as well as data, so use a test port
    // that accepts the request directly to exercise the inbound limits.
    let (sender, receiver) = std::sync::mpsc::channel();
    runtime.configure_io(javascript::runtime::IoBindings::external(Box::new(
        move |job| {
            sender.send(job).expect("accept work");
        },
    )));
    runtime
        .eval_script(
            "let opened; fetch('https://example.test').then(value => opened = value);",
            "entry",
        )
        .expect("fetch");
    let IoJob::HttpStream(request) = receiver.try_recv().expect("stream request") else {
        panic!("expected stream request");
    };
    let token = request.token;
    runtime
        .enqueue(IoEvent::HttpHead {
            token,
            status: 200,
            headers: Vec::new(),
        })
        .expect("head");
    runtime.drive(1).expect("open");
    runtime
        .enqueue(IoEvent::HttpChunk {
            token,
            bytes: b"full".to_vec(),
            done: false,
            error: None,
        })
        .expect("fill queue");
    let blocked = runtime
        .enqueue(IoEvent::Host(javascript::host::HostEvent {
            module: "probe".into(),
            event: "tick".into(),
            payload: javascript::host::PluginValue::Null,
        }))
        .expect_err("ordinary event needs admission");
    assert!(matches!(
        blocked,
        javascript::runtime::EnqueueError::QueueFull(_)
    ));
    runtime
        .enqueue(IoEvent::HttpChunk {
            token,
            bytes: Vec::new(),
            done: true,
            error: None,
        })
        .expect("reserved terminal");
    runtime.drive(2).expect("finish");
    assert_eq!(runtime.counts().queued_bytes, 4);
    runtime
        .eval_script(
            "let body; opened.text().then(value => body = value);",
            "consume",
        )
        .expect("read");
    runtime.drive(3).expect("settle");
    runtime
        .eval_script(
            "if (body !== 'full') throw new Error(String(body));",
            "check",
        )
        .expect("body");
    assert_eq!(runtime.counts().queued_bytes, 0);
}

#[test]
fn repeated_completion_and_cancellation_release_requests() {
    let mut runtime = Runtime::builder().build().expect("runtime");
    for step in 0..64 {
        runtime
            .eval_script(
                r#"
            globalThis.controller = new AbortController();
            fetch('https://example.test/resource', {signal: controller.signal}).catch(() => {});
        "#,
                "request",
            )
            .expect("request");
        let jobs = runtime.take_jobs();
        let Some(IoJob::HttpStream(request)) = jobs.first() else {
            panic!("network work")
        };
        if step % 2 == 0 {
            runtime
                .eval_script("controller.abort()", "cancel")
                .expect("cancel");
            assert!(request.cancel.load(Ordering::Acquire));
            drop(runtime.take_jobs());
        } else {
            runtime
                .enqueue(IoEvent::HttpHead {
                    token: request.token,
                    status: 200,
                    headers: Vec::new(),
                })
                .expect("response head");
            runtime
                .enqueue(IoEvent::HttpChunk {
                    token: request.token,
                    bytes: b"ok".to_vec(),
                    done: true,
                    error: None,
                })
                .expect("completion");
        }
        runtime.drive(step).expect("drive");
        let counts = runtime.counts();
        assert_eq!(
            (counts.requests, counts.promises, counts.queued_bytes),
            (0, 0, 0)
        );
        assert_eq!((counts.peak_requests, counts.peak_promises), (1, 1));
    }
    let weak = runtime.downgrade();
    runtime.shutdown();
    assert!(!weak.is_open());
    assert_eq!(runtime.counts(), RuntimeCounts::default());
}

#[test]
fn a_throwing_delivery_releases_its_promise_and_preserves_the_next_event() {
    let mut runtime = Runtime::builder().build().expect("runtime");
    runtime.eval_script(r#"
        let streamError, responseText;
        fetch('https://example.test/broken').catch(error => streamError = String(error));
        fetch('https://example.test/next').then(response => response.text()).then(text => responseText = text);
        const receive = __host._httpStream;
        __host._httpStream = (...args) => { __host._httpStream = receive; throw new Error('delivery error'); };
    "#, "requests").expect("requests");
    let jobs = runtime.take_jobs();
    let [IoJob::HttpStream(stream), IoJob::HttpStream(whole)] = &jobs[..] else {
        panic!("expected two jobs")
    };
    runtime
        .enqueue(IoEvent::HttpHead {
            token: stream.token,
            status: 200,
            headers: Vec::new(),
        })
        .expect("first");
    runtime
        .enqueue(IoEvent::HttpHead {
            token: whole.token,
            status: 200,
            headers: Vec::new(),
        })
        .expect("response head");
    runtime
        .enqueue(IoEvent::HttpChunk {
            token: whole.token,
            bytes: b"next".to_vec(),
            done: true,
            error: None,
        })
        .expect("second");
    let error = runtime.drive(0).expect_err("failed hook");
    assert!(error.message.contains("delivery error"));
    assert_eq!(runtime.counts().requests, 1);
    assert_eq!(runtime.counts().promises, 1);
    runtime.drive(1).expect("next delivery");
    runtime
        .eval_script(
            r#"
        if (!streamError.includes('could not be converted') || responseText !== 'next') {
            throw new Error('lost dispatch');
        }
    "#,
            "check",
        )
        .expect("retained events");
    assert_eq!(runtime.counts().promises, 0);
}
