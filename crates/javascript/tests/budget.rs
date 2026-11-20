#![cfg(feature = "runtime")]
#![allow(
    clippy::expect_used,
    reason = "test assertions identify the failed step"
)]

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use javascript::runtime::{ErrorKind, ModuleOutcome, Runtime, RuntimeLimits};

#[test]
fn execution_entry_has_a_budget() {
    if let Ok(entry) = std::env::var("IMPASTO_TEST_BUDGET_ENTRY") {
        run_budget_case(&entry);
        return;
    }
    let executable = std::env::current_exe().expect("test executable");
    for entry in ["eval", "timer", "module", "job", "listener"] {
        let mut child = Command::new(&executable)
            .args(["--exact", "execution_entry_has_a_budget", "--nocapture"])
            .env("IMPASTO_TEST_BUDGET_ENTRY", entry)
            .stdin(Stdio::null())
            .spawn()
            .expect("spawn budget test");
        let started = Instant::now();
        loop {
            if let Some(status) = child.try_wait().expect("child status") {
                assert!(status.success(), "{entry} budget probe failed");
                break;
            }
            if started.elapsed() >= Duration::from_secs(5) {
                child.kill().expect("kill over-budget child");
                child.wait().expect("wait for killed child");
                panic!("{entry} did not interrupt");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

fn run_budget_case(entry: &str) {
    let limits = RuntimeLimits {
        eval_budget_ms: 50,
        callback_budget_ms: 30,
        ..RuntimeLimits::default()
    };
    let reports = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
    let mut runtime = Runtime::builder()
        .limits(limits)
        .log({
            let reports = std::rc::Rc::clone(&reports);
            move |_, message| reports.borrow_mut().push(message.to_owned())
        })
        .module("loop", "await Promise.resolve(); for (;;) {}")
        .build()
        .expect("runtime");
    let interrupted = match entry {
        "eval" => {
            runtime
                .eval_script("for (;;) {}", "eval")
                .expect_err("interrupt")
                .message
        }
        "timer" => {
            runtime
                .eval_script("setTimeout(() => { for (;;) {} }, 0)", "timer")
                .expect("timer");
            runtime.drive(0).expect_err("interrupt").message
        }
        "module" => {
            let ticket = runtime.start_module("loop").expect("module ticket");
            match runtime.drive(0) {
                Err(error) => error.message,
                Ok(_) => match runtime.take_module_result(ticket).expect("result") {
                    Some(ModuleOutcome::Rejected(error)) => error.message,
                    _ => panic!("module must reject"),
                },
            }
        }
        "job" => {
            runtime
                .eval_script("Promise.resolve().then(() => { for (;;) {} })", "job")
                .expect("job");
            match runtime.drive(0) {
                Err(error) => error.message,
                Ok(_) => reports.borrow().join("\n"),
            }
        }
        "listener" => {
            runtime
                .listen("m", "e", "() => { for (;;) {} }")
                .expect("listener");
            let error = runtime
                .emit_listener("m", "e", "text")
                .expect_err("interrupt");
            assert_eq!(error.kind, ErrorKind::Script);
            error.message
        }
        _ => panic!("unknown budget entry"),
    };
    assert!(
        interrupted.contains("interrupted"),
        "{entry}: {interrupted}"
    );
    runtime
        .eval_script("globalThis.running = true;", "recovered")
        .expect("runtime recovers");
}

#[test]
fn nested_execution_cannot_extend_parent_budget() {
    let runtime = Runtime::builder()
        .limits(RuntimeLimits {
            eval_budget_ms: 400,
            callback_budget_ms: 30,
            ..RuntimeLimits::default()
        })
        .build()
        .expect("runtime");
    let extension = runtime.extension();
    {
        let _outer = extension.budget("listener", 7);
        let _inner = extension.budget("eval", 0);
        std::thread::sleep(Duration::from_millis(45));
        assert!(extension.past_deadline());
    }
    assert!(!extension.past_deadline());
}
