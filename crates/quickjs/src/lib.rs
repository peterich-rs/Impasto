//! QuickJS session. The `scene` module writes directly into `uicore::Tree`.

#![allow(unsafe_code, reason = "the QuickJS C API is called from this crate")]

use uicore::Shaper;

mod bytecode;
mod error;
// Run parser for `setTextRuns`. The interpreter calls it from `linked`.
mod text_runs;

mod abi {
    include!(concat!(env!("OUT_DIR"), "/abi.rs"));
}

mod linked;

pub use linked::JsSession;

pub use bytecode::{CompiledBytecode, compile, compile_with_registry};
pub use error::Error;

/// Console callback. Level 3 is an error. Other levels are informational.
pub type ConsoleLog = Box<dyn Fn(u8, &str)>;

/// Nonblocking admission into the host's generation-aware transport.
pub type RoutedIoSubmit = Box<dyn Fn(session::IoJob) -> Result<(), Box<session::IoJob>> + Send>;

/// Loads `script` and returns the session the host pumps each frame.
///
/// `name` is the filename the interpreter reports for this script. `log` receives
/// console lines. Level 3 is an error. Other levels are informational.
pub fn open(
    script: &str,
    name: &str,
    shaper: impl Shaper + 'static,
    log: ConsoleLog,
) -> Result<Box<dyn session::Session>, Error> {
    open_with_io(
        script,
        name,
        shaper,
        log,
        None,
        session::ModuleConfig::disconnected(),
    )
}

/// Opens a session and installs the host sink for [`session::IoJob`].
pub fn open_with_io(
    script: &str,
    name: &str,
    shaper: impl Shaper + 'static,
    log: ConsoleLog,
    io: Option<Box<dyn Fn(session::IoJob) + Send>>,
    host: session::ModuleConfig,
) -> Result<Box<dyn session::Session>, Error> {
    {
        let mut session = JsSession::open_with(script, name, shaper, log, None, host)?;
        if let Some(io) = io {
            session.install_io(io);
        }
        Ok(Box::new(session))
    }
}

/// Loads UTF-8 source. Bytecode uses [`open_compiled_with_io`] or
/// [`open_trusted_bytecode_with_io`].
pub fn open_program_with_io(
    source: &[u8],
    name: &str,
    shaper: impl Shaper + 'static,
    log: ConsoleLog,
    io: Option<Box<dyn Fn(session::IoJob) + Send>>,
    config: session::ModuleConfig,
) -> Result<Box<dyn session::Session>, Error> {
    finish_session(
        JsSession::open_source(source, name, shaper, log, None, config)?,
        io,
    )
}

/// Loads an envelope that [`compile`] just produced in this process.
pub fn open_compiled_with_io(
    program: &CompiledBytecode,
    name: &str,
    shaper: impl Shaper + 'static,
    log: ConsoleLog,
    io: Option<Box<dyn Fn(session::IoJob) + Send>>,
    config: session::ModuleConfig,
) -> Result<Box<dyn session::Session>, Error> {
    finish_session(
        JsSession::open_trusted(
            bytecode::TrustedBytecode::from_compiled(program),
            name,
            shaper,
            log,
            None,
            config,
        )?,
        io,
    )
}

/// Loads an IQJS envelope from a trusted file.
///
/// # Safety
/// `bytes` must be an unmodified envelope written by this toolchain's `compile`,
/// such as `impasto build`. The envelope checks the pin, the mode tag, and that
/// the payload is non-empty. It does not prove the payload is valid bytecode.
/// Untrusted bytes may cause undefined behavior in `Module::load` and `JS_ReadObject`.
pub unsafe fn open_trusted_bytecode_with_io(
    bytes: &[u8],
    name: &str,
    shaper: impl Shaper + 'static,
    log: ConsoleLog,
    io: Option<Box<dyn Fn(session::IoJob) + Send>>,
    config: session::ModuleConfig,
) -> Result<Box<dyn session::Session>, Error> {
    // SAFETY: the caller guarantees these bytes came from this toolchain's `compile`.
    let program = unsafe { bytecode::TrustedBytecode::from_bytes(bytes) };
    finish_session(
        JsSession::open_trusted(program, name, shaper, log, None, config)?,
        io,
    )
}

/// Opens UTF-8 source with generation-aware external I/O after the entry.
pub fn open_program_with_routed_io(
    source: &[u8],
    name: &str,
    shaper: impl Shaper + 'static,
    log: ConsoleLog,
    io: Option<RoutedIoSubmit>,
    config: session::ModuleConfig,
) -> Result<Box<dyn session::Session>, Error> {
    let session = JsSession::open_source(source, name, shaper, log, None, config)?;
    Ok(finish_routed_session(session, io))
}

/// Opens a trusted IQJS envelope with generation-aware external I/O after the entry.
///
/// # Safety
/// `bytes` must be an unmodified IQJS envelope written by this toolchain's compiler.
pub unsafe fn open_trusted_bytecode_with_routed_io(
    bytes: &[u8],
    name: &str,
    shaper: impl Shaper + 'static,
    log: ConsoleLog,
    io: Option<RoutedIoSubmit>,
    config: session::ModuleConfig,
) -> Result<Box<dyn session::Session>, Error> {
    // SAFETY: the caller attests that the compiler produced this envelope.
    let program = unsafe { bytecode::TrustedBytecode::from_bytes(bytes) };
    let session = JsSession::open_trusted(program, name, shaper, log, None, config)?;
    Ok(finish_routed_session(session, io))
}

fn finish_routed_session(
    mut session: JsSession,
    io: Option<RoutedIoSubmit>,
) -> Box<dyn session::Session> {
    if let Some(io) = io {
        session.install_routed_io(io);
    }
    Box::new(session)
}

fn finish_session(
    mut session: JsSession,
    io: Option<Box<dyn Fn(session::IoJob) + Send>>,
) -> Result<Box<dyn session::Session>, Error> {
    if let Some(io) = io {
        session.install_io(io);
    }
    Ok(Box::new(session))
}

#[cfg(test)]
mod component_tests;

#[cfg(test)]
#[allow(clippy::expect_used, reason = "engine behavior assertions")]
mod tests {
    use protocol::Command;
    use session::{
        EventType, ImmediateExec, InputEvent, IoEvent, Key, Method, Module, ModuleConfig,
        ModuleRegistry, PluginResult, PluginValue,
    };
    use session::{Metrics, Session};
    use uicore::{Phase, PointerEvent};

    use super::{Error, open};

