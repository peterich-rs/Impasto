#![cfg(feature = "runtime")]
#![allow(clippy::expect_used, reason = "regression tests name each assertion")]

use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use javascript::host::{
    CancelToken, Method, ModuleRegistry, NativeExtension, NativeResource, PluginValue,
};
use javascript::runtime::{ModuleCatalog, ModuleOutcome, Runtime};
use javascript::standard::EntropySource;

#[test]
fn managed_resource_close_gc_and_shutdown_release_on_the_owner() {
    struct Counter {
        value: Rc<usize>,
        dropped: Arc<AtomicUsize>,
    }

    impl Drop for Counter {
        fn drop(&mut self) {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    impl NativeResource for Counter {
        fn call(
            &mut self,
            method: &str,
            _: &[PluginValue],
            cancel: &CancelToken,
        ) -> Result<PluginValue, String> {
            assert!(!cancel.is_cancelled());
            assert_eq!(method, "increment");
            *Rc::make_mut(&mut self.value) += 1;
            Ok(PluginValue::F64(*self.value as f64))
        }
    }

    let dropped = Arc::new(AtomicUsize::new(0));
    let method = {
        let dropped = Arc::clone(&dropped);
        Method::resource("open", move |_| {
            Ok(Box::new(Counter {
                value: Rc::new(0),
                dropped: Arc::clone(&dropped),
            }))
        })
    };
    let mut runtime = Runtime::builder()
        .modules(catalog(method))
        .module(
            "app:test",
            "import {open} from '@impasto/test-native';
             const resource = open();
             if (resource.call('increment') !== 1) throw Error('resource did not run');
             resource.close(); resource.close();
             try { resource.call('increment'); throw Error('closed resource accepted work'); }
             catch (error) { if (!(error instanceof TypeError)) throw error; }
             let abandoned = open();
             abandoned.self = abandoned;
             abandoned = null;
             globalThis.stillOpen = open();",
        )
        .build()
        .expect("build resource runtime");
    let ticket = runtime.start_module("app:test").expect("resource module");
    runtime.drive(0).expect("run resource module");
    assert_eq!(
        runtime.take_module_result(ticket).expect("result"),
        Some(ModuleOutcome::Fulfilled)
    );
    runtime.extension().with(|ctx| ctx.run_gc());
    assert_eq!(dropped.load(Ordering::Relaxed), 2);
    runtime.shutdown();
    assert_eq!(dropped.load(Ordering::Relaxed), 3);
}

#[test]
fn extension_stream_carries_structured_values_credit_and_abort() {
    let (aborted, cancellation) = mpsc::channel();
    let method = Method::stream("open", move |args, sink| {
        assert_eq!(args, [PluginValue::Str("sample".into())]);
        sink.send(
            PluginValue::Map(vec![("data".into(), PluginValue::Str("body".into()))]),
            Duration::ZERO,
        )
        .map_err(|error| error.to_string())?;
        assert!(matches!(
            sink.send(PluginValue::Str("later".into()), Duration::ZERO),
            Err(javascript::host::StreamSendError::Backpressure)
        ));
        aborted
            .send(sink.cancellation())
            .map_err(|error| error.to_string())
    });
    let mut runtime = Runtime::builder()
        .limits(javascript::runtime::RuntimeLimits {
            stream_events: 1,
            ..Default::default()
        })
        .modules(catalog(method))
        .module(
            "app:test",
            "import {open} from '@impasto/test-native';
             const abort = new AbortController();
             const reader = open('sample', abort.signal).getReader();
             globalThis.take = async () => (await reader.read()).value.data;
             globalThis.abort = async () => {
               abort.abort('stopped');
               try { await reader.read(); } catch (reason) { return reason; }
             };",
        )
        .build()
        .expect("build stream runtime");
    let ticket = runtime.start_module("app:test").expect("stream module");
    assert_eq!(
        runtime.take_module_result(ticket).expect("result"),
        Some(ModuleOutcome::Fulfilled)
    );
    assert!(runtime.counts().queued_bytes > 0);
    runtime.drive(0).expect("deliver stream");
    runtime
        .eval_script("take().then(data => globalThis.data = data)", "read.js")
        .expect("read");
    runtime.drive(1).expect("consume");
    runtime
        .eval_script(
            "if (data !== 'body') throw Error('wrong stream item');",
            "check.js",
        )
        .expect("result");
    assert_eq!(runtime.counts().queued_bytes, 0);
    assert!(
        cancellation
            .try_recv()
            .expect("stream token")
            .is_cancelled()
    );

    // An already-aborted signal must return an errored stream before the producer runs.
    let rejected = Method::stream("open", |_, _| panic!("aborted producer ran"));
    let mut runtime = Runtime::builder()
        .modules(catalog(rejected))
        .module(
            "app:test",
            "import {open} from '@impasto/test-native';
             try { await open(AbortSignal.abort(7)).getReader().read(); throw Error('resolved'); }
             catch (reason) { if (reason !== 7) throw reason; }",
        )
        .build()
        .expect("build aborted stream");
    let ticket = runtime
        .start_module("app:test")
        .expect("start aborted stream");
    runtime.drive(0).expect("finish aborted read");
    assert_eq!(
        runtime
            .take_module_result(ticket)
            .expect("aborted stream result"),
        Some(ModuleOutcome::Fulfilled)
    );
}

fn catalog(method: Method) -> ModuleCatalog {
    let module = NativeExtension {
        name: "TestNative",
        specifier: "@impasto/test-native",
        methods: vec![method],
        exports: Default::default(),
    };
    ModuleCatalog::new(ModuleRegistry::try_new(vec![module]).expect("native registry"))
        .expect("module catalog")
}

#[test]
fn awaiting_a_native_read_keeps_the_stream_alive_through_gc() {
    let (producer, receiver) = mpsc::channel();
    let method = Method::stream_with_exec("open", move |_, sink, _| {
        producer.send(sink).expect("receive native producer");
    });
    let mut runtime = Runtime::builder()
        .modules(catalog(method))
        .module(
            "app:test",
            "import {open} from '@impasto/test-native';
             async function read() {
               const reader = open().getReader();
               return (await reader.read()).value;
             }
             if (await read() !== 'ready') throw Error('missing native stream item');",
        )
        .build()
        .expect("build runtime");
    let ticket = runtime.start_module("app:test").expect("start native read");
    runtime.drive(0).expect("reach pending read");
    let mut sink = receiver.try_recv().expect("native producer was started");
    runtime.run_gc().expect("collect while read is pending");
    assert!(
        !sink.cancellation().is_cancelled(),
        "GC canceled a pending read"
    );
    sink.try_send(PluginValue::Str("ready".into()))
        .expect("complete pending read");
    runtime.drive(1).expect("deliver native item");
    assert_eq!(
        runtime.take_module_result(ticket).expect("module result"),
        Some(ModuleOutcome::Fulfilled)
    );
    runtime.run_gc().expect("collect after read settles");
    assert!(
        sink.cancellation().is_cancelled(),
        "settled read retained the stream"
    );
}

#[test]
fn web_primitive_semantics_reuse_vm_jobs_and_allow_injected_entropy() {
    struct FixedEntropy;
    impl EntropySource for FixedEntropy {
        fn fill(&self, bytes: &mut [u8]) -> std::io::Result<()> {
            bytes.fill(0x41);
            Ok(())
        }
    }

    let mut runtime = Runtime::builder()
        .entropy(Arc::new(FixedEntropy))
        .build()
        .expect("runtime");
    runtime.eval_script(
        r#"
        let order = [];
        Promise.resolve().then(() => order.push("promise"));
        queueMicrotask(() => order.push("microtask"));
        const bytes = new Uint8Array([1, 2, 3, 4]);
        if (crypto.getRandomValues(bytes.subarray(1, 3))[0] !== 65 || String(bytes) !== "1,65,65,4") {
            throw Error("getRandomValues changed the wrong view");
        }
        if (crypto.randomUUID() !== "41414141-4141-4141-8141-414141414141") throw Error("UUID");
        for (const [test, code] of [
            [() => btoa("\u0100"), "InvalidCharacterError"],
            [() => crypto.getRandomValues(new Float64Array(1)), "TypeMismatchError"],
            [() => crypto.getRandomValues(new Uint8Array(65537)), "QuotaExceededError"],
            [() => structuredClone({ f() {} }), "DataCloneError"],
            [() => structuredClone([Symbol()]), "DataCloneError"],
            [() => structuredClone(new Proxy([], {})), "DataCloneError"],
        ]) {
            try { test(); throw Error("expected DOMException"); }
            catch (error) { if (!(error instanceof DOMException) || error.name !== code) throw error; }
        }
        const view = new DataView(bytes.buffer, 1, 2);
        const regexp = /x/gi;
        regexp.lastIndex = 4;
        const value = { bytes, view, regexp, list: [, 1], error: new TypeError("bad") };
        value.self = value;
        value.map = new Map([[value, new Set([value])]]);
        value.error.cause = value;
        const copy = structuredClone(value, { transfer: [bytes.buffer] });
        if (bytes.buffer.byteLength !== 0 || copy.bytes.buffer !== copy.view.buffer ||
            copy.view.getUint8(0) !== 65 || copy.self !== copy || 0 in copy.list ||
            !copy.map.get(copy).has(copy) || copy.regexp.lastIndex !== 0 ||
            !(copy.error instanceof TypeError) || copy.error.cause !== copy) {
            throw Error("clone or transfer lost graph identity");
        }
        "#, "primitives.js").expect("native web APIs");
    runtime.drive(0).expect("drain jobs");
    runtime
        .eval_script(
            "if (order.join() !== 'promise,microtask') throw Error(order);",
            "jobs.js",
        )
        .expect("microtask ordering");
}

#[test]
fn readable_stream_runs_start_pull_and_cancel_on_the_vm() {
    let mut runtime = Runtime::builder().build().expect("runtime");
    runtime
        .eval_script(
            r#"
        let index = 0;
        let calls = [];
        let received;
        let stream = new ReadableStream({
          start(controller) { calls.push(controller.desiredSize); },
          pull(controller) {
            controller.enqueue({ index: ++index });
            if (index === 2) controller.close();
          },
        }, { highWaterMark: 0 });
        let reader = stream.getReader();
        async function read() {
          const first = await reader.read();
          const second = await reader.read();
          const last = await reader.read();
          received = [first.value.index, second.value.index, last.done, last.value];
        }
        read();
        let canceled;
        const channel = new ReadableStream({
          cancel(reason) { canceled = reason; return Promise.reject(reason); },
        });
        channel.cancel("cancelled").catch(reason => calls.push(reason));
    "#,
            "streams.js",
        )
        .expect("stream construction");
    runtime.drive(0).expect("drain streams");
    runtime
        .eval_script(
            r#"
        if (!received || String(received) !== "1,2,true," || canceled !== "cancelled" ||
            calls.join() !== "0,cancelled") throw Error("reader or underlying source");
    "#,
            "result.js",
        )
        .expect("stream result");
}

#[test]
fn signal_abort_unblocks_a_native_producer_waiting_for_credit() {
    use javascript::host::{MethodExec, PluginResult, Settle, StreamSendError, Work};

    struct Background;
    impl MethodExec for Background {
        fn worker(&self, settle: Settle, work: Work) {
            std::thread::spawn(move || settle.send(work()));
        }
        fn main_thread(&self, settle: Settle, work: Work) {
            settle.send(work());
        }
        fn now(&self, settle: Settle, result: PluginResult) {
            settle.send(result);
        }
    }

    let (waiting, wait) = mpsc::channel();
    let (ended, end) = mpsc::channel();
    let method = Method::stream("open", move |_, sink| {
        sink.send(PluginValue::F64(1.0), Duration::ZERO)
            .map_err(|error| error.to_string())?;
        waiting.send(()).map_err(|error| error.to_string())?;
        let result = sink.send(PluginValue::F64(2.0), Duration::from_secs(5));
        ended.send(result).map_err(|error| error.to_string())
    });
    let mut runtime = Runtime::builder()
        .executor(Arc::new(Background))
        .limits(javascript::runtime::RuntimeLimits {
            stream_events: 1,
            ..Default::default()
        })
        .modules(catalog(method))
        .module(
            "app:test",
            "import {open} from '@impasto/test-native';
             globalThis.controller = new AbortController();
             globalThis.reader = open(controller.signal).getReader();",
        )
        .build()
        .expect("background runtime");
    let ticket = runtime.start_module("app:test").expect("run producer");
    wait.recv_timeout(Duration::from_secs(2))
        .expect("first item enqueued");
    runtime
        .eval_script(
            "controller.abort('stop');
         reader.read().catch(reason => globalThis.rejected = reason);",
            "abort.js",
        )
        .expect("abort stream");
    assert_eq!(
        end.recv_timeout(Duration::from_secs(2))
            .expect("wake producer"),
        Err(StreamSendError::Closed)
    );
    assert_eq!(runtime.counts().requests, 0);
    runtime.drive(0).expect("settle");
    runtime
        .eval_script(
            "if (rejected !== 'stop') throw Error('lost abort reason');",
            "check.js",
        )
        .expect("abort reason");
    assert_eq!(
        runtime.take_module_result(ticket).expect("module result"),
        Some(ModuleOutcome::Fulfilled)
    );
    assert_eq!(runtime.counts().queued_bytes, 0);
}

#[test]
fn stream_with_exec_drives_async_sink_and_cancels_cleanly() {
    use std::future::Future;
    use std::pin::pin;
    use std::task::{Context, Poll, Wake, Waker};

    struct FlagWake(AtomicUsize);
    impl Wake for FlagWake {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    let (ready_tx, ready_rx) = mpsc::channel();
    let (step_tx, step_rx) = mpsc::channel();
    let step_rx = Arc::new(std::sync::Mutex::new(Some(step_rx)));
    let (done_tx, done_rx) = mpsc::channel();
    let method = Method::stream_with_exec("open", move |args, mut sink, _exec| {
        assert_eq!(args, [PluginValue::Str("async".into())]);
        let step_rx = step_rx.lock().unwrap().take().expect("one open");
        let ready_tx = ready_tx.clone();
        let done_tx = done_tx.clone();
        std::thread::spawn(move || {
            let wake = Arc::new(FlagWake(AtomicUsize::new(0)));
            let waker = Waker::from(Arc::clone(&wake));
            let mut cx = Context::from_waker(&waker);
            {
                let mut first = pin!(sink.send_async(PluginValue::F64(1.0)));
                assert!(matches!(first.as_mut().poll(&mut cx), Poll::Ready(Ok(()))));
            }
            {
                let mut second = pin!(sink.send_async(PluginValue::F64(2.0)));
                assert!(matches!(second.as_mut().poll(&mut cx), Poll::Pending));
                ready_tx.send(()).expect("signal pending");
                step_rx.recv().expect("consume first");
                assert_eq!(wake.0.load(Ordering::SeqCst), 1);
                assert!(matches!(second.as_mut().poll(&mut cx), Poll::Ready(Ok(()))));
            }
            let cancel = sink.cancellation();
            let cancel_wake = Arc::new(FlagWake(AtomicUsize::new(0)));
            let cw = Waker::from(Arc::clone(&cancel_wake));
            let mut ccx = Context::from_waker(&cw);
            {
                let mut cancel_fut = pin!(cancel.cancelled());
                assert!(cancel_fut.as_mut().poll(&mut ccx).is_pending());
                ready_tx.send(()).expect("signal cancel wait");
                step_rx.recv().expect("abort signal");
                assert_eq!(cancel_wake.0.load(Ordering::SeqCst), 1);
                assert!(cancel_fut.as_mut().poll(&mut ccx).is_ready());
            }
            sink.finish();
            let _ = done_tx.send(());
        });
    });
    let mut runtime = Runtime::builder()
        .limits(javascript::runtime::RuntimeLimits {
            stream_events: 1,
            ..Default::default()
        })
        .modules(catalog(method))
        .module(
            "app:test",
            "import {open} from '@impasto/test-native';
             globalThis.ctrl = new AbortController();
             globalThis.reader = open('async', ctrl.signal).getReader();",
        )
        .build()
        .expect("async stream runtime");
    let ticket = runtime.start_module("app:test").expect("start");
    ready_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("producer waiting on credit");

    runtime.drive(0).expect("deliver first");
    runtime
        .eval_script(
            "reader.read().then(part => { globalThis.v1 = part.value; });",
            "read1.js",
        )
        .expect("read1");
    runtime.drive(1).expect("consume first");
    step_tx.send(()).expect("wake second send");

    ready_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("producer waiting on cancel");
    runtime
        .eval_script("ctrl.abort('bye');", "abort.js")
        .expect("abort");
    step_tx.send(()).expect("check cancel");
    done_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("producer finished");
    runtime.drive(2).expect("settle");
    runtime
        .eval_script(
            "if (globalThis.v1 !== 1) throw Error('missing async value');",
            "check.js",
        )
        .expect("check");
    assert_eq!(
        runtime.take_module_result(ticket).expect("result"),
        Some(ModuleOutcome::Fulfilled)
    );
}

#[test]
fn dropping_a_cancel_wait_releases_its_task_before_the_scope_closes() {
    struct Task(AtomicUsize);
    impl std::task::Wake for Task {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    let token = CancelToken::new();
    let task = Arc::new(Task(AtomicUsize::new(0)));
    let waker = std::task::Waker::from(Arc::clone(&task));
    let mut context = std::task::Context::from_waker(&waker);
    for _ in 0..10 {
        {
            let wait = std::pin::pin!(token.cancelled());
            assert!(wait.poll(&mut context).is_pending());
        }
        assert_eq!(Arc::strong_count(&task), 2);
    }
    assert!(!token.is_cancelled());
}
