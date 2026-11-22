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
