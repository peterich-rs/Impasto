use std::sync::atomic::Ordering;

use js::{ModuleStatus, RuntimeBuilder};
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
