use js::{ModuleTicket, RuntimeBuilder};

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
