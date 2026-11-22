#![cfg(feature = "runtime")]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use javascript::host::{
    HostEvent, IoEvent, Method, Module, ModuleRegistry, PluginValue, StreamSink,
};
use javascript::runtime::{ModuleCatalog, ModuleStatus, RuntimeBuilder, RuntimeCounts};

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
fn weak_runtime_tracks_identity_and_shutdown_drops_roots() {
    let mut runtime = RuntimeBuilder::new().build().expect("runtime");
    let weak = runtime.downgrade();
    let other = RuntimeBuilder::new().build().expect("other");
    assert!(weak.same_runtime(&runtime));
    assert!(!weak.same_runtime(&other));
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
            .enqueue(IoEvent::Host(HostEvent {
                module: "Echo".into(),
                event: "tick".into(),
                payload: PluginValue::Null,
            }))
            .is_err()
    );
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
fn missing_plugin_terminal_times_out_from_the_first_clock_sample() {
    let held = Arc::new(Mutex::new(Vec::new()));
    let module = Module {
        name: "TimeoutProbe",
        specifier: "@impasto/timeout-probe",
        exports: Default::default(),
        methods: vec![Method::promise("hang", {
            let held = Arc::clone(&held);
            move |_, settle, _| held.lock().expect("lock").push(settle)
        })],
    };
    let catalog = ModuleCatalog::new(ModuleRegistry::try_new(vec![module]).expect("registry"))
        .expect("catalog");
    let mut runtime = RuntimeBuilder::new()
        .modules(catalog)
        .module(
            "app:timeout",
            "import { hang } from '@impasto/timeout-probe'; globalThis.error = undefined; hang().catch(value => { globalThis.error = String(value); });",
        )
        .limits(javascript::runtime::RuntimeLimits {
            request_timeout_ms: 50,
            ..javascript::runtime::RuntimeLimits::default()
        })
        .build()
        .expect("runtime");
    let ticket = runtime.start_module("app:timeout").expect("start");
    let first = runtime.drive(20_000).expect("clock origin");
    assert_eq!(first.schedule.in_flight, 1);
    assert_eq!(first.schedule.next_deadline_ms, Some(20_050));
    assert!(!first.schedule.runnable());
    runtime.drive(20_050).expect("time out");
    assert_eq!(runtime.counts().requests, 0);
    assert_eq!(runtime.counts().promises, 0);
    runtime
        .eval_script(
            "if (!error || !error.includes('expired')) throw new Error(error);",
            "check",
        )
        .expect("timeout");
    let _ = ticket;
    held.lock().expect("lock").clear();
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
    let sinks: Arc<Mutex<Vec<StreamSink>>> = Arc::new(Mutex::new(Vec::new()));
    let module = Module {
        name: "StreamProbe",
        specifier: "@impasto/stream-probe",
        exports: Default::default(),
        methods: vec![Method::stream_with_exec("open", {
            let sinks = Arc::clone(&sinks);
            move |_, sink, _| sinks.lock().expect("lock").push(sink)
        })],
    };
    let catalog = ModuleCatalog::new(ModuleRegistry::try_new(vec![module]).expect("registry"))
        .expect("catalog");
    let mut runtime = RuntimeBuilder::new()
        .modules(catalog)
        .module(
            "app:queue",
            "import { open } from '@impasto/stream-probe'; globalThis.opened = open();",
        )
        .limits(javascript::runtime::RuntimeLimits {
            queue_events: 1,
            stream_events: 1,
            queue_bytes: 4,
            stream_bytes: 4,
            ..javascript::runtime::RuntimeLimits::default()
        })
        .build()
        .expect("runtime");
    let _ticket = runtime.start_module("app:queue").expect("start");
    let mut sink = sinks.lock().expect("lock").pop().expect("stream sink");
    sink.send(PluginValue::Bytes(b"full".to_vec()), Duration::ZERO)
        .expect("fill queue");
    let blocked = runtime
        .enqueue(IoEvent::Host(HostEvent {
            module: "probe".into(),
            event: "tick".into(),
            payload: PluginValue::Null,
        }))
        .expect_err("ordinary event needs admission");
    assert!(matches!(
        blocked,
        javascript::runtime::EnqueueError::QueueFull(_)
    ));
    sink.finish();
    runtime.drive(2).expect("finish");
    assert_eq!(runtime.counts().queued_bytes, 4);
    runtime
        .eval_script(
            r#"
        let body;
        (async () => {
            const reader = opened.getReader();
            const first = await reader.read();
            body = new TextDecoder().decode(first.value);
            await reader.read();
        })();
    "#,
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
    let sinks: Arc<Mutex<Vec<StreamSink>>> = Arc::new(Mutex::new(Vec::new()));
    let module = Module {
        name: "StreamLoop",
        specifier: "@impasto/stream-loop",
        exports: Default::default(),
        methods: vec![Method::stream_with_exec("open", {
            let sinks = Arc::clone(&sinks);
            move |_, sink, _| sinks.lock().expect("lock").push(sink)
        })],
    };
    let catalog = ModuleCatalog::new(ModuleRegistry::try_new(vec![module]).expect("registry"))
        .expect("catalog");
    let mut runtime = RuntimeBuilder::new()
        .modules(catalog)
        .module(
            "app:loop",
            "import { open } from '@impasto/stream-loop'; globalThis.openStream = open;",
        )
        .build()
        .expect("runtime");
    let loop_ticket = runtime.start_module("app:loop").expect("start");
    runtime.drive(0).expect("init");
    let _ = runtime.take_module_result(loop_ticket);
    for step in 0..64 {
        runtime
            .eval_script(
                r#"
            globalThis.controller = new AbortController();
            globalThis.stream = globalThis.openStream(controller.signal);
        "#,
                "request",
            )
            .expect("request");
        let mut sink = sinks.lock().expect("lock").pop().expect("stream sink");
        if step % 2 == 0 {
            runtime
                .eval_script("controller.abort()", "cancel")
                .expect("cancel");
            assert!(sink.cancellation().is_cancelled());
            drop(sink);
        } else {
            sink.send(PluginValue::Bytes(b"ok".to_vec()), Duration::ZERO)
                .expect("send chunk");
            sink.finish();
            runtime
                .eval_script("stream.getReader().read();", "read")
                .expect("read");
        }
        runtime.drive(step).expect("drive");
        runtime
            .eval_script("globalThis.stream = null;", "drop")
            .expect("drop");
        let counts = runtime.counts();
        assert_eq!((counts.requests, counts.queued_bytes), (0, 0));
        assert_eq!(counts.peak_requests, 1);
    }
    let weak = runtime.downgrade();
    runtime.shutdown();
    assert!(!weak.is_open());
    assert_eq!(runtime.counts(), RuntimeCounts::default());
}
