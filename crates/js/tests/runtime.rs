use std::sync::atomic::Ordering;

use js::{ModuleTicket, RuntimeBuilder};
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
    match runtime.start_module("scene") {
        Err(error) => assert!(error.message.contains("not installed"), "{}", error.message),
        Ok(ticket) => panic!("scene returned {ticket:?}"),
    }
    match runtime.start_module("missing") {
        Err(error) => assert!(
            error.message.contains("not registered"),
            "{}",
            error.message
        ),
        Ok(ModuleTicket::Fulfilled | ModuleTicket::Rejected) => panic!("missing module"),
    }
}