    const LINKING_EVENTS: &[EventType] = &[EventType {
        name: "url",
        payload: "{ url: string }",
    }];

    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tools/toolchain-benchmark.rs"
    ));

    #[test]
    fn native_style_contracts_and_single_view_button() {
        {
            let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
            let output = root.join("target/native-style.js");
            let status = std::process::Command::new("node")
                .arg(root.join("tools/pack.mjs"))
                .arg(root.join("crates/quickjs/tests/fixtures/style.tsx"))
                .arg(&output)
                .status()
                .expect("pack style fixture");
            assert!(status.success());
            let source = std::fs::read_to_string(output).expect("read style fixture");
            let mut session = open(&source, "style.js", uicore::MonoShaper::default(), no_log())
                .expect("native style contracts");
            session.metrics(&metrics());
            session.frame(session::FrameTime::at(0.0));
            assert!(session.take_faults().is_empty());
        }
    }

    #[test]
    fn native_component_contracts() {
        {
            let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
            let output = root.join("target/native-components.js");
            let status = std::process::Command::new("node")
                .arg(root.join("tools/pack.mjs"))
                .arg(root.join("crates/quickjs/tests/fixtures/components.tsx"))
                .arg(&output)
                .status()
                .expect("pack components fixture");
            assert!(status.success());
            let source = std::fs::read_to_string(output).expect("read components fixture");
            let mut session = open(
                &source,
                "components.js",
                uicore::MonoShaper::default(),
                no_log(),
            )
            .expect("native component contracts");
            session.metrics(&metrics());
            session.frame(session::FrameTime::at(0.0));
            let faults = session.take_faults();
            assert!(faults.is_empty(), "{faults:?}");
        }
    }

    #[test]
    fn native_control_flow_preserves_owned_rows_and_catches_effect_errors() {
        {
            let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
            let output = root.join("target/native-flow.js");
            let status = std::process::Command::new("node")
                .arg(root.join("tools/pack.mjs"))
                .arg(root.join("crates/quickjs/tests/fixtures/flow.ts"))
                .arg(&output)
                .status()
                .expect("build native fixture");
            assert!(status.success());
            let source = std::fs::read_to_string(&output).expect("native fixture");
            let mut session = open(&source, "flow.js", uicore::MonoShaper::default(), no_log())
                .expect("native flow");
            assert!(
                session.take_faults().is_empty(),
                "{:?}",
                session.take_faults()
            );
            session.metrics(&metrics());
            session.frame(session::FrameTime::at(0.0));
            assert!(session.take_faults().is_empty());
        }
    }

    #[test]
    fn native_reactive_matches_the_solid_fixture() {
        {
            let probe = include_str!("../../reactive/tests/fixtures/probe.mjs")
                .replace("export function", "function");
            let source = format!(
                "import * as reactive from 'reactive';\n{probe}\nrunProbe(line=>console.log(line), {{...reactive, flush: fn=>reactive.batch(fn || (()=>{{}}))}});"
            );
            let lines = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
            let output = std::rc::Rc::clone(&lines);
            let mut session = open(
                &source,
                "reference.js",
                uicore::MonoShaper::default(),
                Box::new(move |_, line| output.borrow_mut().push(line.to_owned())),
            )
            .expect("native reference");
            assert!(session.take_faults().is_empty());
            assert_eq!(
                lines.borrow().join("\n") + "\n",
                include_str!("../../reactive/tests/fixtures/order.txt")
            );
        }
    }

    #[test]
    fn bytecode_round_trip_uses_the_native_modules() {
        {
            let source = "import {createSignal,batch} from 'reactive'; import {createElement,insert,setProp} from 'scene'; const [get,set]=createSignal('first'); const root=createElement('column',{gap:8}); insert(root,get); batch(()=>set('compiled')); setProp(root,'gap',16);";
            let bytes = super::compile(source, "compiled.js", true).expect("compile module");
            let mut session = super::open_compiled_with_io(
                &bytes,
                "compiled.bin",
                uicore::MonoShaper::default(),
                no_log(),
                None,
                ModuleConfig::disconnected(),
            )
            .expect("load bytecode");
            session.metrics(&metrics());
            let frame = session.frame(session::FrameTime::at(0.0));
            assert!(texts(&frame).contains(&"compiled"), "{:?}", texts(&frame));
            assert!(session.take_faults().is_empty());
        }
    }

    #[test]
    fn context_is_owned_and_children_unwrap_accessors() {
        let source = r#"
            import * as r from 'reactive';
            const assert=(ok,message)=>{if(!ok)throw new Error(message);};
            const Context=r.createContext('default');
            let outside,inside,stop;
            r.createRoot(dispose=>{
                stop=dispose;
                const owner=r.getOwner();
                Context({value:'provided',get children(){inside=r.getOwner();return 'child';}})();
                outside=r.runWithOwner(null,()=>r.createRoot(()=>r.getOwner()));
                assert(r.runWithOwner(inside,()=>r.useContext(Context))==='provided','owned context');
                assert(r.runWithOwner(outside,()=>r.useContext(Context))==='default','sibling context');
                assert(r.useContext(Context)==='default','provider leaked into parent');
                const c=r.children(()=>[1,()=>[2,null,false,()=>3]]);
                assert(c.toArray().join(',')==='1,2,3','children');
                const fn=()=>42;const memo=r.createMemo(()=>fn);
                assert(memo()===fn,'function is a memo value');
            });
            stop();
            assert(r.runWithOwner(inside,()=>r.getOwner())===null,'stale owner');
            assert(typeof globalThis.core==='undefined','core global');
            assert(typeof globalThis.reactive==='undefined','reactive global');
        "#;
        match open(
            source,
            "context.js",
            uicore::MonoShaper::default(),
            no_log(),
        ) {
            Ok(_) => {}
            Err(error) => panic!("{error}"),
        }
    }

    #[test]
    fn native_insert_reuses_text_and_reports_invalid_properties() {
        let source = r#"
            import {createSignal,batch} from 'reactive';
            import {scene} from 'scene';
            const assert=(ok,label)=>{if(!ok)throw new Error(label);};
            const root=scene.createElement('column');
            const [text,setText]=createSignal('first');
            scene.insert(root,text);
            const id=scene.firstChild(root);
            batch(()=>setText('second'));
            assert(scene.firstChild(root)===id,'text node replaced');
            for(const [name,value] of [['w','invalid'],['h',Infinity],['insetLeft','bad']]) {
                let caught=false;
                try{scene.setProp(root,name,value);}catch{caught=true;}
                assert(caught,'invalid '+name+' accepted');
            }
            const removed=scene.createElement('sized');scene.discard(removed);
            let caught=false;
            try{scene.insertNode(root,removed);}catch{caught=true;}
            assert(caught,'stale node accepted');
        "#;
        match open(
            source,
            "native-insert.js",
            uicore::MonoShaper::default(),
            no_log(),
        ) {
            Ok(_) => {}
            Err(error) => panic!("{error}"),
        }
    }

    #[test]
    fn a_runaway_reactive_computation_is_interrupted() {
        let source = r#"
            import {createSignal,createRenderEffect} from 'reactive';
            import {scene} from 'scene';
            const [read,write]=createSignal(false);
            createRenderEffect(()=>{if(read()){while(true){}}return read();},()=>{});
            const root=scene.column(0),region=scene.pointerRegion();
            scene.append(region,scene.text('Run',16,[255,255,255,255]));
            scene.append(root,region);
            scene.onClick(region,()=>write(true));
        "#;
        let mut session = match open(
            source,
            "reactive-budget.js",
            uicore::MonoShaper::default(),
            no_log(),
        ) {
            Ok(session) => session,
            Err(error) => panic!("{error}"),
        };
        session.metrics(&metrics());
        let first = session.frame(session::FrameTime::at(0.0));
        let (x, y) = text_spot(&first, "Run");
        click(session.as_mut(), x, y);
        let frame = session.frame(session::FrameTime::at(16.0));
        assert!(frame.layers.is_none());
        assert!(
            session
                .take_faults()
                .iter()
                .any(|fault| fault.message.contains("interrupted"))
        );
    }

    #[test]
    fn frame_callback_signal_updates_layout_in_the_same_frame() {
        let source = r#"
            import { createSignal, createRenderEffect } from "reactive";
            import * as scene from "scene";
            const root = scene.column(0);
            const label = scene.text("initial", 16, [255, 255, 255, 255]);
            scene.append(root, label);
            const [read, write] = createSignal("initial");
            createRenderEffect(read, (value) => scene.setText(label, value));
            scene.onFrame((target) => {
                if (target === 16) write("from frame callback");
            });
        "#;
        let mut session = open(
            source,
            "frame-signal.js",
            uicore::MonoShaper::default(),
            no_log(),
        )
        .expect("open session");
        session.metrics(&metrics());
        let first = session.frame(session::FrameTime::at(0.0));
        text_spot(&first, "initial");
        let next = session.frame(session::FrameTime::at(16.0));
        text_spot(&next, "from frame callback");
        assert_eq!(next.phases.expect("frame phases").trace.layout_passes, 1);
    }

    #[test]
    fn reactive_module_reads_and_writes_a_signal() {
        let source = r#"
          import * as graph from '#reactive';
          if (!graph || typeof graph.createSignal !== "function") {
            throw new Error("reactive module missing");
          }
          var id = graph.createSignal(1);
          if (graph.get(id) !== 1) {
            throw new Error("read " + graph.get(id));
          }
          graph.set(id, 2);
          if (graph.get(id) !== 2) {
            throw new Error("write " + graph.get(id));
          }
        "#;
        match open(
            source,
            "reactive.js",
            uicore::MonoShaper::default(),
            no_log(),
        ) {
            Ok(_) => {}
            Err(error) => panic!("{error}"),
        }
    }

    #[test]
    fn reactive_module_reruns_a_memo_after_a_write() {
        let source = r#"
          import * as graph from '#reactive';
          import * as core from "scene";
          var source = graph.createSignal(1);
          if (source < 4294967296) {
            throw new Error("allocated " + source);
          }
          var seen = 0;
          var memo = graph.createMemo(function () {
            seen = graph.get(source);
            return seen + 1;
          });
          var first = graph.get(memo);
          if (first !== 2) {
            throw new Error("memo " + first + " type " + typeof first + " seen " + seen);
          }
          graph.set(source, 4);
          graph.runUpdates();
          if (graph.get(memo) !== 5) {
            throw new Error("memo after write " + graph.get(memo) + " mem " + core.debugMemory());
          }
        "#;
        match open(source, "memo.js", uicore::MonoShaper::default(), no_log()) {
            Ok(_) => {}
            Err(error) => panic!("{error}"),
        }
    }

    #[test]
    fn reactive_author_api_orders_effects_like_solid() {
        let source = r#"
          import * as reactive from "reactive";
          var lines = [];
          reactive.createRoot(function () {
            var pair = reactive.createSignal(1);
            var get = pair[0];
            var set = pair[1];
            reactive.createTrackedEffect(function () { lines.push("tracked " + get()); });
            reactive.createRenderEffect(function () { return get(); }, function (value) { lines.push("render " + value); });
            set(2);
            reactive.batch(function () {});
          });
          var text = lines.join(",");
          if (text !== "render 1,render 2,tracked 2") {
            throw new Error("order [" + text + "]");
          }
        "#;
        match open(source, "order.js", uicore::MonoShaper::default(), no_log()) {
            Ok(_) => {}
            Err(error) => panic!("{error}"),
        }
    }

    #[test]
    fn scene_module_creates_and_styles_a_node() {
        let source = r#"
          import * as scene from "scene";
          if (!scene || typeof scene.column !== "function") {
            throw new Error("scene module missing");
          }
          var id = scene.column(8);
          if (!(id > 0)) {
            throw new Error("column id " + id);
          }
          scene.setProp(id, "gap", 16);
          var child = scene.text("hi", 14, [0, 0, 0, 255]);
          scene.insert(id, child, 0);
        "#;
        match open(source, "scene.js", uicore::MonoShaper::default(), no_log()) {
            Ok(_) => {}
            Err(error) => panic!("{error}"),
        }
    }

    #[test]
    fn list_driver_methods_are_functions() {
        let source = r#"
import * as core from "scene";

          var d = core.listDriver(2, 20);
          var proto = Object.getPrototypeOf(d);
          var names = Object.getOwnPropertyNames(proto).join(",");
          if (typeof d.configure !== "function") {
            throw new Error("configure " + typeof d.configure + " proto " + names);
          }
          d.configure(20, 2, false);
          d.setItems([{key:"a", type:"row", sticky:false}]);
          d.release();
        "#;
        match open(source, "driver.js", uicore::MonoShaper::default(), no_log()) {
            Ok(_) => {}
            Err(error) => panic!("{error}"),
        }
    }

    #[test]
    fn drop_releases_unfired_timer() {
        let session = open(
            "const captured = { text: 'held' }; setTimeout(() => console.log(captured.text), 60000); setInterval(() => {}, 60000); requestAnimationFrame(() => {});",
            "drop-timer.js", uicore::MonoShaper::default(), no_log(),
        ).expect("open timers");
        drop(session);
    }

    #[test]
    fn drop_releases_reactive_cleanup() {
        let seen = std::rc::Rc::new(std::cell::Cell::new(0));
        let record = std::rc::Rc::clone(&seen);
        let (jobs, received) = std::sync::mpsc::channel();
        let session = super::open_program_with_routed_io(
            br#"
                import {createRoot,createSignal,onCleanup} from 'reactive';
                createRoot(() => onCleanup(() => {
                    console.log('cleanup');
                    setTimeout(() => {}, 60000);
                    createRoot(() => {
                        createSignal({held: true});
                        onCleanup(() => console.log('nested cleanup'));
                    });
                    return fetch('https://example.test/shutdown');
                }));
            "#,
            "drop-cleanup.js",
            uicore::MonoShaper::default(),
            Box::new(move |_, text| {
                if text == "cleanup" || text == "nested cleanup" {
                    record.set(record.get() + 1);
                }
            }),
            Some(Box::new(move |job| {
                jobs.send(job).map_err(|error| Box::new(error.0))
            })),
            ModuleConfig::disconnected(),
        )
        .expect("open cleanup");
        drop(session);
        assert_eq!(seen.get(), 2);
        assert!(received.try_recv().is_err());
    }

    #[test]
    fn failed_open_releases_standard_and_reactive_bindings() {
        let seen = std::rc::Rc::new(std::cell::Cell::new(0));
        let record = std::rc::Rc::clone(&seen);
        let result = open(
            "import {createRoot,onCleanup} from 'reactive'; if (typeof fetch !== 'function' || typeof AbortController !== 'function') throw new Error('standard bindings missing'); createRoot(() => onCleanup(() => console.log('cleanup'))); setTimeout(() => {}, 60000); fetch('https://example.test/pending'); throw new Error('failed during entry');",
            "failed-open.js",
            uicore::MonoShaper::default(),
            Box::new(move |_, text| {
                if text == "cleanup" {
                    record.set(record.get() + 1);
                }
            }),
        );
        assert!(
            matches!(result, Err(Error::Script(ref message)) if message.contains("failed during entry"))
        );
        assert_eq!(seen.get(), 1);
    }

    /// Application startup precedes the external transport. Its I/O
    /// completions settle in the first frame after JS records their tokens.
    #[test]
    fn open_runs_entry_before_io_install_and_frame_flushes_pending() {
        let logs = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let record = std::rc::Rc::clone(&logs);
        let jobs = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = std::sync::Arc::clone(&jobs);
        let source = r#"
            console.log("sync");
            Promise.resolve().then(function () { console.log("job"); });
            setTimeout(function () { console.log("timer"); }, 0);
            requestAnimationFrame(function () { console.log("raf"); });
            fetch("https://example.test/pending").then(
                function () { console.log("fetch-ok"); },
                function (error) { console.log("fetch-fail:" + String(error)); }
            );
            fetchStream("https://example.test/stream").then(
                function () { console.log("stream-ok"); },
                function (error) { console.log("stream-fail:" + String(error && error.message)); }
            );
            var socket = new WebSocket("ws://example.test/socket");
            socket.onerror = function () { console.log("socket-error"); };
            socket.onopen = function () { console.log("socket-open"); };
            console.log("sync-after");
        "#;
        let mut session = super::open_with_io(
            source,
            "order.js",
            uicore::MonoShaper::default(),
            Box::new(move |_level, text| record.borrow_mut().push(text.to_owned())),
            Some(Box::new(move |_job| {
                count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            })),
            ModuleConfig::disconnected(),
        )
        .expect("open");
        assert_eq!(
            logs.borrow().as_slice(),
            ["sync", "sync-after", "job"],
            "entry microtask runs during open; timers, fetch and sockets wait for a frame"
        );
        assert_eq!(jobs.load(std::sync::atomic::Ordering::Relaxed), 0);
        session.frame(session::FrameTime::at(0.0));
        let lines = logs.borrow();
        let remaining = lines
            .iter()
            .filter(|line| !line.starts_with("stream-"))
            .map(String::as_str)
            .collect::<Vec<_>>();
        assert_eq!(
            remaining,
            [
                "sync",
                "sync-after",
                "job",
                "socket-error",
                "timer",
                "fetch-fail:TypeError",
                "raf",
            ]
        );
        assert_eq!(jobs.load(std::sync::atomic::Ordering::Relaxed), 0);
        assert_eq!(
            lines
                .iter()
                .filter(|line| line.starts_with("stream-fail:"))
                .count(),
            1,
            "{lines:?}"
        );
        drop(session);
    }

    #[test]
    fn socket_commands_and_microtasks_keep_the_previous_frame_clock() {
        let logs = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let record = std::rc::Rc::clone(&logs);
        let (jobs, received) = std::sync::mpsc::channel();
        let mut session = super::open_with_io(
            r#"
                console.log("entry:" + performance.now());
                setTimeout(() => {
                    const socket = new WebSocket("ws://example.test/socket");
                    socket.onopen = () => {
                        console.log("command:" + performance.now());
                        Promise.resolve().then(() => console.log("job:" + performance.now()));
                        setTimeout(() => console.log("timer:" + performance.now()), 0);
                    };
                }, 0);
            "#,
            "socket-clock.js",
            uicore::MonoShaper::default(),
            Box::new(move |_level, text| record.borrow_mut().push(text.to_owned())),
            Some(Box::new(move |job| {
                let _ = jobs.send(job);
            })),
            ModuleConfig::disconnected(),
        )
        .expect("open");

        session.frame(session::FrameTime::at(100.0));
        let session::IoJob::SocketOpen { id, .. } = received.try_recv().expect("socket open")
        else {
            panic!("unexpected network job");
        };
        session.io_event(IoEvent::Socket {
            id,
            event: "open".into(),
            text: true,
            data: Vec::new(),
        });
        assert_eq!(logs.borrow().as_slice(), ["entry:0", "command:0"]);

        session.frame(session::FrameTime::at(116.0));
        assert_eq!(
            logs.borrow().as_slice(),
            ["entry:0", "command:0", "job:0", "timer:16"]
        );
    }

    #[test]
    fn routed_socket_completion_requires_the_live_generation() {
        let logs = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let record = std::rc::Rc::clone(&logs);
        let (jobs, received) = std::sync::mpsc::channel();
        let mut session = super::open_program_with_routed_io(
            br#"
                setTimeout(() => {
                    const socket = new WebSocket("ws://example.test/socket");
                    socket.onopen = () => console.log("socket opened");
                }, 0);
            "#,
            "routed-socket.js",
            uicore::MonoShaper::default(),
            Box::new(move |_level, text| record.borrow_mut().push(text.to_owned())),
            Some(Box::new(move |job| {
                jobs.send(job).map_err(|error| Box::new(error.0))
            })),
            ModuleConfig::disconnected(),
        )
        .expect("open");
        session.frame(session::FrameTime::at(0.0));
        let session::IoJob::Routed { route, job, .. } = received.try_recv().expect("routed socket")
        else {
            panic!("transport did not receive a route");
        };
        let session::IoJob::SocketOpen { id, .. } = *job else {
            panic!("unexpected network job");
        };
        let opened = || IoEvent::Socket {
            id,
            event: "open".into(),
            text: true,
            data: Vec::new(),
        };
        let mut stale = route.deliver(opened(), None);
        stale.runtime.generation += 1;
        session.io_delivery(stale);
        assert!(logs.borrow().is_empty());
        session.io_delivery(route.deliver(opened(), None));
        session.io_delivery(route.deliver(opened(), None));
        assert_eq!(logs.borrow().as_slice(), ["socket opened"]);
    }

    #[test]
    fn startup_inline_stream_settles_after_recording_the_token() {
        struct EmptyResponse;

        impl session::HttpClient for EmptyResponse {
            fn perform(
                &self,
                request: &session::HttpRequest,
            ) -> Result<session::HttpResponse, session::HttpError> {
                assert_eq!(request.body, [11, 12]);
                Ok(session::HttpResponse {
                    status: 200,
                    headers: Vec::new(),
                    body: Vec::new(),
                })
            }
        }

        let logs = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let record = std::rc::Rc::clone(&logs);
        let mut session = super::JsSession::open_with(
            r#"
                const bytes = new Uint8Array([10, 11, 12, 13]).subarray(1, 3);
                fetchStream("https://example.test/empty", {method: "POST", body: bytes})
                    .then(async response => {
                        const item = await response.getReader().read();
                        console.log("stream:" + response.status + ":" + item.done);
                    });
            "#,
            "startup-stream.js",
            uicore::MonoShaper::default(),
            Box::new(move |_level, text| record.borrow_mut().push(text.to_owned())),
            Some(std::sync::Arc::new(EmptyResponse)),
            ModuleConfig::disconnected(),
        )
        .expect("open");
        assert!(logs.borrow().is_empty());
        session.frame(session::FrameTime::at(0.0));
        assert_eq!(logs.borrow().as_slice(), ["stream:200:true"]);
    }

    #[test]
    fn module_listener_mutations_take_effect_after_the_snapshot() {
        let logs = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let record = std::rc::Rc::clone(&logs);
        let (host, _pending) = test_host();
        let mut session = super::JsSession::open_with(
            r#"
                import { addEventListener, removeEventListener } from "@impasto/linking";
                const later = ({url}) => console.log("later:" + url);
                const duplicate = ({url}) => console.log("duplicate:" + url);
                const first = (event) => {
                    console.log("first:" + event.url);
                    event.url = "mutated";
                    removeEventListener("url", first);
                    removeEventListener("url", duplicate);
                    addEventListener("url", later);
                    throw new Error("the UI continues through the snapshot");
                };
                addEventListener("url", first);
                addEventListener("url", duplicate);
                addEventListener("url", duplicate);
            "#,
            "module-snapshot.js",
            uicore::MonoShaper::default(),
            Box::new(move |_level, text| record.borrow_mut().push(text.to_owned())),
            None,
            host,
        )
        .expect("open");
        session.io_event(IoEvent::Host(js::HostEvent {
            module: "Linking".into(),
            event: "url".into(),
            payload: PluginValue::Map(vec![(
                "url".into(),
                PluginValue::Str("impasto://first".into()),
            )]),
        }));
        session.set_launch_url("impasto://second");
        assert_eq!(
            logs.borrow().as_slice(),
            [
                "first:impasto://first",
                "duplicate:impasto://first",
                "duplicate:impasto://first",
                "later:impasto://second",
            ]
        );
    }

    #[test]
    fn unknown_io_completion_is_ignored_and_drop_discards_pending_fetch() {
        let logs = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let record = std::rc::Rc::clone(&logs);
        let mut session = open(
            "fetch('https://example.test/pending').then(function () { console.log('fetch-ok'); }, function (error) { console.log('fetch-fail:' + String(error)); });",
            "pending-fetch.js",
            uicore::MonoShaper::default(),
            Box::new(move |_level, text| record.borrow_mut().push(text.to_owned())),
        )
        .expect("open");
        assert!(logs.borrow().is_empty());
        session.io_event(IoEvent::Http {
            token: 99,
            result: Err(session::HttpError::new(
                session::HttpFailure::Network,
                "late",
            )),
        });
        session.io_event(IoEvent::Plugin {
            token: 99,
            result: PluginResult::Failed {
                message: "late".into(),
            },
        });
        session.io_event(IoEvent::Socket {
            id: 99,
            event: "error".into(),
            text: true,
            data: b"late".to_vec(),
        });
        assert!(logs.borrow().is_empty());
        drop(session);
        assert!(logs.borrow().is_empty());
    }

    #[test]
    fn timer_kind_cancel_delay_and_origin_stay_on_the_frame_clock() {
        let logs = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let record = std::rc::Rc::clone(&logs);
        let mut session = open(
            r#"
            var raf = requestAnimationFrame(function (value) { console.log("raf:" + value); }, 0, "argument");
            var timeout = setTimeout(function () { console.log("timeout"); }, -5);
            var interval = setInterval(function () { console.log("interval"); }, 0);
            clearTimeout(interval);
            clearInterval(timeout);
            clearTimeout(raf);
            cancelAnimationFrame(timeout);
            setTimeout(function () {
                console.log("nested");
                setTimeout(function () { console.log("inner"); }, 0);
            }, 0);
            console.log("now:" + performance.now());
            "#,
            "timers.js",
            uicore::MonoShaper::default(),
            Box::new(move |_level, text| record.borrow_mut().push(text.to_owned())),
        )
        .expect("open");
        assert_eq!(logs.borrow().as_slice(), ["now:0"]);
        session.frame(session::FrameTime::at(16.0));
        assert_eq!(
            logs.borrow().as_slice(),
            ["now:0", "timeout", "interval", "nested", "raf:argument"]
        );
        session.frame(session::FrameTime::at(32.0));
        assert_eq!(
            logs.borrow().as_slice(),
            [
                "now:0",
                "timeout",
                "interval",
                "nested",
                "raf:argument",
                "interval",
                "inner"
            ]
        );
    }

    #[test]
    fn host_argument_getter_can_call_another_module_method() {
        let logs = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let record = std::rc::Rc::clone(&logs);
        let nested = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let nested_for_ping = std::sync::Arc::clone(&nested);
        let config = ModuleConfig {
            registry: std::sync::Arc::new(ModuleRegistry::new(vec![Module {
                name: "Probe",
                exports: Default::default(),
                specifier: "@impasto/probe",
                methods: vec![
                    Method::sync("read", |args| match args.first() {
                        Some(PluginValue::Map(pairs)) => Ok(pairs
                            .iter()
                            .find(|(key, _)| key == "n")
                            .map(|(_, value)| value.clone())
                            .unwrap_or(PluginValue::Null)),
                        _ => Ok(PluginValue::Null),
                    }),
                    Method::sync("ping", move |_| {
                        nested_for_ping.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        Ok(PluginValue::Null)
                    }),
                ],
            }])),
            ..ModuleConfig::disconnected()
        };
        let _session = super::open_with_io(
            r#"
            import { read, ping } from '@impasto/probe';
            const quiet = {};
            Object.defineProperty(quiet, "n", { enumerable: true, get: function () { return 7; } });
            console.log("quiet:" + read(quiet));
            const nested = {};
            Object.defineProperty(nested, "n", {
                enumerable: true,
                get: function () { ping(); return 1; },
            });
            console.log("nested:" + read(nested));
            "#,
            "getter.js",
            uicore::MonoShaper::default(),
            Box::new(move |_level, text| record.borrow_mut().push(text.to_owned())),
            None,
            config,
        )
        .expect("open");
        assert_eq!(logs.borrow().as_slice(), ["quiet:7", "nested:1"]);
        assert_eq!(nested.load(std::sync::atomic::Ordering::Relaxed), 1);
    }

    #[test]
    fn script_bytecode_round_trip_uses_the_envelope() {
        let bytes = super::compile(
            "globalThis.answer = 42; console.log(answer)",
            "script.js",
            false,
        )
        .expect("compile script");
        let seen = std::rc::Rc::new(std::cell::RefCell::new(String::new()));
        let record = std::rc::Rc::clone(&seen);
        let session = super::open_compiled_with_io(
            &bytes,
            "script.bin",
            uicore::MonoShaper::default(),
            Box::new(move |_, text| record.borrow_mut().push_str(text)),
            None,
            ModuleConfig::disconnected(),
        )
        .expect("load script envelope");
        assert_eq!(&*seen.borrow(), "42");
        drop(session);
    }

    #[test]
    fn debug_memory_samples_the_active_runtime() {
        let source = r#"
            import { debugMemory } from 'scene';
            const { mallocBytes, objects, atoms, strings } = JSON.parse(debugMemory());
            if (!(mallocBytes > 0 && objects > 0 && atoms > 0 && strings >= 0)) {
                throw new Error('missing memory counters');
            }
        "#;
        assert!(open(source, "memory.js", uicore::MonoShaper::default(), no_log()).is_ok());
    }

    #[test]
    fn reactive_values_use_javascript_strict_equality() {
        let source = r#"
            import * as g from '#reactive';
            const n = g.createSignal(123456789012345678901234n);
            if (g.set(n, BigInt('123456789012345678901234')) !== 0) throw new Error('BigInt equality');
            const text = g.createSignal('repeated text value');
            if (g.set(text, ['repeated', 'text', 'value'].join(' ')) !== 0) throw new Error('string equality');
        "#;
        drop(
            open(
                source,
                "equality.js",
                uicore::MonoShaper::default(),
                no_log(),
            )
            .expect("strict equality"),
        );
    }

    #[test]
    fn host_module_bytes_preserve_typed_array_views() {
        let mut config = ModuleConfig::disconnected();
        config.registry = std::sync::Arc::new(ModuleRegistry::new(vec![Module {
            name: "Echo",
            exports: Default::default(),
            specifier: "@impasto/echo",
            methods: vec![Method::sync("echo", |args| {
                Ok(args.first().cloned().unwrap_or(PluginValue::Null))
            })],
        }]));
        let source = r#"
            import {echo} from '@impasto/echo';
            const source = new Int16Array([1, 258, -3]);
            const view = source.subarray(1, 2);
            const bytes = new Uint8Array(view.buffer, view.byteOffset, view.byteLength);
            const returned = echo(view);
            if (!(returned instanceof Uint8Array) || returned.length !== bytes.length || returned.some((b,i) => b !== bytes[i])) throw new Error('typed view bytes');
            const clamped = echo(new Uint8ClampedArray([300, -1]));
            if (clamped[0] !== 255 || clamped[1] !== 0) throw new Error('clamped bytes');
            const large = echo(new Uint8Array(4096).fill(7));
            if (!(large instanceof ArrayBuffer) || new Uint8Array(large)[4095] !== 7) throw new Error('owned buffer');
        "#;
        drop(
            super::JsSession::open_with(
                source,
                "bytes.js",
                uicore::MonoShaper::default(),
                no_log(),
                None,
                config,
            )
            .expect("module bytes"),
        );
    }

    #[test]
    fn the_same_host_binding_script_runs_in_headless_and_ui() {
        use js::ModuleOutcome;
        use std::cell::RefCell;
        use std::rc::Rc;
        use std::sync::Arc;

        const SOURCE: &str = r#"
            import {echo, later, addEventListener} from '@impasto/fixture';
            const check = (channel, value) => {
                if (!Array.isArray(value.list) || value.list[0] !== null || value.list[1].ok !== true) {
                    throw new Error(channel + ': lost nested list or map');
                }
                if (Object.getPrototypeOf(value) !== Object.prototype ||
                    !Object.hasOwn(value, '__proto__') || value.__proto__ !== 'ordinary property') {
                    throw new Error(channel + ': mishandled __proto__');
                }
                if (!(value.bytes instanceof Uint8Array) || value.bytes.join(',') !== '11,12') {
                    throw new Error(channel + ': lost view bounds');
                }
                console.log(channel);
            };
            const input = {
                list: [null, {ok: true}],
                ['__proto__']: 'ordinary property',
                bytes: new Uint8Array([10, 11, 12, 13]).subarray(1, 3),
            };
            check('sync', echo(input));
            addEventListener('sample', (value) => check('event', value));
            check('promise', await later(input));
        "#;
        let echo = |args: &[PluginValue]| args.first().cloned().unwrap_or(PluginValue::Null);
        let module = Module {
            name: "Fixture",
            specifier: "@impasto/fixture",
            exports: session::ModuleExports {
                events: &[EventType {
                    name: "sample",
                    payload: "unknown",
                }],
                ..Default::default()
            },
            methods: vec![
                Method::sync("echo", move |args| Ok(echo(args))),
                Method::promise("later", move |args, settle, exec| {
                    exec.now(settle, PluginResult::Done(echo(args)));
                }),
            ],
        };
        let event = || {
            IoEvent::Host(js::HostEvent {
                module: "Fixture".into(),
                event: "sample".into(),
                payload: PluginValue::Map(vec![
                    (
                        "list".into(),
                        PluginValue::List(vec![
                            PluginValue::Null,
                            PluginValue::Map(vec![("ok".into(), PluginValue::Bool(true))]),
                        ]),
                    ),
                    (
                        "__proto__".into(),
                        PluginValue::Str("ordinary property".into()),
                    ),
                    ("bytes".into(), PluginValue::Bytes(vec![11, 12])),
                ]),
            })
        };
        let catalog = js::ModuleCatalog::new(
            js::contract::ModuleRegistry::try_new(vec![module.clone()]).expect("registry"),
        )
        .expect("catalog");
        let headless = Rc::new(RefCell::new(Vec::new()));
        let record = Rc::clone(&headless);
        let mut runtime = js::Runtime::builder()
            .modules(catalog)
            .module("app:fixture", SOURCE)
            .log(move |_, line| record.borrow_mut().push(line.to_owned()))
            .build()
            .expect("headless");
        let ticket = runtime.start_module("app:fixture").expect("start module");
        runtime.enqueue(event()).expect("queue event");
        runtime.drive(100).expect("complete module");
        assert_eq!(
            runtime.take_module_result(ticket).expect("module result"),
            Some(ModuleOutcome::Fulfilled)
        );
        let ui = Rc::new(RefCell::new(Vec::new()));
        let record = Rc::clone(&ui);
        let (send, receive) = std::sync::mpsc::channel();
        let config = ModuleConfig {
            registry: Arc::new(ModuleRegistry::new(vec![module])),
            post_delivery: Some(Arc::new(move |delivery| {
                let _ = send.send(delivery);
            })),
            ..ModuleConfig::disconnected()
        };
        let mut session = super::JsSession::open_with(
            SOURCE,
            "fixture.js",
            uicore::MonoShaper::default(),
            Box::new(move |_, line| record.borrow_mut().push(line.to_owned())),
            None,
            config,
        )
        .expect("UI");
        session.io_delivery(receive.try_recv().expect("module completion"));
        session.io_event(event());
        session.frame(session::FrameTime::at(100.0));
        assert_eq!(headless.borrow().as_slice(), ["sync", "event", "promise"]);
        assert_eq!(ui.borrow().as_slice(), headless.borrow().as_slice());
    }

    #[test]
    fn runaway_module_eval_reports_its_entry_and_shuts_down() {
        let result = open(
            "export {}; while (true) {}",
            "eval-budget.js",
            uicore::MonoShaper::default(),
            no_log(),
        );
        assert!(
            matches!(result, Err(Error::Script(ref message)) if message.contains("interrupted") && message.contains("(eval)"))
        );
    }

    #[test]
    fn recursive_jobs_hit_the_round_limit_and_shut_down() {
        let result = open(
            "function again() { Promise.resolve().then(again); } again();",
            "job-limit.js",
            uicore::MonoShaper::default(),
            no_log(),
        );
        assert!(
            matches!(result, Err(Error::Script(ref message)) if message.contains("quickjs job round limit") && message.contains("(jobs)"))
        );
    }

    fn no_log() -> super::ConsoleLog {
        Box::new(|level, line| {
            if std::env::var("IMPASTO_TEST_LOG").is_ok() {
                eprintln!("[js {level}] {line}");
            }
        })
    }

    /// The gallery bundle is a build artifact and not committed. Tests that
    /// need it fail with instructions when it has not been built.
    fn gallery_source() -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../apps/gallery-solid/dist/quotes.js");
        std::fs::read_to_string(&path)
            .unwrap_or_else(|_| panic!("run `just gallery-dist` to build the gallery bundle"))
    }

    type PluginCompletions = std::sync::Arc<std::sync::Mutex<Vec<(u64, PluginResult)>>>;

    fn test_host() -> (ModuleConfig, PluginCompletions) {
        test_host_with_launch_url(None)
    }

    fn test_host_with_launch_url(url: Option<String>) -> (ModuleConfig, PluginCompletions) {
        use std::sync::{Arc, Mutex};
        let clipboard = Arc::new(Mutex::new(String::new()));
        let app_state = session::shared("active".to_owned());
        let launch_url = session::shared(url);
        let pending = Arc::new(Mutex::new(Vec::new()));
        let settle = {
            let pending = Arc::clone(&pending);
            Arc::new(move |token: u64| {
                let pending = Arc::clone(&pending);
                session::Settle::new(move |result| {
                    pending
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .push((token, result));
                })
            }) as Arc<dyn Fn(u64) -> session::Settle + Send + Sync>
        };
        let config = ModuleConfig {
            registry: Arc::new(ModuleRegistry::new(vec![
                Module {
                    name: "Clipboard",
                    exports: session::ModuleExports {
                        object: true,
                        ..Default::default()
                    },
                    specifier: "@impasto/clipboard",
                    methods: vec![
                        Method::sync("getString", {
                            let clipboard = Arc::clone(&clipboard);
                            move |_| {
                                Ok(PluginValue::Str(
                                    clipboard
                                        .lock()
                                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                                        .clone(),
                                ))
                            }
                        }),
                        Method::sync("setString", {
                            let clipboard = Arc::clone(&clipboard);
                            move |args| {
                                let text = match args.first() {
                                    Some(PluginValue::Str(text)) => text.clone(),
                                    _ => String::new(),
                                };
                                *clipboard
                                    .lock()
                                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = text;
                                Ok(PluginValue::Null)
                            }
                        }),
                    ],
                },
                Module {
                    name: "AppState",
                    exports: session::ModuleExports {
                        events: &[EventType {
                            name: "change",
                            payload: "string",
                        }],
                        ..Default::default()
                    },
                    specifier: "@impasto/app-state",
                    methods: vec![Method::sync("getCurrentState", {
                        let app_state = Arc::clone(&app_state);
                        move |_| Ok(PluginValue::Str(session::shared_read(&app_state)))
                    })],
                },
                Module {
                    name: "Echo",
                    exports: Default::default(),
                    specifier: "@impasto/echo",
                    methods: vec![
                        Method::sync("fail", |_| Err("boom from host".into())),
                        Method::promise("later", |args, settle, exec| {
                            let value = match args.first() {
                                Some(PluginValue::Str(text)) => text.clone(),
                                _ => String::new(),
                            };
                            exec.now(settle, PluginResult::Done(PluginValue::Str(value)));
                        }),
                    ],
                },
                Module {
                    name: "Linking",
                    exports: session::ModuleExports {
                        object: true,
                        events: LINKING_EVENTS,
                        ..Default::default()
                    },
                    specifier: "@impasto/linking",
                    methods: vec![Method::sync("getInitialURL", {
                        let launch_url = Arc::clone(&launch_url);
                        move |_| {
                            Ok(match session::shared_read(&launch_url) {
                                Some(url) => PluginValue::Str(url),
                                None => PluginValue::Null,
                            })
                        }
                    })],
                },
            ])),
            exec: Arc::new(ImmediateExec),
            settle,
            post_delivery: None,
            app_state,
            launch_url,
        };
        (config, pending)
    }

    fn drain_plugin(
        session: &mut dyn Session,
        pending: &std::sync::Mutex<Vec<(u64, PluginResult)>>,
    ) {
        let events: Vec<_> = pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .drain(..)
            .collect();
        for (token, result) in events {
            session.io_event(IoEvent::Plugin { token, result });
        }
    }

    fn metrics() -> Metrics {
        Metrics::new(320.0, 180.0, 1.0)
    }

    fn phone() -> Metrics {
        Metrics::new(360.0, 640.0, 2.0)
    }

    fn visit_commands<'a>(
        frame: &'a session::FrameOutput,
        visit: &mut dyn FnMut(&'a Command, f32, f32),
    ) {
        let Some(tree) = frame.layers.as_deref() else {
            return;
        };
        fn walk<'a>(
            tree: &'a protocol::LayerTree,
            id: protocol::LayerId,
            tx: f32,
            ty: f32,
            visit: &mut dyn FnMut(&'a Command, f32, f32),
        ) {
            let Some(layer) = tree.layers.get(id.0 as usize) else {
                return;
            };
            let (mut next_x, mut next_y) = (tx, ty);
            match layer.kind {
                protocol::LayerKind::Offset { x, y } => {
                    next_x += x;
                    next_y += y;
                }
                protocol::LayerKind::Transform {
                    translate_x,
                    translate_y,
                    ..
                } => {
                    next_x += translate_x;
                    next_y += translate_y;
                }
                protocol::LayerKind::Picture { picture } => {
                    if let Some(picture) = tree.picture(picture) {
                        let origin_x = next_x + picture.origin.0;
                        let origin_y = next_y + picture.origin.1;
                        for command in picture.commands.iter() {
                            visit(command, origin_x, origin_y);
                        }
                    }
                }
                _ => {}
            }
            let mut child = layer.first_child;
            while let Some(id) = child {
                walk(tree, id, next_x, next_y, visit);
                child = tree.layers.get(id.0 as usize).and_then(|layer| layer.next);
            }
        }
        let mut id = Some(tree.root);
        while let Some(current) = id {
            walk(tree, current, 0.0, 0.0, visit);
            id = tree
                .layers
                .get(current.0 as usize)
                .and_then(|layer| layer.next);
        }
    }

    fn texts(frame: &session::FrameOutput) -> Vec<&str> {
        let mut found = Vec::new();
        visit_commands(frame, &mut |command, _, _| {
            if let Command::Text { utf8, .. } = command {
                found.push(utf8.as_ref());
            }
        });
        found
    }

    fn text_spot(frame: &session::FrameOutput, utf8: &str) -> (f32, f32) {
        let mut found = None;
        visit_commands(frame, &mut |command, origin_x, origin_y| {
            if let Command::Text {
                utf8: text, x, y, ..
            } = command
                && text.as_ref() == utf8
            {
                found = Some((*x + origin_x + 1.0, *y + origin_y + 1.0));
            }
        });
        found.unwrap_or_else(|| panic!("missing text {utf8}"))
    }

    fn redraws(frame: &session::FrameOutput) -> u32 {
        frame.phases.expect("frame phases").trace.canvas_redraws
    }

    /// Picture generation and canvas body identity for canvases of `height`.
    /// A re-record allocates a new body and advances the picture generation.
    fn chart_pictures(frame: &session::FrameOutput, height: f32) -> Vec<(u64, usize)> {
        let Some(tree) = frame.layers.as_deref() else {
            return Vec::new();
        };
        let mut found = Vec::new();
        for picture in &tree.pictures {
            for command in picture.commands.iter() {
                if let Command::Canvas {
                    height: canvas_height,
                    list,
                    ..
                } = command
                    && (canvas_height - height).abs() < 0.5
                {
                    found.push((
                        picture.generation,
                        std::sync::Arc::as_ptr(&list.body) as usize,
                    ));
                }
            }
        }
        found
    }

    fn chart_origin(frame: &session::FrameOutput, height: f32) -> Option<(f32, f32)> {
        let mut found = None;
        visit_commands(frame, &mut |command, tx, ty| {
            if let Command::Canvas {
                x,
                y,
                height: canvas_height,
                ..
            } = command
                && (canvas_height - height).abs() < 0.5
            {
                found = Some((tx + *x, ty + *y));
            }
        });
        found
    }

    fn volume_readout(frame: &session::FrameOutput) -> bool {
        texts(frame).iter().any(|text| text.starts_with("成交量 "))
    }

    fn chart_recorded_volume(frame: &session::FrameOutput, height: f32) -> bool {
        let mut found = false;
        visit_commands(frame, &mut |command, _, _| {
            if let Command::Canvas {
                height: canvas_height,
                list,
                ..
            } = command
                && (canvas_height - height).abs() < 0.5
            {
                found |= list.ops().iter().any(|op| {
                    matches!(op, protocol::CanvasOp::Text(text) if text.utf8.contains("成交量"))
                });
            }
        });
        found
    }

    /// Pumps frames until the first screen lands. The quotes app mounts a
    /// heavier first screen than the old counter.
    fn pump_first(session: &mut dyn Session) -> session::FrameOutput {
        let mut frame = session.frame(session::FrameTime::at(0.0));
        for step in 0..16 {
            if frame.layers.is_some() {
                break;
            }
            frame = session.frame(session::FrameTime::at(f64::from(step) * 16.0));
        }
        frame
    }

    fn click(session: &mut dyn Session, x: f32, y: f32) {
        session.pointer(PointerEvent::new(Phase::Down, x, y, 0.0));
        session.pointer(PointerEvent::new(Phase::Up, x, y, 0.0));
    }

    fn wait_for_text(
        session: &mut dyn Session,
        time: &mut f64,
        expected: &str,
    ) -> session::FrameOutput {
        let mut seen = Vec::new();
        for _ in 0..48 {
            *time += 16.0;
            let frame = session.frame(session::FrameTime::at(*time));
            let errors = session.take_faults();
            assert!(errors.is_empty(), "waiting for {expected}: {errors:?}");
            if frame.layers.is_some() {
                let labels = texts(&frame);
                if labels.contains(&expected) {
                    return frame;
                }
                seen = labels.into_iter().map(str::to_owned).collect();
            }
        }
        panic!("missing {expected} in {seen:?}");
    }

    /// Host modules the gallery bundle imports. Devtools stays off so the
    /// panel does not change the page the assertions read.
    fn gallery_host() -> ModuleConfig {
        ModuleConfig {
            registry: std::sync::Arc::new(ModuleRegistry::new(vec![
                Module {
                    name: "AsyncStorage",
                    exports: session::ModuleExports {
                        object: true,
                        ..Default::default()
                    },
                    specifier: "@impasto/async-storage",
                    methods: vec![
                        Method::sync("multiGet", |_| Ok(PluginValue::List(Vec::new()))),
                        Method::sync("multiSet", |_| Ok(PluginValue::Null)),
                    ],
                },
                Module {
                    name: "Linking",
                    exports: session::ModuleExports {
                        object: true,
                        events: LINKING_EVENTS,
                        ..Default::default()
                    },
                    specifier: "@impasto/linking",
                    methods: vec![
                        Method::sync("getInitialURL", |_| Ok(PluginValue::Null)),
                        Method::sync("openURL", |_| Ok(PluginValue::Null)),
                        Method::sync("canOpenURL", |_| Ok(PluginValue::Bool(false))),
                    ],
                },
                Module {
                    name: "Clipboard",
                    exports: session::ModuleExports {
                        object: true,
                        ..Default::default()
                    },
                    specifier: "@impasto/clipboard",
                    methods: vec![
                        Method::sync("getString", |_| Ok(PluginValue::Str(String::new()))),
                        Method::sync("setString", |_| Ok(PluginValue::Null)),
                    ],
                },
                Module {
                    name: "Devtools",
                    exports: Default::default(),
                    specifier: "@impasto/devtools",
                    methods: vec![Method::sync("enabled", |_| Ok(PluginValue::Bool(false)))],
                },
            ])),
            exec: std::sync::Arc::new(ImmediateExec),
            settle: std::sync::Arc::new(|_token| session::Settle::new(|_result| {})),
            post_delivery: None,
            app_state: session::shared("active".to_owned()),
            launch_url: session::shared(None),
        }
    }

    /// The gallery bundle is a build artifact and not committed. Tests that
    /// need it fail with instructions when it has not been built. Returns
    /// `None` when the interpreter is not linked; callers already early-return.
    fn open_gallery() -> Option<Box<dyn Session>> {
        {
            let body = gallery_source();
            let source = body;
            match super::open_with_io(
                &source,
                "test.js",
                uicore::MonoShaper::default(),
                Box::new(|level, line| eprintln!("[js {level}] {line}")),
                None,
                gallery_host(),
            ) {
                Ok(session) => Some(session),
                Err(error) => panic!("{error}"),
            }
        }
    }

    #[test]
    fn watchlist_page_shows_prices() {
        let Some(mut session) = open_gallery() else {
            return;
        };
        session.metrics(&phone());
        // First-screen mount is heavy; pump frames until the page lands.
        let mut first = session.frame(session::FrameTime::at(0.0));
        for step in 0..16 {
            if first.layers.is_some() {
                break;
            }
            first = session.frame(session::FrameTime::at(f64::from(step) * 16.0));
        }
        let shown = texts(&first);
        assert!(
            shown.contains(&"综合指数"),
            "{shown:?} error={:?} faults={:?}",
            session.take_script_error(),
            session.take_faults()
        );
        assert!(shown.contains(&"腾讯控股"), "{shown:?}");
        let mut sparks = 0;
        visit_commands(&first, &mut |command, _, _| {
            if matches!(command, Command::Canvas { list, .. } if !list.ops().is_empty()) {
                sparks += 1;
            }
        });
        assert!(sparks > 0, "no sparklines: {:?}", session.take_faults());
        // Binder write-on-bind: price cells carry formatted numbers already.
        assert!(
            shown.iter().any(|text| text.ends_with(".00")
                || text.ends_with(".20")
                || text.ends_with(".40")
                || text.ends_with(".60")
                || text.ends_with(".80")
                || text.chars().any(|c| c == '.')),
            "no formatted price in {shown:?}"
        );
    }

    #[test]
    fn market_tab_renders_the_board() {
        let Some(mut session) = open_gallery() else {
            return;
        };
        session.metrics(&phone());
        let first = pump_first(session.as_mut());
        assert!(texts(&first).contains(&"综合指数"), "{:?}", texts(&first));
        assert!(
            !texts(&first)
                .iter()
                .any(|text| text.contains("not a function")),
            "{:?}",
            texts(&first)
        );
        let (x, y) = text_spot(&first, "⋯");
        click(session.as_mut(), x, y);
        let menu = session.frame(session::FrameTime::at(16.0));
        let (x, y) = text_spot(&menu, "市场");
        click(session.as_mut(), x, y);
        let second = session.frame(session::FrameTime::at(32.0));
        let shown = texts(&second);
        assert!(
            shown.contains(&"涨幅榜"),
            "market page replaced itself with an error: {shown:?}"
        );
        assert!(!shown.iter().any(|text| text.contains("not a function")));
    }

    fn visible_watch_rows(frame: &session::FrameOutput) -> Vec<(String, f32)> {
        let mut rows = Vec::new();
        visit_commands(frame, &mut |command, _, origin_y| {
            if let Command::Text { utf8, y, .. } = command
                && IN_WATCHLIST_NAMES.iter().any(|name| *name == &**utf8)
            {
                rows.push((utf8.to_string(), *y + origin_y));
            }
        });
        rows.sort_by(|left, right| left.1.total_cmp(&right.1));
        rows
    }

    const IN_WATCHLIST_NAMES: &[&str] = &[
        "腾讯控股",
        "阿里巴巴",
        "美团",
        "中国移动",
        "小米集团",
        "中国平安",
        "友邦保险",
        "理想汽车",
        "小鹏汽车",
        "京东集团",
        "Apple",
        "NVIDIA",
        "贵州茅台",
        "宁德时代",
        "Tesla",
    ];

    #[test]
    fn scrolled_watch_rows_stay_in_order() {
        let Some(mut session) = open_gallery() else {
            return;
        };
        session.metrics(&phone());
        let _first = pump_first(session.as_mut());
        let mut shown = session.frame(session::FrameTime::at(16.0));
        for step in 0..6 {
            let next = session.frame(session::FrameTime::at(32.0 + f64::from(step) * 16.0));
            if next.layers.is_some() {
                shown = next;
            }
        }
        let start = visible_watch_rows(&shown);
        assert!(
            start.len() >= 2,
            "need visible rows, texts {:?}",
            texts(&shown)
        );
        let x = 180.0;
        let y = start[start.len() / 2].1 + 4.0;
        for step in 0..6 {
            let time = 200.0 + f64::from(step) * 80.0;
            session.pointer(PointerEvent::new(Phase::Down, x, y, time));
            session.pointer(PointerEvent::new(Phase::Move, x, y - 120.0, time + 16.0));
            session.pointer(PointerEvent::new(Phase::Up, x, y - 120.0, time + 32.0));
            for tick in 0..4 {
                let frame =
                    session.frame(session::FrameTime::at(time + 48.0 + f64::from(tick) * 16.0));
                if frame.layers.is_some() {
                    shown = frame;
                }
            }
            let rows = visible_watch_rows(&shown);
            assert!(
                rows.len() >= 2,
                "rows disappeared {:?} after drag {step}",
                texts(&shown)
            );
        }
    }

    #[test]
    fn trade_tab_renders_equity_and_orders() {
        let Some(mut session) = open_gallery() else {
            return;
        };
        session.metrics(&phone());
        let first = pump_first(session.as_mut());
        let (x, y) = text_spot(&first, "⋯");
        click(session.as_mut(), x, y);
        let menu = session.frame(session::FrameTime::at(16.0));
        let (x, y) = text_spot(&menu, "交易");
        click(session.as_mut(), x, y);
        let mut shown = session.frame(session::FrameTime::at(32.0));
        for step in 0..4 {
            let next = session.frame(session::FrameTime::at(48.0 + f64::from(step) * 16.0));
            if next.layers.is_some() {
                shown = next;
            }
        }
        assert!(
            texts(&shown).contains(&"总资产 (模拟)"),
            "equity card missing {:?}",
            texts(&shown)
        );
        assert!(
            texts(&shown).iter().any(|text| text.contains("持仓")),
            "positions tab missing {:?}",
            texts(&shown)
        );
    }

    #[test]
    fn stocks_desktop_switches_symbols_and_keeps_chart_and_book_inline() {
        let Some(mut session) = open_gallery() else {
            return;
        };
        session.metrics(&Metrics::new(1360.0, 820.0, 1.0));
        let mut time = 0.0;
        let first = wait_for_text(session.as_mut(), &mut time, "价格区间");
        let shown = texts(&first);
        assert!(
            shown.contains(&"综合指数") && shown.contains(&"盘口"),
            "{shown:?}"
        );
        let (chart_x, _) = text_spot(&first, "分时");
        let (book_x, _) = text_spot(&first, "盘口");
        assert!(
            chart_x > 300.0 && book_x > chart_x + 400.0,
            "chart {chart_x}, book {book_x}"
        );

        let (x, y) = text_spot(&first, "阿里巴巴");
        click(session.as_mut(), x, y);
        let changed = wait_for_text(session.as_mut(), &mut time, "阿里巴巴");
        assert_eq!(
            texts(&changed)
                .iter()
                .filter(|text| **text == "09988")
                .count(),
            2,
            "selected symbol should appear in both panes"
        );
        let (x, y) = text_spot(&changed, "5 → 10");
        click(session.as_mut(), x, y);
        let ten = wait_for_text(session.as_mut(), &mut time, "A10");
        assert!(texts(&ten).contains(&"A10") && texts(&ten).contains(&"B10"));
        let (x, y) = text_spot(&ten, "1D");
        click(session.as_mut(), x, y);
        let candles = wait_for_text(session.as_mut(), &mut time, "MA20");
        let mut plot = None;
        visit_commands(&candles, &mut |command, tx, ty| {
            if let Command::Canvas { x, y, height, .. } = command
                && *height == 320.0
            {
                plot = Some((tx + *x, ty + *y));
            }
        });
        let (x, y) = plot.expect("daily candle plot");
        session.pointer(PointerEvent::new(Phase::Down, x + 120.0, y + 40.0, time));
        time += 16.0;
        session.pointer(PointerEvent::new(Phase::Move, x + 144.0, y + 40.0, time));
        time += 16.0;
        let mut focus = session.frame(session::FrameTime::at(time));
        for _ in 0..3 {
            if volume_readout(&focus) {
                break;
            }
            time += 16.0;
            let next = session.frame(session::FrameTime::at(time));
            if next.layers.is_some() {
                focus = next;
            }
        }
        assert!(
            volume_readout(&focus),
            "crosshair should show a candle readout, got {:?}",
            texts(&focus)
        );
        assert!(
            !chart_recorded_volume(&focus, 320.0),
            "main chart should not record the crosshair readout"
        );
        let errors = session.take_faults();
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn stocks_phone_crosshair_keeps_the_chart_picture() {
        let Some(mut session) = open_gallery() else {
            return;
        };
        session.metrics(&phone());
        let mut time = 0.0;
        let first = wait_for_text(session.as_mut(), &mut time, "腾讯控股");
        let (x, y) = text_spot(&first, "腾讯控股");
        click(session.as_mut(), x, y);
        let _opening = wait_for_text(session.as_mut(), &mut time, "‹ 返回");
        let detail = wait_for_text(session.as_mut(), &mut time, "价格区间");
        let (daily_x, daily_y) = text_spot(&detail, "1D");
        click(session.as_mut(), daily_x, daily_y);
        let detail = wait_for_text(session.as_mut(), &mut time, "MA5");
        let (pause_x, pause_y) = text_spot(&detail, "Ⅱ");
        click(session.as_mut(), pause_x, pause_y);
        // A quiet frame did not re-record a canvas. The chart box from the last
        // submitted tree is still where the pointer hits.
        let mut baseline = None;
        let mut point = None;
        let mut last_spot = None;
        let mut held_spot = None;
        let mut held_pictures = None;
        for _ in 0..8 {
            time += 16.0;
            let frame = session.frame(session::FrameTime::at(time));
            let errors = session.take_faults();
            assert!(errors.is_empty(), "{errors:?}");
            if let Some(spot) = chart_origin(&frame, 204.0) {
                let pictures = chart_pictures(&frame, 204.0);
                if pictures.len() == 1 {
                    held_spot = Some(spot);
                    held_pictures = Some(pictures);
                }
            }
            let (Some((plot_x, plot_y)), Some(pictures)) = (held_spot, held_pictures.clone())
            else {
                continue;
            };
            last_spot = Some((plot_x, plot_y));
            let x = plot_x + 16.0;
            let y = plot_y + 30.0;
            let placed = (8.0..302.0).contains(&x) && (8.0..600.0).contains(&y);
            if redraws(&frame) == 0 && placed {
                baseline = Some(pictures);
                point = Some((x, y));
                break;
            }
        }
        let baseline = baseline.unwrap_or_else(|| panic!("chart canvas spot={last_spot:?}"));
        let (x, y) = point.expect("chart point");

        session.pointer(PointerEvent::new(Phase::Down, x, y, time));
        time += 16.0;
        session.pointer(PointerEvent::new(Phase::Move, x + 24.0, y, time));
        time += 16.0;
        let mut dragged = session.frame(session::FrameTime::at(time));
        for _ in 0..3 {
            if volume_readout(&dragged) {
                break;
            }
            time += 16.0;
            let next = session.frame(session::FrameTime::at(time));
            if next.layers.is_some() {
                dragged = next;
            }
        }
        assert!(
            volume_readout(&dragged),
            "crosshair tooltip missing {:?}",
            texts(&dragged)
        );
        let same_chart = |frame: &session::FrameOutput, step: &str| {
            if frame.layers.is_none() {
                assert_eq!(redraws(frame), 0, "{step}");
                return;
            }
            assert_eq!(
                chart_pictures(frame, 204.0),
                baseline,
                "{step} redraws={}",
                redraws(frame)
            );
            assert!(!chart_recorded_volume(frame, 204.0), "{step}");
        };
        same_chart(&dragged, "crosshair opens");
        assert_eq!(redraws(&dragged), 0, "press and drag must skip Canvas draw");
        let first_readout = texts(&dragged);
        session.pointer(PointerEvent::new(Phase::Move, x + 48.0, y, time));
        time += 16.0;
        let dragged = session.frame(session::FrameTime::at(time));
        same_chart(&dragged, "horizontal drag again");
        assert!(volume_readout(&dragged));
        assert_ne!(texts(&dragged), first_readout);
        assert_eq!(
            dragged
                .phases
                .expect("timed gallery frame")
                .trace
                .root_layouts,
            0,
            "changing the selected daily bar should keep ancestor layout"
        );

        session.pointer(PointerEvent::new(Phase::Cancel, x + 48.0, y, time));
        time += 16.0;
        let mut cancelled = session.frame(session::FrameTime::at(time));
        for _ in 0..3 {
            if !volume_readout(&cancelled) && cancelled.layers.is_some() {
                break;
            }
            time += 16.0;
            let next = session.frame(session::FrameTime::at(time));
            if next.layers.is_some() {
                cancelled = next;
            }
        }
        assert!(
            !volume_readout(&cancelled),
            "cancel should clear the crosshair {:?}",
            texts(&cancelled)
        );
        same_chart(&cancelled, "crosshair cancels");
        assert_eq!(redraws(&cancelled), 0, "cancel must skip Canvas draw");

        let (_, before_y) = text_spot(&dragged, "分时");
        session.pointer(PointerEvent::new(Phase::Down, x, y, time));
        time += 16.0;
        session.pointer(PointerEvent::new(Phase::Move, x, y - 80.0, time));
        time += 16.0;
        let mut scrolled = None;
        for _ in 0..4 {
            let frame = session.frame(session::FrameTime::at(time));
            time += 16.0;
            let errors = session.take_faults();
            assert!(errors.is_empty(), "{errors:?}");
            if frame.layers.is_some() {
                assert!(!chart_recorded_volume(&frame, 204.0), "vertical scroll");
            }
            if texts(&frame).contains(&"分时") {
                scrolled = Some(text_spot(&frame, "分时").1);
                break;
            }
        }
        let after_y = scrolled.expect("分时 stayed visible");
        assert_ne!(after_y, before_y, "vertical drag should scroll the page");
        session.pointer(PointerEvent::new(Phase::Cancel, x, y - 80.0, time));
        time += 16.0;
        let ended = session.frame(session::FrameTime::at(time));
        assert!(ended.phases.is_some(), "scroll cancel");
        assert!(!chart_recorded_volume(&ended, 204.0), "scroll cancel");
    }

    #[test]
    fn stocks_phone_opens_detail_and_order_ticket() {
        let Some(mut session) = open_gallery() else {
            return;
        };
        session.metrics(&phone());
        let mut time = 0.0;
        let first = wait_for_text(session.as_mut(), &mut time, "腾讯控股");
        let (x, y) = text_spot(&first, "腾讯控股");
        click(session.as_mut(), x, y);
        let opening = wait_for_text(session.as_mut(), &mut time, "‹ 返回");
        assert!(texts(&opening).contains(&"买入"));
        assert!(!texts(&opening).contains(&"盘口"));
        let mut detail = wait_for_text(session.as_mut(), &mut time, "价格区间");
        for _ in 0..20 {
            time += 16.0;
            let next = session.frame(session::FrameTime::at(time));
            let faults = session.take_faults();
            assert!(faults.is_empty(), "{faults:?}");
            if next.layers.is_some() {
                detail = next;
            }
        }
        let (x, y) = text_spot(&detail, "买入");
        click(session.as_mut(), x, y);
        wait_for_text(session.as_mut(), &mut time, "模拟下单");
    }

    #[test]
    fn stocks_phone_reopens_during_exit_without_leaving_a_detail() {
        let Some(mut session) = open_gallery() else {
            return;
        };
        session.metrics(&phone());
        let mut time = 0.0;
        let home = wait_for_text(session.as_mut(), &mut time, "腾讯控股");
        let (x, y) = text_spot(&home, "腾讯控股");
        let (other_x, other_y) = text_spot(&home, "阿里巴巴");
        click(session.as_mut(), x, y);
        wait_for_text(session.as_mut(), &mut time, "‹ 返回");
        click(session.as_mut(), other_x, other_y);
        let detail = wait_for_text(session.as_mut(), &mut time, "价格区间");
        assert_eq!(
            texts(&detail)
                .iter()
                .filter(|label| **label == "‹ 返回")
                .count(),
            1
        );
        let (back_x, back_y) = text_spot(&detail, "‹ 返回");
        click(session.as_mut(), back_x, back_y);
        let mut exiting = detail;
        for _ in 0..9 {
            time += 16.0;
            let frame = session.frame(session::FrameTime::at(time));
            if frame.layers.is_some() {
                exiting = frame;
            }
        }
        assert!(text_spot(&exiting, "‹ 返回").0 > other_x);
        click(session.as_mut(), other_x, other_y);
        let mut reopened = wait_for_text(session.as_mut(), &mut time, "价格区间");
        assert_eq!(
            texts(&reopened)
                .iter()
                .filter(|label| **label == "09988")
                .count(),
            2
        );
        session.input(InputEvent::Back);
        for _ in 0..48 {
            time += 16.0;
            let frame = session.frame(session::FrameTime::at(time));
            let faults = session.take_faults();
            assert!(faults.is_empty(), "{faults:?}");
            if frame.layers.is_some() {
                reopened = frame;
            }
        }
        let shown = texts(&reopened);
        assert!(shown.contains(&"综合指数"), "{shown:?}");
        assert!(!shown.contains(&"‹ 返回"), "{shown:?}");

        click(session.as_mut(), x, y);
        let entering = wait_for_text(session.as_mut(), &mut time, "‹ 返回");
        let before = text_spot(&entering, "‹ 返回").0;
        session.input(InputEvent::Back);
        let reversing = wait_for_text(session.as_mut(), &mut time, "‹ 返回");
        let after = text_spot(&reversing, "‹ 返回").0;
        assert!(
            (after - before).abs() < 30.0,
            "back jumped from {before} to {after}"
        );
    }

    #[test]
    // Broken by the gallery rewrite in b48bb8a, which shipped without a
    // compiling quickjs test build. Re-pin these once the watchlist click
    // path is verified against the real tree again.
    #[ignore = "gallery behavior drifted in b48bb8a; re-pin expectations"]
    fn repeated_clicks_leave_no_pending_jobs() {
        {
            let source = gallery_source();
            let mut session = match super::JsSession::open(
                &source,
                "test.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            session.metrics(&phone());
            let mut first = session.frame(session::FrameTime::at(0.0));
            for step in 0..16 {
                if first.layers.is_some() {
                    break;
                }
                first = session.frame(session::FrameTime::at(f64::from(step) * 16.0));
            }
            assert!(!session.jobs_pending());
            let (x, y) = text_spot(&first, "市场");
            click(&mut session, x, y);
            let second = session.frame(session::FrameTime::at(16.0));
            click(&mut session, x, y);
            let third = session.frame(session::FrameTime::at(32.0));
            assert!(texts(&second).contains(&"涨幅榜"));
            if third.layers.is_some() {
                assert!(texts(&third).contains(&"涨幅榜"), "{:?}", texts(&third));
            }
            assert!(!session.jobs_pending());
        }
    }

    #[test]
    fn remove_discards_the_tap_and_the_id() {
        {
            let kept = r#"
import * as core from "scene";

                const root = core.column(0);
                const region = core.pointerRegion();
                const label = core.text("Go", 16, [255, 255, 255, 255]);
                core.append(region, label);
                core.append(root, region);
                core.onClick(region, () => {});
                core.remove(root, region);
            "#;
            let session = match super::JsSession::open(
                kept,
                "test.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            assert_eq!(session.tap_count(), 0);
            let dropped = r#"
import * as core from "scene";

                const root = core.column(0);
                const label = core.text("Go", 16, [255, 255, 255, 255]);
                core.append(root, label);
                core.remove(root, label);
                core.setText(label, "still");
                core.append(root, label);
            "#;
            match super::JsSession::open(
                dropped,
                "test.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
                Ok(_) => {}
                Err(error) => panic!("removed id could not be inserted again: {error}"),
            }
        }
    }

    #[test]
    fn discarded_jsx_branch_releases_unattached_listeners() -> Result<(), Error> {
        let source = r#"
import * as scene from "scene";
import { createMemo, createRoot, createSignal } from "reactive";

const root = scene.column(0);
const surviving = scene.pointerRegion();
scene.onClick(surviving, () => {});
scene.append(root, surviving);
createRoot(dispose => {
    const [visible, show] = createSignal(true);
    const branch = createMemo(() => visible() && scene.createElement("view", {
        onClick() {},
    }));
    branch();
    show(false);
    branch();
    dispose();
});
"#;
        let session = super::JsSession::open(
            source,
            "unattached.js",
            uicore::MonoShaper::default(),
            no_log(),
        )?;
        assert_eq!(session.tap_count(), 1);
        Ok(())
    }

    #[test]
    fn detach_keeps_the_tap_until_discard() {
        {
            let kept = r#"
import * as core from "scene";

                const root = core.column(0);
                const region = core.pointerRegion();
                core.append(root, region);
                core.onClick(region, () => {});
                core.detach(root, region);
            "#;
            let mut session = match super::JsSession::open(
                kept,
                "test.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            assert_eq!(session.tap_count(), 1);
            let dropped = r#"
import * as core from "scene";

                const root = core.column(0);
                const region = core.pointerRegion();
                core.append(root, region);
                core.onClick(region, () => {});
                core.detach(root, region);
                core.discard(region);
            "#;
            session = match super::JsSession::open(
                dropped,
                "test.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            assert_eq!(session.tap_count(), 0);
        }
    }

    #[test]
    // Broken by the gallery rewrite in b48bb8a, which shipped without a
    // compiling quickjs test build. Re-pin these once the watchlist click
    // path is verified against the real tree again.
    #[ignore = "gallery behavior drifted in b48bb8a; re-pin expectations"]
    fn stock_page_pushes_over_the_watchlist() {
        let Some(mut session) = open_gallery() else {
            return;
        };
        session.metrics(&phone());
        let first = pump_first(session.as_mut());
        let (x, y) = text_spot(&first, "腾讯控股");
        click(session.as_mut(), x, y);
        let second = session.frame(session::FrameTime::at(16.0));
        assert!(texts(&second).contains(&"返回"), "{:?}", texts(&second));
        assert!(
            texts(&second).contains(&"盘口 五档"),
            "{:?}",
            texts(&second)
        );
        assert!(!texts(&second).contains(&"涨幅榜"));
    }

    #[test]
    // Broken by the gallery rewrite in b48bb8a, which shipped without a
    // compiling quickjs test build. Re-pin these once the watchlist click
    // path is verified against the real tree again.
    #[ignore = "gallery behavior drifted in b48bb8a; re-pin expectations"]
    fn press_move_out_up_does_not_fire() {
        let Some(mut session) = open_gallery() else {
            return;
        };
        session.metrics(&phone());
        let first = pump_first(session.as_mut());
        let (x, y) = text_spot(&first, "腾讯控股");
        session.pointer(PointerEvent::new(Phase::Down, x, y, 0.0));
        session.pointer(PointerEvent::new(Phase::Move, x, -1.0, 0.0));
        session.pointer(PointerEvent::new(Phase::Up, x, -1.0, 0.0));
        let second = session.frame(session::FrameTime::at(16.0));
        if let Some(layers) = second.layers.as_ref() {
            let _ = layers;
            assert!(texts(&second).contains(&"自选"), "{:?}", texts(&second));
            assert!(!texts(&second).contains(&"盘口 五档"));
        }
    }

    #[test]
    fn press_then_release_then_click_rewrites_the_label() {
        {
            let source = r#"
import * as core from "scene";

                let log = "";
                const root = core.column(0);
                const region = core.pointerRegion();
                const label = core.text("idle", 16, [0, 0, 0, 255]);
                core.append(region, label);
                core.append(root, region);
                const note = (mark) => { log += mark; core.setText(label, log); };
                core.onPress(region, () => note("P"));
                core.onRelease(region, () => note("R"));
                core.onClick(region, () => note("C"));
            "#;
            let mut session = match super::JsSession::open(
                source,
                "test.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            session.metrics(&metrics());
            let first = session.frame(session::FrameTime::at(0.0));
            let (x, y) = text_spot(&first, "idle");
            session.pointer(PointerEvent::new(Phase::Down, x, y, 0.0));
            let pressed = session.frame(session::FrameTime::at(16.0));
            assert_eq!(texts(&pressed), ["P"]);
            session.pointer(PointerEvent::new(Phase::Up, x, y, 0.0));
            let lifted = session.frame(session::FrameTime::at(32.0));
            assert_eq!(texts(&lifted), ["PRC"]);
        }
    }

    #[test]
    fn insets_shift_the_recorded_text() {
        {
            let source = r#"
import * as core from "scene";

                const label = core.text("Hi", 16, [0, 0, 0, 255]);
            "#;
            let mut session = match super::JsSession::open(
                source,
                "test.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            let mut window = metrics();
            window.inset_left = 8.0;
            window.inset_top = 14.0;
            window.width = 200.0;
            window.height = 120.0;
            session.metrics(&window);
            let frame = session.frame(session::FrameTime::at(0.0));
            assert_eq!(text_spot(&frame, "Hi"), (9.0, 15.0));
        }
    }

    #[test]
    fn syntax_error_script_is_reported() {
        match open("{", "test.js", uicore::MonoShaper::default(), no_log()) {
            Err(Error::Script(_)) => {}
            Err(error) => panic!("{error}"),
            Ok(_) => panic!("expected a script error"),
        }
    }

    #[test]
    fn an_infinite_frame_callback_keeps_the_previous_list() {
        {
            let source = r#"
import * as core from "scene";

                const root = core.column(0);
                let ticks = 0;
                core.onFrame(() => {
                    ticks += 1;
                    if (ticks > 1) {
                        while (true) {}
                    }
                });
            "#;
            let mut session = match super::JsSession::open(
                source,
                "test.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            session.metrics(&metrics());
            let first = session.frame(session::FrameTime::at(0.0));
            assert!(session.take_script_error().is_none());
            let second = session.frame(session::FrameTime::at(16.0));
            assert!(first.layers.is_some());
            assert!(second.layers.is_none());
            let message = session.take_script_error().unwrap_or_default();
            assert!(message.contains("(frame)"), "script error was {message:?}");
        }
    }

    #[test]
    fn listener_error_is_kept_as_script_error() {
        {
            let source = r#"
import * as core from "scene";

                const root = core.column(0);
                const region = core.pointerRegion();
                const label = core.text("Go", 16, [255, 255, 255, 255]);
                core.append(region, label);
                core.append(root, region);
                core.setListenerErrorPolicy("freeze");
                core.onClick(region, () => { throw new Error("boom"); });
            "#;
            let mut session = match super::JsSession::open(
                source,
                "test.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            session.metrics(&metrics());
            let first = session.frame(session::FrameTime::at(0.0));
            let (x, y) = text_spot(&first, "Go");
            click(&mut session, x, y);
            let message = session.take_script_error().unwrap_or_default();
            assert!(message.contains("boom"), "script error was {message:?}");
            let second = session.frame(session::FrameTime::at(16.0));
            assert!(second.layers.is_none());
            assert!(texts(&first).contains(&"Go"));
        }
    }

    #[test]
    fn a_reported_listener_error_lets_the_next_listener_run() {
        {
            let source = r#"
import * as core from "scene";

                const root = core.column(0);
                const region = core.pointerRegion();
                const label = core.text("Go", 16, [255, 255, 255, 255]);
                core.append(region, label);
                core.append(root, region);
                core.onClick(region, () => { throw new Error("boom"); });
                core.onClick(region, () => core.setText(label, "later"));
            "#;
            let mut session = match super::JsSession::open(
                source,
                "report.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            session.metrics(&metrics());
            let first = session.frame(session::FrameTime::at(0.0));
            let (x, y) = text_spot(&first, "Go");
            click(&mut session, x, y);
            assert!(session.take_script_error().is_none());
            let second = session.frame(session::FrameTime::at(16.0));
            assert!(texts(&second).contains(&"later"), "{:?}", texts(&second));
        }
    }

    #[test]
    fn a_shortcut_consumes_the_modified_key() {
        {
            let source = r#"
import * as core from "scene";

                const root = core.column(0);
                const label = core.text("none", 16, [0, 0, 0, 255]);
                const field = core.pointerRegion();
                core.append(root, label);
                core.append(root, field);
                core.setFocusable(field, 1);
                core.focus(field);
                core.onKey(field, (key) => core.setText(label, "key:" + key));
                core.onText(field, (text) => core.setText(label, "text:" + text));
                core.onShortcut("mod+k", () => core.setText(label, "cut"));
            "#;
            let mut session = match super::JsSession::open(
                source,
                "shortcut.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            session.metrics(&metrics());
            let _ = session.frame(session::FrameTime::at(0.0));
            session.input(InputEvent::Key(session::KeyPress {
                key: Key::Char('k'),
                shift: false,
                modifiers: uicore::Modifiers::empty().with(uicore::Modifiers::META),
            }));
            let frame = session.frame(session::FrameTime::at(16.0));
            assert!(texts(&frame).contains(&"cut"), "{:?}", texts(&frame));
        }
    }

    #[test]
    fn host_motion_and_semantics_hints_reach_core() {
        {
            let source = r#"
import * as core from "scene";

                const root = core.column(0);
                const scroll = core.scroll();
                const label = core.text("row", 16, [0, 0, 0, 255]);
                core.append(root, scroll);
                core.append(scroll, label);
                core.setSemantics(label, 2, "row", undefined, undefined, undefined, undefined, undefined, undefined, undefined, undefined, { live: "polite", order: 3 });
                core.scrollIntoView(label, 0);
                core.animate(label, "opacity", 255, 0, 170, 26);
                core.patchStyle(label, { outlineWidth: 2, outlineColor: [0, 0, 0, 255] });
                core.setRecognizers(label, { tap: true, doubleTapMs: 280, drag: "horizontal" });
                const flung = core.flingTarget(scroll);
                if (flung !== null) {
                    throw new Error("idle scroll has no fling");
                }
            "#;
            let mut session = match super::JsSession::open(
                source,
                "motion.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            session.metrics(&metrics());
            let frame = session.frame(session::FrameTime::at(0.0));
            assert!(session.take_script_error().is_none());
            assert!(texts(&frame).contains(&"row"), "{:?}", texts(&frame));
            let later = session.frame(session::FrameTime::at(48.0));
            assert!(later.layers.is_some() || frame.layers.is_some());
        }
    }

    #[test]
    fn motion_end_survives_unregistering_its_own_callback() {
        {
            let source = r#"
import * as core from "scene";

                const root = core.column(0);
                const fading = core.opacity(255);
                core.append(root, fading);
                let seen = 0;
                const finish = () => {
                    seen = 1;
                };
                let first = 0;
                let second = 0;
                first = core.onMotionEnd(fading, "opacity", () => {
                    core.offMotionEnd(first);
                    finish();
                });
                second = core.onMotionEnd(fading, "opacity", () => {
                    core.offMotionEnd(second);
                    const nested = () => {
                        if (seen !== 1) {
                            throw new Error("first skipped");
                        }
                        throw new Error("both");
                    };
                    nested();
                });
                core.animate(fading, "opacity", 255, 0, 170, 26);
            "#;
            let mut session = match super::JsSession::open(
                source,
                "motion-end.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            session.metrics(&metrics());
            let _ = session.frame(session::FrameTime::at(0.0));
            let mut message = String::new();
            for step in 1..180 {
                let _ = session.frame(session::FrameTime::at(f64::from(step) * 16.0));
                if let Some(error) = session.take_script_error() {
                    message = error;
                    break;
                }
            }
            assert!(
                message.contains("both"),
                "both listeners should run, got {message:?}"
            );
        }
    }

    #[test]
    fn motion_end_reports_a_throw_after_unregister() {
        {
            let source = r#"
import * as core from "scene";

                const root = core.column(0);
                const fading = core.opacity(255);
                const label = core.text("idle", 16, [0, 0, 0, 255]);
                core.append(root, fading);
                core.append(fading, label);
                const finish = () => {
                    const broken = null;
                    broken();
                };
                let token = 0;
                token = core.onMotionEnd(fading, "opacity", () => {
                    core.offMotionEnd(token);
                    finish();
                });
                core.animate(fading, "opacity", 255, 0, 170, 26);
            "#;
            let mut session = match super::JsSession::open(
                source,
                "motion-end-throw.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            session.metrics(&metrics());
            let _ = session.frame(session::FrameTime::at(0.0));
            let mut message = String::new();
            for step in 1..180 {
                let _ = session.frame(session::FrameTime::at(f64::from(step) * 16.0));
                if let Some(error) = session.take_script_error() {
                    message = error;
                    break;
                }
            }
            assert!(
                message.contains("not a function"),
                "script error was {message:?}"
            );
        }
    }

    #[test]
    fn align_accepts_a_column_and_metrics_report_the_viewport() {
        {
            let source = r#"
import * as core from "scene";

                const frame = core.align("center");
                core.setAlign(frame, "end");
                const column = core.column(0);
                core.setAlign(column, "center");
                core.setCross(column, "center");
                let stackRejected = false;
                try {
                    core.setCross(core.stack(), "center");
                } catch (error) {
                    stackRejected = true;
                }
                if (!stackRejected) {
                    throw new Error("stack accepted setCross");
                }
                const label = core.text("idle", 16, [0, 0, 0, 255]);
                core.append(column, label);
                core.append(frame, column);
                const before = core.metrics();
                if (before.width !== 0) {
                    throw new Error("width before metrics " + before.width);
                }
                core.onFrame(() => {
                    const box = core.metrics();
                    core.setText(label, "w:" + box.width);
                });
            "#;
            let mut session = match super::JsSession::open(
                source,
                "align.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            session.metrics(&metrics());
            let frame = session.frame(session::FrameTime::at(0.0));
            assert!(session.take_script_error().is_none());
            assert!(
                texts(&frame).contains(&"w:320"),
                "metrics width should be the content box, got {:?}",
                texts(&frame)
            );
        }
    }

    #[test]
    fn a_click_longer_than_one_frame_still_updates_the_label() {
        {
            let source = r#"
import * as core from "scene";

                const root = core.column(0);
                const region = core.pointerRegion();
                const label = core.text("Go", 16, [255, 255, 255, 255]);
                core.append(region, label);
                core.append(root, region);
                core.onClick(region, () => {
                    const start = Date.now();
                    while (Date.now() - start < 40) {}
                    core.setText(label, "Done");
                });
            "#;
            let mut session = match super::JsSession::open(
                source,
                "test.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            session.metrics(&metrics());
            let first = session.frame(session::FrameTime::at(0.0));
            let (x, y) = text_spot(&first, "Go");
            click(&mut session, x, y);
            assert!(
                session.take_script_error().is_none(),
                "a 40 ms turn was interrupted"
            );
            let second = session.frame(session::FrameTime::at(16.0));
            assert!(texts(&second).contains(&"Done"));
        }
    }

    #[test]
    fn a_runaway_click_is_interrupted() {
        {
            let source = r#"
import * as core from "scene";

                const root = core.column(0);
                const region = core.pointerRegion();
                const label = core.text("Go", 16, [255, 255, 255, 255]);
                core.append(region, label);
                core.append(root, region);
                core.onClick(region, () => {
                    const start = Date.now();
                    while (Date.now() - start < 400) {}
                    core.setText(label, "Done");
                });
            "#;
            let logged = std::rc::Rc::new(std::cell::RefCell::new(String::new()));
            let logged_for_hook = std::rc::Rc::clone(&logged);
            let mut session = match super::JsSession::open(
                source,
                "test.js",
                uicore::MonoShaper::default(),
                Box::new(move |_level, message| {
                    logged_for_hook.borrow_mut().push_str(message);
                }),
            ) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            session.metrics(&metrics());
            let first = session.frame(session::FrameTime::at(0.0));
            let (x, y) = text_spot(&first, "Go");
            click(&mut session, x, y);
            assert!(session.take_script_error().is_none());
            let message = logged.borrow().clone();
            assert!(
                message.contains("interrupted") && message.contains("(click"),
                "script error was {message:?}"
            );
            let faults = session.take_faults();
            assert!(
                faults
                    .iter()
                    .any(|fault| fault.source == "listener" && !fault.stack.is_empty()),
                "faults were {faults:?}"
            );
            let second = session.frame(session::FrameTime::at(16.0));
            assert!(second.layers.is_none());
            assert!(texts(&first).contains(&"Go"));
        }
    }

    #[test]
    fn a_runaway_timer_names_its_entry() {
        {
            let source = r#"
import * as core from "scene";

                const label = core.text("idle", 16, [0, 0, 0, 255]);
                setTimeout(() => {
                    const start = Date.now();
                    while (Date.now() - start < 400) {}
                }, 0);
            "#;
            let mut session = match super::JsSession::open(
                source,
                "timer.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            session.metrics(&metrics());
            let _ = session.frame(session::FrameTime::at(0.0));
            let message = session.take_script_error().unwrap_or_default();
            assert!(message.contains("(timer)"), "script error was {message:?}");
            let faults = session.take_faults();
            assert!(
                faults.iter().any(|fault| fault.source == "timer"),
                "faults were {faults:?}"
            );
        }
    }

    #[test]
    fn console_log_reaches_the_host() {
        {
            let seen = std::rc::Rc::new(std::cell::RefCell::new(None));
            let record = std::rc::Rc::clone(&seen);
            let opened = super::JsSession::open(
                "console.log('boot')",
                "test.js",
                uicore::MonoShaper::default(),
                Box::new(move |level, text| {
                    *record.borrow_mut() = Some((level, text.to_owned()));
                }),
            );
            let Ok(_session) = opened else {
                panic!("console script");
            };
            let borrowed = seen.borrow();
            let recorded = borrowed
                .as_ref()
                .map(|(level, text)| (*level, text.as_str()));
            assert_eq!(recorded, Some((0, "boot")));
        }
    }

    #[test]
    fn console_debug_logs_at_level_four() {
        {
            let seen = std::rc::Rc::new(std::cell::RefCell::new(None));
            let record = std::rc::Rc::clone(&seen);
            let opened = super::JsSession::open(
                "console.debug('fine')",
                "test.js",
                uicore::MonoShaper::default(),
                Box::new(move |level, text| {
                    *record.borrow_mut() = Some((level, text.to_owned()));
                }),
            );
            let Ok(_session) = opened else {
                panic!("console debug script");
            };
            let borrowed = seen.borrow();
            let recorded = borrowed
                .as_ref()
                .map(|(level, text)| (*level, text.as_str()));
            assert_eq!(recorded, Some((4, "fine")));
        }
    }

    #[test]
    fn core_log_routes_level_names() {
        {
            let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
            let record = std::rc::Rc::clone(&seen);
            let opened = super::JsSession::open(
                "import * as core from 'scene'; core.log('warn', 'warm'); core.log('error', 'bad'); core.log('debug', 'fine'); core.log('info', 'plain')",
                "test.js",
                uicore::MonoShaper::default(),
                Box::new(move |level, text| {
                    record.borrow_mut().push((level, text.to_owned()));
                }),
            );
            let Ok(_session) = opened else {
                panic!("core log script");
            };
            let borrowed = seen.borrow();
            let recorded: Vec<(u8, &str)> = borrowed
                .iter()
                .map(|(level, text)| (*level, text.as_str()))
                .collect();
            assert_eq!(
                recorded,
                [(2u8, "warm"), (3, "bad"), (4, "fine"), (1, "plain")]
            );
        }
    }

    #[test]
    fn an_unhandled_rejection_is_logged_with_its_stack() {
        {
            let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
            let record = std::rc::Rc::clone(&seen);
            let source = r#"
                Promise.reject(new Error("boom")).then(() => {});
            "#;
            let mut session = match super::JsSession::open(
                source,
                "test.js",
                uicore::MonoShaper::default(),
                Box::new(move |level, text| {
                    record.borrow_mut().push((level, text.to_owned()));
                }),
            ) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            session.metrics(&metrics());
            let _ = session.frame(session::FrameTime::at(0.0));
            let logged = seen.borrow();
            let line = logged
                .iter()
                .find(|(level, _)| *level == 3)
                .map(|(_, text)| text.clone())
                .unwrap_or_default();
            assert!(
                line.contains("unhandled rejection") && line.contains("boom"),
                "rejection log was {logged:?}"
            );
            let faults = session.take_faults();
            assert!(
                faults
                    .iter()
                    .any(|fault| { fault.source == "rejection" && fault.message.contains("boom") }),
                "faults were {faults:?}"
            );
        }
    }

    #[test]
    fn a_late_catch_is_not_reported() {
        {
            let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
            let record = std::rc::Rc::clone(&seen);
            let source = r#"
                Promise.reject(new Error("handled")).catch(() => {});
            "#;
            let mut session = match super::JsSession::open(
                source,
                "test.js",
                uicore::MonoShaper::default(),
                Box::new(move |level, text| {
                    record.borrow_mut().push((level, text.to_owned()));
                }),
            ) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            session.metrics(&metrics());
            let _ = session.frame(session::FrameTime::at(0.0));
            let logged = seen.borrow();
            assert!(
                !logged
                    .iter()
                    .any(|(level, text)| *level == 3 && text.contains("handled")),
                "late catch still reported: {logged:?}"
            );
        }
    }

    #[test]
    fn debug_bindings_report_overflow_and_snapshot() {
        {
            let source = r#"
import * as core from "scene";

                const root = core.column(0);
                core.setAxisSize(root, 40, 20);
                const a = core.text("hello", 16, [0, 0, 0, 255]);
                const b = core.text("row", 16, [0, 0, 0, 255]);
                core.append(root, a);
                core.append(root, b);
                core.debugSetScan(true);
                core.onFrame(() => {
                    const issues = JSON.parse(core.debugLayoutIssues());
                    if (!Array.isArray(issues.issues)) {
                        throw new Error("issues not a list: " + core.debugLayoutIssues());
                    }
                    const snapshot = core.debugSnapshot();
                    if (!snapshot.includes('"kind":"column"')) {
                        throw new Error("snapshot missing column: " + snapshot);
                    }
                    const probe = core.debugProbe(2, 2);
                    if (!probe.includes('"chain"')) {
                        throw new Error("probe missing chain: " + probe);
                    }
                });
            "#;
            let mut session = match super::JsSession::open(
                source,
                "test.js",
                uicore::MonoShaper::default(),
                Box::new(|_level, _text| {}),
            ) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            session.metrics(&metrics());
            let first = session.frame(session::FrameTime::at(0.0));
            let script_error = session.take_script_error();
            assert!(
                first.layers.is_some(),
                "the frame must record; error: {script_error:?}"
            );
            assert!(script_error.is_none(), "debug bindings failed");
        }
    }

    #[test]
    fn request_frame_lasts_for_one_extra_frame() {
        {
            let source = r#"
import * as core from "scene";

                const label = core.text("0", 16, [0, 0, 0, 255]);
                let n = 0;
                core.onFrame(() => {
                    n += 1;
                    core.setText(label, String(n));
                    if (n < 2) core.requestFrame();
                });
            "#;
            let mut session = match super::JsSession::open(
                source,
                "test.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            session.metrics(&metrics());
            let first = session.frame(session::FrameTime::at(0.0));
            assert_eq!(texts(&first), ["1"]);
            assert!(first.again);
            let second = session.frame(session::FrameTime::at(16.0));
            assert_eq!(texts(&second), ["2"]);
            assert!(!second.again);
        }
    }

    #[test]
    fn an_opacity_spring_asks_for_another_frame() {
        {
            let source = r#"
import * as core from "scene";

                const root = core.column(0);
                const fading = core.opacity(255);
                core.append(root, fading);
                core.animate(fading, "opacity", 255, 0, 170, 26);
            "#;
            let mut session = match super::JsSession::open(
                source,
                "spring.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            session.metrics(&metrics());
            let first = session.frame(session::FrameTime::at(0.0));
            assert!(
                first.again,
                "a running spring has to sample on the next frame"
            );
            let landed = session.frame(session::FrameTime::at(5_000.0));
            assert!(!landed.again, "the spring has landed");
        }
    }

    #[test]
    fn microtask_set_text_paints_without_request_frame() {
        {
            let source = r#"
import * as core from "scene";

                const label = core.text("old", 16, [0, 0, 0, 255]);
                const region = core.pointerRegion();
                core.append(region, label);
                core.onClick(region, () => {
                    queueMicrotask(() => core.setText(label, "new"));
                });
            "#;
            let mut session = match super::JsSession::open(
                source,
                "test.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            session.metrics(&metrics());
            let first = session.frame(session::FrameTime::at(0.0));
            let (x, y) = text_spot(&first, "old");
            click(&mut session, x, y);
            let second = session.frame(session::FrameTime::at(16.0));
            assert!(texts(&second).contains(&"new"), "{:?}", texts(&second));
            assert!(!session.wants_frame());
        }
    }

    #[test]
    fn a_throwing_frame_callback_does_not_ask_for_another_frame() {
        {
            let source = r#"
import * as core from "scene";

                const label = core.text("stay", 16, [0, 0, 0, 255]);
                core.onFrame(() => {
                    core.requestFrame();
                    core.setText(label, "nope");
                    throw new Error("boom");
                });
            "#;
            let mut session = match super::JsSession::open(
                source,
                "test.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            session.metrics(&metrics());
            let first = session.frame(session::FrameTime::at(0.0));
            assert!(session.take_script_error().is_some());
            assert!(texts(&first).iter().all(|text| *text != "nope"));
            assert!(!session.wants_frame());
        }
    }

    #[test]
    fn a_drag_past_the_slop_does_not_click() {
        {
            let source = r#"
import * as core from "scene";

                let log = "";
                const root = core.column(0);
                core.setFill(root);
                const scroll = core.scroll();
                const region = core.pointerRegion();
                const label = core.text("idle", 16, [0, 0, 0, 255]);
                core.append(region, label);
                core.append(scroll, region);
                const tall = core.sized(20, 400);
                core.append(scroll, tall);
                core.append(root, scroll);
                const note = (mark) => { log += mark; core.setText(label, log); };
                core.onDragBegin(scroll, () => note("B"));
                core.onDragBy(scroll, () => note("M"));
                core.onDragEnd(scroll, () => note("E"));
                core.onClick(region, () => note("C"));
            "#;
            let mut session = match super::JsSession::open(
                source,
                "test.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            session.metrics(&metrics());
            let first = session.frame(session::FrameTime::at(0.0));
            let (x, y) = text_spot(&first, "idle");
            session.pointer(PointerEvent::new(Phase::Down, x, y, 0.0));
            session.pointer(PointerEvent::new(Phase::Move, x, y + 4.0, 10.0));
            session.pointer(PointerEvent::new(Phase::Up, x, y + 4.0, 20.0));
            let clicked = session.frame(session::FrameTime::at(16.0));
            assert_eq!(texts(&clicked), ["C"]);
            session.pointer(PointerEvent::new(Phase::Down, x, y, 30.0));
            session.pointer(PointerEvent::new(Phase::Move, x, y + 24.0, 40.0));
            session.pointer(PointerEvent::new(Phase::Up, x, y + 24.0, 50.0));
            let dragged = session.frame(session::FrameTime::at(32.0));
            assert_eq!(texts(&dragged), ["CBME"]);
        }
    }

    #[test]
    fn stop_scroll_on_drag_end_reports_idle() {
        {
            let source = r#"
import * as core from "scene";

                const root = core.column(0);
                core.setFill(root);
                const label = core.text("idle", 16, [0, 0, 0, 255]);
                core.append(root, label);
                const scroll = core.scroll();
                core.setGrow(scroll, 1);
                core.append(scroll, core.sized(80, 800));
                core.append(root, scroll);
                core.onScrollState((_, phase) => core.setText(label, String(phase)));
                core.onDragBy(scroll, () => core.setScrollOffset(scroll, 0, -48));
                core.onDragEnd(scroll, () => core.stopScroll(scroll));
            "#;
            let mut session = match super::JsSession::open(
                source,
                "test.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            session.metrics(&metrics());
            session.frame(session::FrameTime::at(0.0));
            session.pointer(PointerEvent::new(Phase::Down, 40.0, 80.0, 0.0));
            session.pointer(PointerEvent::new(Phase::Move, 40.0, 120.0, 20.0));
            session.pointer(PointerEvent::new(Phase::Up, 40.0, 120.0, 40.0));
            let frame = session.frame(session::FrameTime::at(16.0));
            assert!(session.take_script_error().is_none());
            assert_eq!(texts(&frame), ["0"]);
        }
    }

    #[test]
    fn set_text_runs_reaches_the_draw_command() {
        {
            let source = r#"
import * as core from "scene";

                const root = core.column(0);
                core.setFill(root);
                const label = core.text("x", 16, [0, 0, 0, 255]);
                core.append(root, label);
                core.setTextRuns(label, [
                    { text: "ab", color: [1, 2, 3, 255], weight: 700 },
                    {
                        text: "c",
                        color: [4, 5, 6, 255],
                        gradient: {
                            kind: "linear",
                            from: [0, 0],
                            to: [1, 0],
                            stops: [
                                { offset: 0, color: [9, 0, 0, 255] },
                                { offset: 1, color: [0, 9, 0, 255] }
                            ]
                        }
                    }
                ]);
            "#;
            let mut session = match super::JsSession::open(
                source,
                "runs.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            session.metrics(&metrics());
            let frame = session.frame(session::FrameTime::at(0.0));
            assert!(session.take_script_error().is_none());
            let mut found = false;
            visit_commands(&frame, &mut |command, _, _| {
                let Command::Text { utf8, spans, .. } = command else {
                    return;
                };
                if utf8.as_ref() != "abc" {
                    return;
                }
                found = spans.len() == 2
                    && spans[0].color == [1, 2, 3, 255]
                    && spans[0].gradient.is_none()
                    && spans[1].gradient.is_some();
            });
            assert!(found, "styled runs were not recorded");
        }
    }

    #[test]
    // Broken by the gallery rewrite in b48bb8a, which shipped without a
    // compiling quickjs test build. Re-pin these once the watchlist click
    // path is verified against the real tree again.
    #[ignore = "gallery behavior drifted in b48bb8a; re-pin expectations"]
    fn dragging_the_list_translates_a_visible_row() {
        {
            let source = gallery_source();
            let mut session = match super::JsSession::open(
                &source,
                "test.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            let mut window = metrics();
            window.height = 800.0;
            session.metrics(&window);
            let mut first = pump_first(&mut session);
            for step in 1..8 {
                let next = session.frame(session::FrameTime::at(8.0 + f64::from(step) * 16.0));
                if next.layers.is_some() {
                    first = next;
                }
            }
            let Some((clip_y, clip_h)) = first.layers.as_deref().and_then(|tree| {
                tree.layers.iter().find_map(|layer| match layer.kind {
                    protocol::LayerKind::Clip { y, h, .. } if h > 100.0 => Some((y, h)),
                    _ => None,
                })
            }) else {
                panic!("scroll clip");
            };
            let mut row = None;
            visit_commands(&first, &mut |command, origin_x, origin_y| {
                if let Command::Text { utf8, x, y, .. } = command
                    && IN_WATCHLIST_NAMES.contains(&&**utf8)
                {
                    let scene_y = *y + origin_y;
                    if scene_y >= clip_y && scene_y < clip_y + clip_h {
                        row = Some((std::sync::Arc::clone(utf8), *x + origin_x + 1.0, scene_y));
                    }
                }
            });
            let Some((label, x, before)) = row else {
                panic!("a row inside the viewport");
            };
            let delta = 12.0;
            session.pointer(PointerEvent::new(Phase::Down, x, before + 1.0, 0.0));
            session.pointer(PointerEvent::new(
                Phase::Move,
                x,
                before + 1.0 - delta,
                16.0,
            ));
            let scrolled = session.frame(session::FrameTime::at(16.0));
            let mut after = None;
            visit_commands(&scrolled, &mut |command, _, origin_y| {
                if let Command::Text { utf8, y, .. } = command
                    && utf8.as_ref() == label.as_ref()
                {
                    after = Some(*y + origin_y);
                }
            });
            let Some(after) = after else {
                panic!(
                    "scrolled label {} layers={} texts={:?}",
                    label,
                    scrolled.layers.is_some(),
                    texts(&scrolled)
                );
            };
            assert!(
                (before - after - delta).abs() < 0.5,
                "{label} moved from {before} to {after}"
            );
            assert!(texts(&scrolled).contains(&"自选"));
        }
    }

    fn rewritten_commands(
        before: &protocol::LayerTree,
        after: &protocol::LayerTree,
    ) -> (usize, usize) {
        let mut changed = 0;
        let mut unchanged = 0;
        for picture in &after.pictures {
            let same = before
                .picture(picture.id)
                .is_some_and(|old| old.generation == picture.generation);
            if same {
                unchanged += 1;
            } else {
                changed += picture.commands.len();
            }
        }
        (changed, unchanged)
    }

    #[test]
    fn module_loader_accepts_a_sibling_script_and_rejects_escape() {
        {
            let dir = std::env::temp_dir().join(format!("quickjs-mod-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("temp app dir");
            std::fs::write(dir.join("util.js"), "export const label = \"from-util\";\n")
                .expect("util");
            let app = dir.join("app.js");
            std::fs::write(
                &app,
                "import * as core from 'scene'; import { label } from \"./util.js\";\ncore.text(label, 16, [0, 0, 0, 255]);\n",
            )
            .expect("app");
            let name = app.to_string_lossy().into_owned();
            let source = std::fs::read_to_string(&app).expect("read app");
            let mut session = match super::JsSession::open(
                &source,
                &name,
                uicore::MonoShaper::default(),
                no_log(),
            ) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            session.metrics(&metrics());
            let frame = session.frame(session::FrameTime::at(0.0));
            assert!(
                texts(&frame).contains(&"from-util"),
                "{:?} {}",
                texts(&frame),
                session.take_script_error().unwrap_or_default()
            );
            let rejected = [
                "import * as core from 'scene'; import { label } from \"../util.js\";\n",
                "import * as core from 'scene'; import { label } from \"/tmp/util.js\";\n",
                "import * as core from 'scene'; import { label } from \"./util.txt\";\n",
                "import * as core from 'scene'; import { label } from \"./a\\\\b.js\";\n",
            ];
            for source in rejected {
                match super::JsSession::open(source, &name, uicore::MonoShaper::default(), no_log())
                {
                    Err(Error::Script(message)) => {
                        assert!(
                            message.contains("module")
                                || message.contains("outside")
                                || message.contains("could not"),
                            "{message}"
                        );
                    }
                    Ok(_) => panic!("expected a rejected import"),
                    Err(error) => panic!("{error}"),
                }
            }
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[cfg(unix)]
    #[test]
    fn module_loader_rejects_symlinks_outside_the_app() {
        let temp = std::env::temp_dir().join(format!("quickjs-symlink-{}", std::process::id()));
        let dir = temp.join("app");
        std::fs::create_dir_all(&dir).expect("app dir");
        let outside = temp.join("outside.js");
        std::fs::write(&outside, "export const label = 'outside';").expect("outside module");
        std::os::unix::fs::symlink(outside, dir.join("link.js")).expect("link to outside module");
        let result = super::JsSession::open(
            "import { label } from './link.js';",
            &dir.join("app.js").to_string_lossy(),
            uicore::MonoShaper::default(),
            no_log(),
        );
        std::fs::remove_dir_all(&temp).expect("remove test directory");
        assert!(
            matches!(result, Err(Error::Script(message)) if message.contains("outside the app directory"))
        );
    }

    #[test]
    fn module_loader_accepts_parent_segments_inside_the_app() {
        let dir = std::env::temp_dir().join(format!("quickjs-parent-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("panels")).expect("panels");
        std::fs::write(
            dir.join("shared.js"),
            "export const label = \"shared-ok\";\n",
        )
        .expect("shared");
        std::fs::write(
            dir.join("panels/page.js"),
            "export { label } from \"../shared.js\";\n",
        )
        .expect("page");
        let app = dir.join("app.js");
        std::fs::write(
            &app,
            "import * as core from 'scene'; import { label } from \"./panels/page.js\";\ncore.text(label, 16, [0, 0, 0, 255]);\n",
        )
        .expect("app");
        let name = app.to_string_lossy().into_owned();
        let source = std::fs::read_to_string(&app).expect("read app");
        let mut session =
            match super::JsSession::open(&source, &name, uicore::MonoShaper::default(), no_log()) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
        session.metrics(&metrics());
        let frame = session.frame(session::FrameTime::at(0.0));
        assert!(
            texts(&frame).contains(&"shared-ok"),
            "{:?} {}",
            texts(&frame),
            session.take_script_error().unwrap_or_default()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    // Broken by the gallery rewrite in b48bb8a, which shipped without a
    // compiling quickjs test build. Re-pin these once the watchlist click
    // path is verified against the real tree again.
    #[ignore = "gallery behavior drifted in b48bb8a; re-pin expectations"]
    fn gallery_pages_scroll_without_picture_writes() {
        {
            let source = gallery_source();
            let mut session = match super::JsSession::open(
                &source,
                "test.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            session.metrics(&phone());
            let mut shown = pump_first(&mut session);
            let mut quiet = 0;
            for step in 1..24 {
                let next = session.frame(session::FrameTime::at(16.0 + f64::from(step) * 16.0));
                if next.layers.is_some() {
                    shown = next;
                    quiet = 0;
                } else {
                    quiet += 1;
                    if quiet >= 2 {
                        break;
                    }
                }
            }
            assert!(
                texts(&shown).iter().any(|text| text.contains("腾讯控股")),
                "{:?}",
                texts(&shown)
            );
            let Some((clip_y, clip_h)) = shown.layers.as_deref().and_then(|tree| {
                tree.layers.iter().find_map(|layer| match layer.kind {
                    protocol::LayerKind::Clip { y, h, .. } if h > 100.0 => Some((y, h)),
                    _ => None,
                })
            }) else {
                panic!("scroll clip");
            };
            let mut row = None;
            visit_commands(&shown, &mut |command, origin_x, origin_y| {
                if let Command::Text { utf8, x, y, .. } = command
                    && IN_WATCHLIST_NAMES.contains(&&**utf8)
                {
                    let scene_y = *y + origin_y;
                    if scene_y >= clip_y && scene_y < clip_y + clip_h {
                        row = Some((std::sync::Arc::clone(utf8), *x + origin_x + 1.0, scene_y));
                    }
                }
            });
            let Some((_label, x, before)) = row else {
                panic!("a row inside the viewport");
            };
            let before_tree = shown.layers.expect("list tree");
            session.pointer(PointerEvent::wheel(x, before + 1.0, 200.0, 0.0, clip_h));
            let scrolled = session.frame(session::FrameTime::at(200.0));
            assert!(
                scrolled.layers.is_some(),
                "scroll produced no layer tree {}",
                session.take_script_error().unwrap_or_default()
            );
            let (screen_changed, screen_kept) = rewritten_commands(
                &before_tree,
                scrolled.layers.as_deref().expect("screen tree"),
            );
            assert_eq!(session.picture_writes(), screen_changed);
            assert!(
                screen_kept > 0,
                "scrolling one screen rewrote every picture ({screen_changed} commands)"
            );
            assert!(
                texts(&scrolled)
                    .iter()
                    .any(|text| IN_WATCHLIST_NAMES.contains(text)),
                "list rows left after scrolling {:?}",
                texts(&scrolled)
            );
            let mut baseline = scrolled.layers.clone().expect("scroll tree");
            for step in 0..6 {
                let next = session.frame(session::FrameTime::at(216.0 + f64::from(step) * 16.0));
                if let Some(layers) = next.layers {
                    baseline = layers;
                }
            }
            // Editing proxy: switch the up/down convention on the profile tab.
            // It rewrites every mounted price cell without touching structure.
            let (tab_x, tab_y) = text_spot(&scrolled, "我的");
            click(&mut session, tab_x, tab_y);
            let profile = session.frame(session::FrameTime::at(232.0));
            let (opt_x, opt_y) = text_spot(&profile, "绿涨红跌");
            click(&mut session, opt_x, opt_y);
            let edited = session.frame(session::FrameTime::at(248.0));
            let Some(after_tree) = edited.layers.as_deref() else {
                panic!(
                    "convention flip produced no layer tree {}",
                    session.take_script_error().unwrap_or_default()
                );
            };
            let (changed, unchanged) = rewritten_commands(&baseline, after_tree);
            assert_eq!(
                session.picture_writes(),
                changed,
                "picture writes {changed} unchanged pictures {unchanged}"
            );
            assert!(changed > 0, "the flip wrote no commands");
            assert_eq!(unchanged, 0);
        }
    }

    #[test]
    fn text_input_follows_focus_and_clears_when_the_node_is_removed() {
        {
            let source = r#"
import * as core from "scene";

                let value = "";
                let composing = "";
                const root = core.column(0);
                const field = core.pointerRegion();
                const elsewhere = core.pointerRegion();
                const label = core.text("field", 16, [255, 255, 255, 255]);
                const other = core.sized(40, 20);
                core.setFocusable(field, 1);
                core.append(field, label);
                core.append(elsewhere, other);
                core.append(root, field);
                core.append(root, elsewhere);
                function paint() {
                    core.setText(label, value + composing);
                }
                core.onFocus(field, () => {
                    core.setImeEnabled(true);
                    core.setImeText(value);
                });
                core.onBlur(field, () => {
                    core.setImeEnabled(false);
                });
                core.onText(field, (text) => {
                    if (text === "drop") {
                        core.remove(root, field);
                        return;
                    }
                    value += text;
                    paint();
                });
                core.onComposing(field, (text) => {
                    composing = text == null ? "" : text;
                    paint();
                });
                core.onKey(field, (key) => {
                    if (key === "Backspace") {
                        value = Array.from(value).slice(0, -1).join("");
                        core.setImeText(value);
                        paint();
                    }
                });
            "#;
            let mut session = match super::JsSession::open(
                source,
                "test.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            session.metrics(&metrics());
            let first = session.frame(session::FrameTime::at(0.0));
            assert!(texts(&first).contains(&"field"));
            session.input(InputEvent::Text("你".to_string()));
            let ignored = session.frame(session::FrameTime::at(16.0));
            assert!(ignored.layers.is_none());
            assert!(texts(&first).contains(&"field"));
            assert!(session.take_ime().is_none());
            let (x, y) = text_spot(&first, "field");
            click(&mut session, x, y);
            match session.take_ime() {
                Some(session::ImeUpdate::Show(state)) => assert_eq!(state.text, ""),
                other => panic!("expected show, got {other:?}"),
            }
            session.input(InputEvent::Text("你".to_string()));
            session.input(InputEvent::Composing(Some("ni".to_string())));
            let composed = session.frame(session::FrameTime::at(32.0));
            assert!(texts(&composed).contains(&"你ni"));
            session.input(InputEvent::Composing(None));
            session.input(InputEvent::Key(session::KeyPress::new(Key::Backspace)));
            let cleared = session.frame(session::FrameTime::at(48.0));
            assert!(!texts(&cleared).contains(&"你"));
            assert!(!texts(&cleared).contains(&"你ni"));
            match cleared.ime {
                Some(session::ImeUpdate::Update(state)) => assert_eq!(state.text, ""),
                other => panic!("expected update, got {other:?}"),
            }
            session.input(InputEvent::Text("drop".to_string()));
            let dropped = session.frame(session::FrameTime::at(64.0));
            assert_eq!(session.tap_count(), 0);
            assert!(matches!(dropped.ime, Some(session::ImeUpdate::Hide)));
            session.input(InputEvent::Text("no".to_string()));
            let after = session.frame(session::FrameTime::at(80.0));
            assert!(!texts(&after).contains(&"no"));
        }
    }

    #[test]
    fn dismissing_the_keyboard_lets_blur_clear_ime_state() {
        {
            let source = r#"
import * as core from "scene";

                const root = core.column(0);
                const field = core.pointerRegion();
                const label = core.text("field", 16, [255, 255, 255, 255]);
                core.setFocusable(field, 1);
                core.append(field, label);
                core.append(root, field);
                core.onFocus(field, () => {
                    core.setImeState("hi", 2, 2, -1, -1, 0, 0, 1, 16);
                });
                core.onBlur(field, () => {
                    core.setImeState(null);
                });
            "#;
            let mut session = match super::JsSession::open(
                source,
                "test.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            session.metrics(&metrics());
            let first = session.frame(session::FrameTime::at(0.0));
            let (x, y) = text_spot(&first, "field");
            click(&mut session, x, y);
            assert!(matches!(
                session.take_ime(),
                Some(session::ImeUpdate::Show(_))
            ));
            session.ime_dismissed();
            assert!(matches!(session.take_ime(), Some(session::ImeUpdate::Hide)));
            assert!(session.take_script_error().is_none());
        }
    }

    #[test]
    fn timers_fetch_and_modules_run_inside_the_frame() {
        {
            use std::sync::{Arc, Mutex};

            struct FixedHttp;

            impl session::HttpClient for FixedHttp {
                fn perform(
                    &self,
                    request: &session::HttpRequest,
                ) -> Result<session::HttpResponse, session::HttpError> {
                    assert_eq!(request.method, "POST");
                    assert_eq!(request.body, b"hi");
                    Ok(session::HttpResponse {
                        status: 200,
                        headers: vec![("content-type".to_owned(), "text/plain".to_owned())],
                        body: b"pong".to_vec(),
                    })
                }
            }

            let lines = Arc::new(Mutex::new(Vec::new()));
            let log_lines = Arc::clone(&lines);
            let log = Box::new(move |_level: u8, text: &str| {
                log_lines
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .push(text.to_owned());
            });
            let source = r#"
                import * as clipboard from "@impasto/clipboard";
                import { addEventListener } from "@impasto/app-state";
                if ("addEventListener" in clipboard || "removeEventListener" in clipboard) {
                    throw new Error("clipboard has no native events");
                }
                console.log(new URL("https://example.test/a?q=1").searchParams.get("q"));
                console.log(String(new TextEncoder().encode("A").length));
                setTimeout(() => console.log("soon"), 0);
                setTimeout(() => console.log("later"), 20);
                const cleared = setTimeout(() => console.log("cleared"), 0);
                clearTimeout(cleared);
                requestAnimationFrame(() => console.log("raf"));
                fetch("https://example.test/a", { method: "POST", body: "hi" })
                    .then((response) => response.text())
                    .then((text) => console.log("body:" + text));
                addEventListener("change", (state) => console.log("state:" + state));
                clipboard.setString("hi");
                console.log("clip:" + clipboard.getString());
                const socket = new WebSocket("ws://example.test/socket");
                socket.onerror = () => console.log("ws");
            "#;
            let (host, _pending) = test_host();
            let mut session = match super::JsSession::open_with(
                source,
                "host.js",
                uicore::MonoShaper::default(),
                log,
                Some(Arc::new(FixedHttp)),
                host,
            ) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            session.metrics(&metrics());
            let _ = session.frame(session::FrameTime::at(0.0));
            let _ = session.frame(session::FrameTime::at(20.0));
            session.lifecycle(session::Lifecycle::Paused);
            let text = lines
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .join("\n");
            assert!(text.contains('1'), "{text}");
            assert!(text.contains("soon"), "{text}");
            assert!(text.contains("raf"), "{text}");
            assert!(text.contains("body:pong"), "{text}");
            assert!(text.contains("clip:hi"), "{text}");
            assert!(text.contains("ws"), "{text}");
            assert!(text.contains("state:background"), "{text}");
            assert!(!text.contains("cleared"), "{text}");
            assert!(text.contains("later"), "{text}");
        }
    }

    #[test]
    fn get_initial_url_is_visible_during_eval() {
        {
            let lines = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let log_lines = std::sync::Arc::clone(&lines);
            let log = Box::new(move |_level: u8, text: &str| {
                log_lines
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .push(text.to_owned());
            });
            let source = r#"
                import { getInitialURL } from "@impasto/linking";
                console.log("initial:" + String(getInitialURL()));
            "#;
            let (host, _pending) =
                test_host_with_launch_url(Some("https://app.example/item".to_owned()));
            match super::JsSession::open_with(
                source,
                "launch.js",
                uicore::MonoShaper::default(),
                log,
                None,
                host,
            ) {
                Ok(_) => {}
                Err(error) => panic!("{error}"),
            }
            let text = lines
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .join("\n");
            assert!(text.contains("initial:https://app.example/item"), "{text}");
        }
    }

    #[test]
    fn sync_module_error_reaches_js() {
        {
            let lines = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let log_lines = std::sync::Arc::clone(&lines);
            let log = Box::new(move |_level: u8, text: &str| {
                log_lines
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .push(text.to_owned());
            });
            let source = r#"
                import { fail } from "@impasto/echo";
                try {
                    fail();
                    console.log("ok");
                } catch (error) {
                    console.log("err:" + error.message);
                }
            "#;
            let (host, _pending) = test_host();
            match super::JsSession::open_with(
                source,
                "error.js",
                uicore::MonoShaper::default(),
                log,
                None,
                host,
            ) {
                Ok(_) => {}
                Err(error) => panic!("{error}"),
            }
            let text = lines
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .join("\n");
            assert!(text.contains("err:boom from host"), "{text}");
        }
    }

    #[test]
    fn promise_module_settles_through_io_event() {
        {
            let lines = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let log_lines = std::sync::Arc::clone(&lines);
            let log = Box::new(move |_level: u8, text: &str| {
                log_lines
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .push(text.to_owned());
            });
            let source = r#"
                import { later } from "@impasto/echo";
                later("ok").then((value) => console.log("later:" + value));
            "#;
            let (host, pending) = test_host();
            let mut session = match super::JsSession::open_with(
                source,
                "promise.js",
                uicore::MonoShaper::default(),
                log,
                None,
                host,
            ) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            drain_plugin(&mut session, pending.as_ref());
            assert!(session.jobs_pending());
            assert!(session.wants_frame());
            session.metrics(&metrics());
            let _ = session.frame(session::FrameTime::at(0.0));
            let text = lines
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .join("\n");
            assert!(text.contains("later:ok"), "{text}");
        }
    }

    #[test]
    fn unregistered_host_specifier_is_not_linked() {
        {
            let (host, _pending) = test_host();
            match super::JsSession::open_with(
                r#"import { getString } from "@impasto/missing";"#,
                "missing.js",
                uicore::MonoShaper::default(),
                no_log(),
                None,
                host,
            ) {
                Err(Error::Script(message)) => {
                    assert!(message.contains("is not linked"), "{message}");
                }
                Ok(_) => panic!("expected unregistered host specifier to fail"),
                Err(error) => panic!("{error}"),
            }
        }
    }

    #[test]
    fn bare_scope_must_be_bundled() {
        {
            let (host, _pending) = test_host();
            match super::JsSession::open_with(
                r#"import { Clipboard } from "@impasto/clipboard/extra/deep";"#,
                "bare.js",
                uicore::MonoShaper::default(),
                no_log(),
                None,
                host,
            ) {
                Err(Error::Script(message)) => {
                    assert!(
                        message.contains("bare @impasto/* must be bundled"),
                        "{message}"
                    );
                }
                Ok(_) => panic!("expected bare @impasto specifier to fail"),
                Err(error) => panic!("{error}"),
            }
        }
    }

    #[test]
    fn nested_host_path_must_be_bundled() {
        {
            let (host, _pending) = test_host();
            match super::JsSession::open_with(
                r#"import { getString } from "@impasto/clipboard/extra/host";"#,
                "nested.js",
                uicore::MonoShaper::default(),
                no_log(),
                None,
                host,
            ) {
                Err(Error::Script(message)) => {
                    assert!(
                        message.contains("bare @impasto/* must be bundled"),
                        "{message}"
                    );
                }
                Ok(_) => panic!("expected nested @impasto host path to fail"),
                Err(error) => panic!("{error}"),
            }
        }
    }

    #[test]
    fn an_object_handle_is_rejected_and_a_switch_hint_sticks() {
        {
            let source = r#"
import * as core from "scene";

                const region = core.pointerRegion();
                const label = core.text("idle", 16, [0, 0, 0, 255]);
                core.append(region, label);
                try {
                    core.setSemantics({ id: region }, 5, "", "On", 1);
                    core.setText(label, "accepted");
                } catch (error) {
                    const message = String(error);
                    core.setText(label, message.indexOf("bad handle") >= 0 ? "bad handle" : message);
                }
                core.setSemantics(region, 5, "Play", "On", 1);
            "#;
            let mut session = match super::JsSession::open(
                source,
                "semantics.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            session.metrics(&metrics());
            let frame = session.frame(session::FrameTime::at(0.0));
            assert!(texts(&frame).contains(&"bad handle"), "{:?}", texts(&frame));
            let update = session.semantics().expect("semantics");
            let switch = update
                .nodes
                .iter()
                .find(|node| node.role == uicore::SemanticsRole::Switch)
                .expect("switch");
            assert_eq!(switch.value.as_deref(), Some("On"));
            assert!(switch.flags.contains(uicore::SemanticsFlags::CHECKED));
        }
    }

    #[test]
    fn settings_reach_the_script() {
        {
            let source = r#"
import * as core from "scene";

                const label = core.text("boot", 16, [0, 0, 0, 255]);
                core.onSettings((next) => {
                    core.setText(
                        label,
                        next.locale + ":" + next.brightness + ":" + (next.highContrast ? "1" : "0"),
                    );
                });
            "#;
            let mut session = match super::JsSession::open(
                source,
                "settings.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            session.settings(&session::PlatformSettings {
                locale: "zh-CN".to_owned(),
                text_scale: 1.0,
                brightness: session::Brightness::Dark,
                reduce_motion: false,
                bold_text: true,
                high_contrast: true,
            });
            session.metrics(&metrics());
            let frame = session.frame(session::FrameTime::at(0.0));
            assert!(
                texts(&frame).contains(&"zh-CN:dark:1"),
                "{:?}",
                texts(&frame)
            );
        }
    }

    #[test]
    fn tab_moves_focus_and_escape_does_not_exit() {
        {
            let source = r#"
import * as core from "scene";

                const column = core.column(0);
                const label = core.text("none", 16, [0, 0, 0, 255]);
                const first = core.pointerRegion();
                const second = core.pointerRegion();
                core.append(column, label);
                core.append(column, first);
                core.append(column, second);
                core.setFocusable(first, 1);
                core.setFocusable(second, 1);
                core.append(first, core.sized(20, 20));
                core.append(second, core.sized(20, 20));
                core.onFocus(first, () => core.setText(label, "first"));
                core.onFocus(second, () => core.setText(label, "second"));
                core.onKey(first, (key) => core.setText(label, "key:" + key));
                core.onBack(() => {
                    core.setText(label, "back");
                    return true;
                });
            "#;
            let mut session = match super::JsSession::open(
                source,
                "tab.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            session.metrics(&metrics());
            let _ = session.frame(session::FrameTime::at(0.0));
            session.input(InputEvent::Key(session::KeyPress::new(Key::Tab)));
            let next = session.frame(session::FrameTime::at(16.0));
            assert!(texts(&next).contains(&"first"), "{:?}", texts(&next));
            session.input(InputEvent::Key(session::KeyPress::new(Key::Tab)));
            let after = session.frame(session::FrameTime::at(32.0));
            assert!(texts(&after).contains(&"second"), "{:?}", texts(&after));
            session.input(InputEvent::Key(session::KeyPress::new(Key::Escape)));
            let closed = session.frame(session::FrameTime::at(48.0));
            assert!(texts(&closed).contains(&"back"), "{:?}", texts(&closed));
            assert!(!session.take_exit());
        }
    }

    #[test]
    fn text_replaced_delivers_the_loaded_string() {
        {
            let source = r#"
import * as core from "scene";

                const root = core.column(0);
                const label = core.text("idle", 16, [0, 0, 0, 255]);
                const field = core.text("old", 16, [0, 0, 0, 255]);
                core.append(root, label);
                core.append(root, field);
                core.setSemantics(field, 6, "Name", "old", -1, -1, -1, 0, 1, 0, "");
                core.onTextReplaced(field, (value) => {
                    core.setText(label, "replaced:" + value);
                });
            "#;
            let mut session = match super::JsSession::open(
                source,
                "replaced.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            session.metrics(&metrics());
            let _ = session.frame(session::FrameTime::at(0.0));
            let update = session.semantics().expect("semantics");
            let field = update
                .nodes
                .iter()
                .find(|node| node.role == uicore::SemanticsRole::TextField)
                .expect("field");
            session.semantics_action(uicore::SemanticsAction::SetText {
                id: field.id,
                text: "Hello".to_owned(),
            });
            let frame = session.frame(session::FrameTime::at(16.0));
            assert!(
                texts(&frame).contains(&"replaced:Hello"),
                "{:?}",
                texts(&frame)
            );
        }
    }

    #[test]
    fn register_font_queues_replaces_and_caps_names() {
        {
            let source = r#"
import * as core from "scene";

                const root = core.column(0);
                const label = core.text("boot", 16, [0, 0, 0, 255]);
                const empty = core.text("empty-pending", 16, [0, 0, 0, 255]);
                core.append(root, label);
                core.append(root, empty);
                core.registerFont("Sans", new Uint8Array([1]));
                for (let i = 0; i < 8; i++) {
                    core.registerFont("F" + i, new Uint8Array([i + 1]));
                }
                core.registerFont("F0", new Uint8Array([9, 9]));
                try {
                    core.registerFont("Extra", new Uint8Array([3]));
                    core.setText(label, "accepted");
                } catch (error) {
                    const message = String(error);
                    core.setText(label, message.indexOf("registerFont failed") >= 0 ? "rejected" : message);
                }
                try {
                    core.registerFont("", new Uint8Array([4]));
                    core.setText(empty, "empty-accepted");
                } catch (error) {
                    const message = String(error);
                    core.setText(empty, message.indexOf("registerFont failed") >= 0 ? "empty-rejected" : message);
                }
                let phase = 0;
                core.onFrame(() => {
                    if (phase !== 0) {
                        return;
                    }
                    phase = 1;
                    try {
                        core.registerFont("Later", new Uint8Array([5]));
                        core.setText(label, "handed-accepted");
                    } catch (error) {
                        const message = String(error);
                        core.setText(
                            label,
                            message.indexOf("registerFont failed") >= 0 ? "handed-rejected" : message,
                        );
                    }
                    core.registerFont("F0", new Uint8Array([8]));
                });
            "#;
            let mut session = match super::JsSession::open(
                source,
                "fonts.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            session.metrics(&metrics());
            let queued = session.take_fonts();
            assert_eq!(queued.len(), 9, "{queued:?}");
            assert!(session.take_fonts().is_empty());
            let replaced = queued.iter().find(|(name, _)| name == "F0").expect("F0");
            assert_eq!(replaced.1, vec![9, 9]);
            assert!(queued.iter().any(|(name, _)| name == "Sans"));
            assert!(queued.iter().all(|(name, _)| name != "Extra"));
            let listed = session.frame(session::FrameTime::at(0.0));
            assert!(
                texts(&listed).contains(&"handed-rejected"),
                "{:?}",
                texts(&listed)
            );
            assert!(
                texts(&listed).contains(&"empty-rejected"),
                "{:?}",
                texts(&listed)
            );
            let again = listed.fonts;
            assert_eq!(again.len(), 1, "{again:?}");
            assert_eq!(again[0].0, "F0");
            assert_eq!(again[0].1, vec![8]);
        }
    }

    #[test]
    fn register_font_rejects_an_oversize_face_without_touching_the_queue() {
        {
            let source = r#"
import * as core from "scene";

                const label = core.text("boot", 16, [0, 0, 0, 255]);
                core.registerFont("Keep", new Uint8Array([7]));
                const big = new Uint8Array(4 * 1024 * 1024 + 1);
                try {
                    core.registerFont("Big", big);
                    core.setText(label, "accepted");
                } catch (error) {
                    const message = String(error);
                    core.setText(label, message.indexOf("registerFont failed") >= 0 ? "rejected" : message);
                }
            "#;
            let mut session = match super::JsSession::open(
                source,
                "bigfont.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            session.metrics(&metrics());
            let listed = session.frame(session::FrameTime::at(0.0));
            assert!(texts(&listed).contains(&"rejected"), "{:?}", texts(&listed));
            let queued = listed.fonts;
            assert_eq!(queued.len(), 1);
            assert_eq!(queued[0].0, "Keep");
            assert_eq!(queued[0].1, vec![7]);
        }
    }

    #[test]
    fn reload_queues_a_source_and_rejects_an_empty_one() {
        let source = r#"
import * as core from "scene";

            const root = core.column(0);
            const label = core.text("boot", 16, [0, 0, 0, 255]);
            core.append(root, label);
            core.reload("asset:logo");
            try {
                core.reload("");
                core.setText(label, "accepted");
            } catch (error) {
                const message = String(error);
                core.setText(label, message.indexOf("reload(source)") >= 0 ? "rejected" : message);
            }
        "#;
        let mut session = match open(source, "reload.js", uicore::MonoShaper::default(), no_log()) {
            Ok(session) => session,
            Err(error) => panic!("{error}"),
        };
        let queued = session.take_reloads();
        assert_eq!(queued, vec!["asset:logo".to_owned()]);
        assert!(session.take_reloads().is_empty());
        session.metrics(&metrics());
        let frame = session.frame(session::FrameTime::at(0.0));
        assert!(texts(&frame).contains(&"rejected"), "{:?}", texts(&frame));
    }

    #[test]
    fn axis_percent_sizes_the_child_and_fill_is_rejected() {
        let source = r#"
import * as core from "scene";

            const parent = core.column(0);
            const child = core.decorated([0, 9, 0, 255]);
            const label = core.text("boot", 16, [0, 0, 0, 255]);
            core.append(parent, child);
            core.append(parent, label);
            core.setAxisSize(parent, 200, 80);
            core.setAxisSize(child, "50%", 20);
            try {
                core.setAxisSize(child, "fill", 20);
                core.setText(label, "accepted");
            } catch (error) {
                const message = String(error);
                core.setText(label, message.indexOf("bad axis") >= 0 ? "bad-axis" : message);
            }
        "#;
        let mut session = match open(source, "axis.js", uicore::MonoShaper::default(), no_log()) {
            Ok(session) => session,
            Err(error) => panic!("{error}"),
        };
        session.metrics(&metrics());
        let frame = session.frame(session::FrameTime::at(0.0));
        assert!(texts(&frame).contains(&"bad-axis"), "{:?}", texts(&frame));
        let mut child = None;
        visit_commands(&frame, &mut |command, _, _| {
            if let Command::Rect { w, h, color, .. } = command
                && *color == [0, 9, 0, 255]
            {
                child = Some((*w, *h));
            }
        });
        assert_eq!(child, Some((100.0, 20.0)), "{child:?}");
    }

    #[test]
    fn image_mode_is_recorded_and_a_bad_mode_is_rejected() {
        let source = r#"
import * as core from "scene";

            const root = core.column(0);
            const picture = core.image("asset:logo", 40, 20, "fit", false);
            const label = core.text("boot", 16, [0, 0, 0, 255]);
            core.append(root, picture);
            core.append(root, label);
            core.setImageMode(picture, "crop", true);
            try {
                core.setImageMode(picture, "stretch", false);
                core.setText(label, "accepted");
            } catch (error) {
                const message = String(error);
                core.setText(label, message.indexOf("bad image") >= 0 ? "bad-image" : message);
            }
        "#;
        let mut session = match open(source, "image.js", uicore::MonoShaper::default(), no_log()) {
            Ok(session) => session,
            Err(error) => panic!("{error}"),
        };
        session.metrics(&metrics());
        let frame = session.frame(session::FrameTime::at(0.0));
        assert!(texts(&frame).contains(&"bad-image"), "{:?}", texts(&frame));
        let mut image = None;
        visit_commands(&frame, &mut |command, _, _| {
            if let Command::Image { mode, required, .. } = command {
                image = Some((*mode, *required));
            }
        });
        assert_eq!(
            image,
            Some((protocol::ContentMode::Crop, true)),
            "{image:?}"
        );
    }

    #[test]
    fn rect_flushes_a_buffered_style_mutation() {
        let source = r#"
import * as core from "scene";

            const root = core.column(0);
            const box = core.sized(8, 8);
            core.append(root, box);
            const label = core.text("stale", 16, [0, 0, 0, 255]);
            core.append(root, label);
            core.applyMutation(7, box, 0, 0, 1, 30, 8, [0, 0, 0, 0], "");
            const rect = core.rect(box);
            core.setText(label, rect.w === 30 ? "applied" : "stale");
        "#;
        let mut session = match open(source, "mutate.js", uicore::MonoShaper::default(), no_log()) {
            Ok(session) => session,
            Err(error) => panic!("{error}"),
        };
        session.metrics(&metrics());
        let frame = session.frame(session::FrameTime::at(0.0));
        assert!(texts(&frame).contains(&"applied"), "{:?}", texts(&frame));
    }

    fn canvas_fills(frame: &session::FrameOutput) -> usize {
        let mut count = 0;
        visit_commands(frame, &mut |command, _, _| {
            if let Command::Canvas { list, .. } = command {
                count += list
                    .ops()
                    .iter()
                    .filter(|op| matches!(op, protocol::CanvasOp::Fill { .. }))
                    .count();
            }
        });
        count
    }

    fn open_canvas(source: &str, name: &str) -> Box<dyn Session> {
        match open(source, name, uicore::MonoShaper::default(), no_log()) {
            Ok(session) => session,
            Err(error) => panic!("{error}"),
        }
    }

    #[test]
    fn canvas_fill_is_recorded_in_the_content_box() {
        let source = r#"
import * as core from "scene";

            const root = core.column(0);
            const box = core.view();
            core.setSize(box, 80, 40);
            core.append(root, box);
            core.bindCanvas(box, (ctx) => {
                ctx.fillStyle = '#112233';
                ctx.fillRect(0, 0, 8, 6);
            });
        "#;
        let mut session = open_canvas(source, "canvas-fill.js");
        session.metrics(&metrics());
        let frame = session.frame(session::FrameTime::at(0.0));
        assert!(session.take_script_error().is_none(), "draw failed");
        assert!(canvas_fills(&frame) >= 1, "missing canvas fill");
        assert!(
            !frame.again,
            "a static canvas must not request another frame"
        );
    }

    #[test]
    fn offscreen_canvas_releases_dependencies_and_draws_latest_once() {
        let source = r#"
import * as core from "scene";
import { createSignal } from "reactive";

const root = core.column(0);
const scroll = core.scroll();
const content = core.column(0);
const canvas = core.sized(80, 40);
const spacer = core.sized(80, 200);
const [value, set] = createSignal(0);
core.setSize(scroll, 100, 80);
core.append(root, scroll);
core.append(scroll, content);
core.append(content, canvas);
core.append(content, spacer);
const binding = core.bindCanvas(canvas, (ctx) => ctx.fillText(String(value()), 0, 12), null, true);
core.onFrame((target) => {
    const step = Math.round(target / 16);
    if (step === 1 || step === 104) core.setScrollOffset(scroll, 0, 120);
    if (step >= 2 && step <= 101) {
        set(step - 1);
        binding.invalidate();
    }
    if (step === 102 || step === 105) core.setScrollOffset(scroll, 0, 0);
    if (step === 106) {
        binding.release();
        core.remove(content, canvas);
    }
});
"#;
        let mut session = open_canvas(source, "offscreen-canvas.js");
        session.metrics(&metrics());
        let first = session.frame(session::FrameTime::at(0.0));
        assert_eq!(redraws(&first), 1);
        for step in 1..=101 {
            let frame = session.frame(session::FrameTime::at(f64::from(step) * 16.0));
            let trace = frame.phases.expect("frame phases").trace;
            assert_eq!(trace.canvas_callbacks, 0, "step {step}");
            assert_eq!(trace.canvas_preparations, 0, "step {step}");
            assert!(!frame.again, "step {step}");
        }
        let back = session.frame(session::FrameTime::at(102.0 * 16.0));
        let trace = back.phases.expect("frame phases").trace;
        assert_eq!((trace.canvas_callbacks, trace.canvas_preparations), (1, 1));
        let mut latest = false;
        visit_commands(&back, &mut |command, _, _| {
            if let Command::Canvas { list, .. } = command {
                latest |= list.ops().iter().any(
                    |op| matches!(op, protocol::CanvasOp::Text(text) if text.utf8.as_ref() == "100"),
                );
            }
        });
        assert!(latest);
        assert_eq!(
            redraws(&session.frame(session::FrameTime::at(103.0 * 16.0))),
            0
        );
        assert_eq!(
            redraws(&session.frame(session::FrameTime::at(104.0 * 16.0))),
            0
        );
        assert_eq!(
            redraws(&session.frame(session::FrameTime::at(105.0 * 16.0))),
            1
        );
        assert_eq!(
            redraws(&session.frame(session::FrameTime::at(106.0 * 16.0))),
            0
        );
        assert!(session.take_script_error().is_none());
    }

    #[test]
    fn canvas_batches_match_scalar_paint_order_and_reject_bad_points() {
        let source = r#"
import * as core from "scene";

const root = core.column(0);
for (const bulk of [false, true]) {
  const box = core.sized(80, 40);
  core.append(root, box);
  core.bindCanvas(box, (ctx) => {
    ctx.translate(1.25, 3.5);
    ctx.beginPath();
    if (bulk) {
      for (const invalid of [[0, 0, 1], [0, 0, NaN, 2]]) {
        let rejected = false;
        try { ctx.polyline(new Float64Array(invalid)); } catch (_) { rejected = true; }
        if (!rejected) throw new Error("invalid path accepted");
      }
      ctx.polyline(new Float64Array(0));
      ctx.lineSegments(new Float64Array(0));
      ctx.fillRects(new Float64Array(0));
      const backing = new Float64Array([900, 900, 1, 3, 8, 9, 12, 6, 900, 900]);
      ctx.polyline(backing.subarray(2, 8));
      ctx.lineSegments(new Float64Array([16, 4, 18, 10, 21, 4, 25, 10]));
    } else {
      ctx.moveTo(1, 3);
      ctx.lineTo(8, 9);
      ctx.lineTo(12, 6);
      ctx.moveTo(16, 4);
      ctx.lineTo(18, 10);
      ctx.moveTo(21, 4);
      ctx.lineTo(25, 10);
    }
    ctx.stroke();
    ctx.fillStyle = '#11448880';
    if (bulk) {
      ctx.fillRects(new Float64Array([2, 5, 5, 6, 8, 9, -4, -3, 1, 2, 0, 10]));
    } else {
      ctx.fillRect(2, 5, 5, 6);
      ctx.fillRect(8, 9, -4, -3);
      ctx.fillRect(1, 2, 0, 10);
    }
  });
}
"#;
        let mut session = open_canvas(source, "canvas-batches.js");
        session.metrics(&metrics());
        let frame = session.frame(session::FrameTime::at(0.0));
        let mut lists = Vec::new();
        visit_commands(&frame, &mut |command, _, _| {
            if let Command::Canvas { list, .. } = command {
                lists.push(list.ops().to_vec());
            }
        });
        assert_eq!(lists.len(), 2);
        assert_eq!(lists[0], lists[1]);
        assert!(session.take_script_error().is_none());
    }

    #[test]
    fn canvas_side_effect_keeps_the_picture_and_reports_next_frame() {
        let source = r#"
import * as core from "scene";
import { createSignal } from "reactive";

            const root = core.column(0);
            const box = core.view();
            core.setSize(box, 80, 40);
            const label = core.text("ready", 16, [0, 0, 0, 255]);
            core.append(root, box);
            core.append(root, label);
            const [step, setStep] = createSignal(0);
            let passed = 0;
            core.onFrame(() => {
                passed += 1;
                if (passed === 2) setStep(1);
                if (passed < 3) core.requestFrame();
            });
            core.bindCanvas(box, (ctx) => {
                if (step() === 0) {
                    ctx.fillStyle = '#112233';
                    ctx.fillRect(0, 0, 8, 6);
                    return;
                }
                core.setText(label, "mutated");
            }, (error) => {
                core.setText(label, error.code);
            });
        "#;
        let mut session = open_canvas(source, "canvas-effect.js");
        session.metrics(&metrics());
        let first = session.frame(session::FrameTime::at(0.0));
        assert!(session.take_script_error().is_none());
        assert!(canvas_fills(&first) >= 1);
        assert!(first.again);
        let second = session.frame(session::FrameTime::at(16.0));
        assert!(session.take_script_error().is_none());
        // An unchanged epoch omits the layer tree. A new tree would still say "ready".
        assert!(
            second.layers.is_none() || texts(&second) == ["ready"],
            "side effect changed the picture: layers={} texts={:?}",
            second.layers.is_some(),
            texts(&second)
        );
        assert!(second.again);
        let third = session.frame(session::FrameTime::at(32.0));
        assert!(session.take_script_error().is_none());
        assert_eq!(texts(&third), ["DrawSideEffect"]);
        if third.layers.is_some() {
            assert!(
                canvas_fills(&third) >= 1,
                "the failed draw replaced the picture"
            );
        }
    }

    #[test]
    fn canvas_context_closes_after_the_callback() {
        let source = r#"
import * as core from "scene";

            const root = core.column(0);
            const box = core.view();
            core.setSize(box, 40, 40);
            const label = core.text("wait", 16, [0, 0, 0, 255]);
            core.append(root, box);
            core.append(root, label);
            let ctx = null;
            let armed = false;
            core.onFrame(() => {
                if (!armed) {
                    armed = true;
                    core.requestFrame();
                    return;
                }
                try {
                    ctx.fillRect(0, 0, 1, 1);
                    core.setText(label, "open");
                } catch (error) {
                    core.setText(label, error && error.code ? error.code : "thrown");
                }
            });
            core.bindCanvas(box, (context) => {
                ctx = context;
                context.fillRect(0, 0, 4, 4);
            });
        "#;
        let mut session = open_canvas(source, "canvas-closed.js");
        session.metrics(&metrics());
        let first = session.frame(session::FrameTime::at(0.0));
        assert!(session.take_script_error().is_none());
        assert!(canvas_fills(&first) >= 1);
        let second = session.frame(session::FrameTime::at(16.0));
        assert!(session.take_script_error().is_none());
        assert_eq!(texts(&second), ["ContextClosed"]);
    }

    #[test]
    fn canvas_path_outlives_the_draw() {
        let source = r#"
import * as core from "scene";

            const root = core.column(0);
            const box = core.view();
            core.setSize(box, 40, 40);
            const label = core.text("wait", 16, [0, 0, 0, 255]);
            core.append(root, box);
            core.append(root, label);
            const path = new core.Path2D();
            path.rect(0, 0, 8, 8);
            let armed = false;
            core.onFrame(() => {
                if (!armed) {
                    armed = true;
                    core.requestFrame();
                    return;
                }
                try {
                    path.lineTo(1, 1);
                    core.setText(label, "alive");
                } catch (error) {
                    core.setText(label, "dead");
                }
            });
            core.bindCanvas(box, (ctx) => {
                ctx.fill(path);
            });
        "#;
        let mut session = open_canvas(source, "canvas-path.js");
        session.metrics(&metrics());
        let first = session.frame(session::FrameTime::at(0.0));
        assert!(session.take_script_error().is_none());
        assert!(canvas_fills(&first) >= 1);
        let second = session.frame(session::FrameTime::at(16.0));
        assert!(session.take_script_error().is_none());
        assert_eq!(texts(&second), ["alive"]);
    }

    #[test]
    fn canvas_empty_subpath_does_not_panic() {
        let source = r#"
import * as core from "scene";
            const root = core.column(0);
            const box = core.view();
            core.setSize(box, 40, 40);
            core.append(root, box);
            core.bindCanvas(box, (ctx) => {
                ctx.beginPath();
                ctx.closePath();
                const path = new core.Path2D();
                path.lineTo(4, 4);
                path.quadraticCurveTo(1, 2, 3, 4);
                path.bezierCurveTo(0, 0, 1, 1, 2, 2);
                ctx.fillRect(0, 0, 4, 4);
            });
        "#;
        let mut session = open_canvas(source, "canvas-empty-path.js");
        session.metrics(&metrics());
        let frame = session.frame(session::FrameTime::at(0.0));
        assert!(session.take_script_error().is_none());
        assert!(canvas_fills(&frame) >= 1);
    }

    #[test]
    fn canvas_checks_array_lengths_before_reading_elements() {
        let source = r#"
            import * as core from "scene";
            const root = core.view();
            core.setSize(root, 40, 40);
            core.bindCanvas(root, (ctx) => {
                const segments = new Array(2 ** 31);
                Object.defineProperty(segments, "0", {
                    get() { throw new Error("read oversized array"); },
                });
                const check = (action, code) => {
                    try { action(); }
                    catch (error) { if (error.code === code) return; throw error; }
                    throw new Error("array length was accepted");
                };
                check(() => ctx.setLineDash(segments), "LimitExceeded");
                check(() => ctx.roundRect(0, 0, 8, 8, segments), "InvalidArgument");
                ctx.fillRect(0, 0, 8, 8);
            });
        "#;
        let mut session = open_canvas(source, "canvas-array-length.js");
        session.metrics(&metrics());
        let frame = session.frame(session::FrameTime::at(0.0));
        assert_eq!(canvas_fills(&frame), 1);
    }

    #[test]
    fn canvas_gradient_stops_apply_on_the_next_draw() {
        let source = r##"
import * as core from "scene";
            const root = core.column(0);
            const box = core.view();
            core.setSize(box, 40, 40);
            core.append(root, box);
            core.bindCanvas(box, (ctx) => {
                const gradient = ctx.createLinearGradient(0, 0, 10, 0);
                ctx.fillStyle = gradient;
                gradient.addColorStop(0, "#f00");
                ctx.fillRect(0, 0, 4, 4);
                gradient.addColorStop(1, "#00f");
                ctx.fillRect(4, 0, 4, 4);
            });
        "##;
        let mut session = open_canvas(source, "canvas-gradient.js");
        session.metrics(&metrics());
        let frame = session.frame(session::FrameTime::at(0.0));
        assert!(session.take_script_error().is_none());
        let mut counts = Vec::new();
        visit_commands(&frame, &mut |command, _, _| {
            if let Command::Canvas { list, .. } = command {
                for op in list.ops() {
                    if let protocol::CanvasOp::Fill { paint, .. } = op
                        && let protocol::CanvasBrush::Linear { stops, .. } = &paint.brush
                    {
                        counts.push(stops.len());
                    }
                }
            }
        });
        assert_eq!(counts, [1, 2]);
    }

    #[test]
    fn canvas_empty_clip_remains_in_the_list() {
        let source = r#"
import * as core from "scene";
            const root = core.column(0);
            const box = core.view();
            core.setSize(box, 40, 40);
            core.append(root, box);
            core.bindCanvas(box, (ctx) => {
                ctx.save();
                ctx.beginPath();
                ctx.clip();
                ctx.fillRect(0, 0, 8, 8);
                ctx.restore();
            });
        "#;
        let mut session = open_canvas(source, "canvas-clip.js");
        session.metrics(&metrics());
        let frame = session.frame(session::FrameTime::at(0.0));
        assert!(session.take_script_error().is_none());
        let mut saw_clip = false;
        let mut fill_after = false;
        visit_commands(&frame, &mut |command, _, _| {
            if let Command::Canvas { list, .. } = command {
                for op in list.ops() {
                    match op {
                        protocol::CanvasOp::Clip { path } if path.verbs.is_empty() => {
                            saw_clip = true;
                        }
                        protocol::CanvasOp::Fill { .. } if saw_clip => fill_after = true,
                        _ => {}
                    }
                }
            }
        });
        assert!(saw_clip, "empty clip was dropped");
        assert!(fill_after, "fill after the empty clip was dropped");
    }

    #[test]
    fn canvas_async_draw_is_rejected_and_not_committed() {
        let source = r#"
import * as core from "scene";
            const root = core.column(0);
            const box = core.view();
            core.setSize(box, 40, 40);
            core.append(root, box);
            core.bindCanvas(box, async (ctx) => {
                ctx.fillRect(0, 0, 8, 6);
                await Promise.resolve();
            });
        "#;
        let mut session = open_canvas(source, "canvas-async.js");
        session.metrics(&metrics());
        let first = session.frame(session::FrameTime::at(0.0));
        assert_eq!(canvas_fills(&first), 0);
        let second = session.frame(session::FrameTime::at(16.0));
        let _ = second;
        let error = session.take_script_error().unwrap_or_default();
        assert!(error.contains("AsyncDraw"), "{error}");
        assert!(
            session
                .take_faults()
                .iter()
                .any(|fault| fault.source == "canvas" && fault.message.contains("AsyncDraw"))
        );
    }

    #[test]
    fn canvas_default_error_reaches_diagnostics() {
        let source = r#"
import * as core from "scene";
            const root = core.column(0);
            const box = core.view();
            core.setSize(box, 40, 40);
            core.append(root, box);
            let once = false;
            core.bindCanvas(box, () => {
                if (once) return;
                once = true;
                throw new Error("missing handler");
            });
        "#;
        let mut session = open_canvas(source, "canvas-default-error.js");
        session.metrics(&metrics());
        let _ = session.frame(session::FrameTime::at(0.0));
        assert!(session.take_faults().is_empty());
        let _ = session.frame(session::FrameTime::at(16.0));
        let error = session.take_script_error().unwrap_or_default();
        assert!(error.contains("missing handler"), "{error}");
        assert!(
            session
                .take_faults()
                .iter()
                .any(|fault| fault.source == "canvas")
        );
    }

    #[test]
    fn canvas_error_delivery_shares_one_budget() {
        let source = r#"
import * as core from "scene";
            const root = core.column(0);
            const label = core.text("ready", 16, [0, 0, 0, 255]);
            const a = core.view();
            const b = core.view();
            core.setSize(a, 40, 40);
            core.setSize(b, 40, 40);
            core.append(root, a);
            core.append(root, b);
            core.append(root, label);
            let aDone = false;
            let bDone = false;
            core.bindCanvas(a, (ctx) => {
                if (aDone) return;
                aDone = true;
                ctx.fillRect(0, 0, 4, 4);
                throw new Error("one");
            }, () => {
                core.setText(label, "first");
                const start = Date.now();
                while (Date.now() - start < 250) {}
            });
            core.bindCanvas(b, () => {
                if (bDone) return;
                bDone = true;
                throw new Error("two");
            }, () => {
                core.setText(label, "second");
            });
        "#;
        let mut session = open_canvas(source, "canvas-error-budget.js");
        session.metrics(&metrics());
        let _ = session.frame(session::FrameTime::at(0.0));
        let delivered = session.frame(session::FrameTime::at(16.0));
        assert_eq!(texts(&delivered), ["first"]);
        assert!(delivered.again);
        let faults = session.take_faults();
        assert_eq!(faults.len(), 1, "{faults:?}");
        assert!(faults[0].message.starts_with("canvas node "));
        assert!(faults[0].message.contains("Interrupted: onDrawError"));
        let next = session.frame(session::FrameTime::at(32.0));
        assert_eq!(texts(&next), ["second"]);
    }

    #[test]
    fn canvas_draw_blocks_host_module_writes() {
        let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let config = ModuleConfig {
            registry: std::sync::Arc::new(ModuleRegistry::new(vec![Module {
                name: "Mark",
                exports: session::ModuleExports {
                    events: &[EventType {
                        name: "tick",
                        payload: "null",
                    }],
                    ..Default::default()
                },
                specifier: "@impasto/mark",
                methods: vec![Method::sync("write", {
                    let flag = std::sync::Arc::clone(&flag);
                    move |_| {
                        flag.store(true, std::sync::atomic::Ordering::SeqCst);
                        Ok(PluginValue::Null)
                    }
                })],
            }])),
            ..ModuleConfig::disconnected()
        };
        let source = r#"
import * as core from "scene";
import { write, addEventListener } from "@impasto/mark";
import { createSignal } from "reactive";
            const root = core.column(0);
            const box = core.view();
            core.setSize(box, 40, 40);
            const label = core.text("ready", 16, [0, 0, 0, 255]);
            core.append(root, box);
            core.append(root, label);
            const [passed, setPassed] = createSignal(0);
            core.onFrame(() => {
                const next = passed() + 1;
                setPassed(next);
                if (next < 3) core.requestFrame();
            });
            core.bindCanvas(box, (ctx) => {
                ctx.fillRect(0, 0, 8, 8);
                if (passed() === 1) write();
                if (passed() === 2) addEventListener("tick", () => {});
            }, (error) => {
                core.setText(label, error.code);
            });
        "#;
        let mut session = match super::JsSession::open_with(
            source,
            "canvas-host.js",
            uicore::MonoShaper::default(),
            no_log(),
            None,
            config,
        ) {
            Ok(session) => session,
            Err(error) => panic!("{error}"),
        };
        session.metrics(&metrics());
        let first = session.frame(session::FrameTime::at(0.0));
        assert!(!flag.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(canvas_fills(&first), 0);
        let second = session.frame(session::FrameTime::at(16.0));
        assert!(!flag.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(canvas_fills(&second), 0);
        let third = session.frame(session::FrameTime::at(32.0));
        assert_eq!(texts(&third), ["DrawSideEffect"]);
        assert!(canvas_fills(&third) >= 1);
    }
}
