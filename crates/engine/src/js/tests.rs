use crate::input::{Phase, PointerEvent};
use crate::scene::Command;
use crate::session;
use crate::text::test_support::mono_text;
use session::{
    EventType, ImmediateExec, InputEvent, IoEvent, Key, Method, Module, ModuleConfig,
    ModuleRegistry, PluginResult, PluginValue,
};
use session::{Metrics, Session};

use super::{Error, open};

const LINKING_EVENTS: &[EventType] = &[EventType {
    name: "url",
    payload: "{ url: string }",
}];

#[test]
fn native_style_contracts_and_single_view_button() {
    {
        let source = super::fixture_bundle("style.tsx");
        let mut session = open(
            &source,
            "style.js",
            crate::text::MonoShaper::default(),
            no_log(),
        )
        .expect("native style contracts");
        session.metrics(&metrics());
        session.frame(session::FrameTime::at(0.0));
        assert!(session.take_faults().is_empty());
    }
}

#[test]
fn native_component_contracts() {
    {
        let source = super::fixture_bundle("components.tsx");
        let mut session = open(
            &source,
            "components.js",
            crate::text::MonoShaper::default(),
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
fn native_gpu_components_record_owned_inputs_for_raster() {
    use crate::scene::BACKDROP_TEXTURE;

    let source = super::fixture_bundle("gpu-effects.tsx");
    let mut session = open(
        &source,
        "gpu-effects.js",
        crate::text::MonoShaper::default(),
        no_log(),
    )
    .expect("load GPU components");
    session.metrics(&metrics());
    let frame = session.frame(session::FrameTime::at(0.0));
    let faults = session.take_faults();
    assert!(faults.is_empty(), "{faults:?}");
    let tree = frame.layers.as_deref().expect("GPU component layer tree");
    assert!(tree.has_animated_effects());
    assert!(tree.has_gpu_views());
    let mut effects = 0;
    visit_commands(&frame, &mut |command, _, _| {
        if let Command::ShaderEffect {
            bounds,
            shader,
            uniforms,
            textures,
        } = command
        {
            let program = tree.shader_program(*shader).expect("registered shader");
            assert!(program.source.contains("effect_sample"));
            assert_eq!(bounds[2..], [72.0, 32.0]);
            assert_eq!(uniforms.as_ref(), [1, 2, 3, 4]);
            assert_eq!(textures[0].as_ref(), "asset:fixture");
            assert_eq!(textures[1].as_ref(), BACKDROP_TEXTURE);
            effects += 1;
        }
    });
    assert_eq!(effects, 1);
    let images = tree.image_requirements();
    assert!(
        images
            .iter()
            .any(|(source, _)| source.as_ref() == "asset:fixture")
    );
    assert!(
        images
            .iter()
            .any(|(source, _)| source.as_ref() == "external:gpuview:test-producer")
    );
    assert!(
        !images
            .iter()
            .any(|(source, _)| source.as_ref() == BACKDROP_TEXTURE)
    );

    click(session.as_mut(), 40.0, 20.0);
    let next = session.frame(session::FrameTime::at(16.0));
    let tree = next
        .layers
        .as_deref()
        .expect("updated GPU component layer tree");
    assert!(!tree.has_animated_effects());
    assert_eq!(
        tree.image_requirements(),
        [(
            std::sync::Arc::from("external:gpuview:next-producer"),
            false
        )]
    );
}

#[test]
fn native_control_flow_preserves_owned_rows_and_catches_effect_errors() {
    {
        let source = super::fixture_bundle("flow.ts");
        let mut session = open(
            &source,
            "flow.js",
            crate::text::MonoShaper::default(),
            no_log(),
        )
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
        let probe =
            include_str!("../../tests/fixtures/probe.mjs").replace("export function", "function");
        let source = format!(
            "import * as reactive from 'reactive';\n{probe}\nrunProbe(line=>console.log(line), {{...reactive, flush: fn=>reactive.batch(fn || (()=>{{}}))}});"
        );
        let lines = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let output = std::rc::Rc::clone(&lines);
        let mut session = open(
            &source,
            "reference.js",
            crate::text::MonoShaper::default(),
            Box::new(move |_, line| output.borrow_mut().push(line.to_owned())),
        )
        .expect("native reference");
        assert!(session.take_faults().is_empty());
        assert_eq!(
            lines.borrow().join("\n") + "\n",
            include_str!("../../tests/fixtures/order.txt")
        );
    }
}

#[test]
fn bytecode_round_trip_uses_the_native_modules() {
    {
        let source = "import {createSignal,batch} from 'reactive'; import {createElement,insert,setProp} from 'scene'; const [get,set]=createSignal('first'); const root=createElement('view',{style:{gap:8}}); insert(root,get); batch(()=>set('compiled')); setProp(root,'style',{gap:16});";
        let bytes = super::compile(source, "compiled.js", true).expect("compile module");
        let mut session = super::open_compiled(
            &bytes,
            "compiled.bin",
            crate::text::MonoShaper::default(),
            no_log(),
            ModuleConfig::disconnected(),
        )
        .expect("load bytecode");
        session.metrics(&metrics());
        let frame = session.frame(session::FrameTime::at(0.0));
        assert!(
            texts(&frame).iter().any(|text| text == "compiled"),
            "{:?}",
            texts(&frame)
        );
        assert!(session.take_faults().is_empty());
    }
}

#[test]
fn bytecode_links_registered_application_exports() {
    let source = r#"
        import { amount } from '@app/order/native';
        import { createElement, insert } from 'scene';
        const root = createElement('view', {});
        insert(root, amount());
    "#;
    let methods = &[("@app/order/native", &["amount"][..])];
    let bytecode = super::compile_with_native_exports(
        source,
        "app.js",
        true,
        ModuleRegistry::default(),
        methods,
    )
    .expect("compile against owned extension declarations");
    let module = Module {
        name: "Order",
        specifier: "@app/order/native",
        methods: vec![Method::sync("amount", |_| {
            Ok(PluginValue::Str("2599".into()))
        })],
        exports: Default::default(),
    };
    let config = ModuleConfig {
        registry: std::sync::Arc::new(
            ModuleRegistry::with_extensions(Vec::new(), vec![module]).expect("register order"),
        ),
        ..ModuleConfig::disconnected()
    }
    .restrict(&[]);
    let mut session = super::open_compiled(
        &bytecode,
        "app.bin",
        crate::text::MonoShaper::default(),
        no_log(),
        config,
    )
    .expect("load against the registered extension");
    session.metrics(&metrics());
    let frame = session.frame(session::FrameTime::at(0.0));
    assert!(texts(&frame).iter().any(|text| text == "2599"));
    assert!(session.take_faults().is_empty());
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
        crate::text::MonoShaper::default(),
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
            import * as exports from 'scene';
            const assert=(ok,label)=>{if(!ok)throw new Error(label);};
            for (const name of ['column', 'opacity', 'textWithRuns', 'setFontSize', 'setAxisSize', '_mutate']) {
                assert(!(name in scene) && !(name in exports), 'obsolete ABI still exposes ' + name);
            }
            const root=scene.createElement('view');
            const [text,setText]=createSignal('first');
            scene.insert(root,text);
            const id=scene.firstChild(root);
            batch(()=>setText('second'));
            assert(scene.firstChild(root)===id,'text node replaced');
            for(const [name,value] of [['width','invalid'],['height',Infinity],['left','bad']]) {
                let caught=false;
                try{scene.setProp(root,"style",{[name]:value});}catch{caught=true;}
                assert(caught,'invalid '+name+' accepted');
            }
            const removed=scene.createElement('view');scene.discard(removed);
            let caught=false;
            try{scene.insertNode(root,removed);}catch{caught=true;}
            assert(caught,'stale node accepted');
        "#;
    match open(
        source,
        "native-insert.js",
        crate::text::MonoShaper::default(),
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
            const root=scene.createElement('view'),region=scene.createElement('view');
            scene.append(region,scene.createElement('text', { value: 'Run', style: { fontSize: 16, color: [255,255,255,255] } }));
            scene.append(root,region);
            scene.onClick(region,()=>write(true));
        "#;
    let mut session = match open(
        source,
        "reactive-budget.js",
        crate::text::MonoShaper::default(),
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
            const root = scene.createElement('view');
            const label = scene.createElement('text', { value: "initial", style: { fontSize: 16, color: [255, 255, 255, 255] } });
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
        crate::text::MonoShaper::default(),
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
        crate::text::MonoShaper::default(),
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
    match open(
        source,
        "memo.js",
        crate::text::MonoShaper::default(),
        no_log(),
    ) {
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
    match open(
        source,
        "order.js",
        crate::text::MonoShaper::default(),
        no_log(),
    ) {
        Ok(_) => {}
        Err(error) => panic!("{error}"),
    }
}

#[test]
fn scene_module_creates_and_styles_a_node() {
    let source = r#"
          import * as scene from "scene";
          if (!scene || typeof scene.createElement !== "function") {
            throw new Error("scene module missing");
          }
          var id = scene.createElement('view', { style: { gap: 8 } });
          if (!(id > 0)) {
            throw new Error("column id " + id);
          }
          scene.setProp(id, "style", { gap: 16 });
          var child = scene.createElement('text', { value: "hi", style: { fontSize: 14, color: [0, 0, 0, 255] } });
          scene.insert(id, child, 0);
        "#;
    match open(
        source,
        "scene.js",
        crate::text::MonoShaper::default(),
        no_log(),
    ) {
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
    match open(
        source,
        "driver.js",
        crate::text::MonoShaper::default(),
        no_log(),
    ) {
        Ok(_) => {}
        Err(error) => panic!("{error}"),
    }
}

#[test]
fn drop_releases_unfired_timer() {
    let session = open(
            "const captured = { text: 'held' }; setTimeout(() => console.log(captured.text), 60000); setInterval(() => {}, 60000); requestAnimationFrame(() => {});",
            "drop-timer.js", crate::text::MonoShaper::default(), no_log(),
        ).expect("open timers");
    drop(session);
}

#[test]
fn drop_releases_reactive_cleanup() {
    let seen = std::rc::Rc::new(std::cell::Cell::new(0));
    let record = std::rc::Rc::clone(&seen);
    let session = super::open_program(
        br#"
                import {createRoot,createSignal,onCleanup} from 'reactive';
                createRoot(() => onCleanup(() => {
                    console.log('cleanup');
                    setTimeout(() => {}, 60000);
                    createRoot(() => {
                        createSignal({held: true});
                        onCleanup(() => console.log('nested cleanup'));
                    });
                }));
            "#,
        "drop-cleanup.js",
        crate::text::MonoShaper::default(),
        Box::new(move |_, text| {
            if text == "cleanup" || text == "nested cleanup" {
                record.set(record.get() + 1);
            }
        }),
        ModuleConfig::disconnected(),
    )
    .expect("open cleanup");
    drop(session);
    assert_eq!(seen.get(), 2);
}

#[test]
fn failed_open_releases_standard_and_reactive_bindings() {
    let seen = std::rc::Rc::new(std::cell::Cell::new(0));
    let record = std::rc::Rc::clone(&seen);
    let result = open(
        "import {createRoot,onCleanup} from 'reactive'; if (typeof URL !== 'function' || typeof AbortController !== 'function') throw new Error('standard bindings missing'); createRoot(() => onCleanup(() => console.log('cleanup'))); setTimeout(() => {}, 60000); throw new Error('failed during entry');",
        "failed-open.js",
        crate::text::MonoShaper::default(),
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

/// Application startup microtasks run during `open`; timers and rAF wait for a frame.
#[test]
fn open_runs_entry_before_frame_flushes_timers() {
    let logs = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let record = std::rc::Rc::clone(&logs);
    let source = r#"
            console.log("sync");
            Promise.resolve().then(function () { console.log("job"); });
            setTimeout(function () { console.log("timer"); }, 0);
            requestAnimationFrame(function () { console.log("raf"); });
            console.log("sync-after");
        "#;
    let mut session = super::open_with_config(
        source,
        "order.js",
        crate::text::MonoShaper::default(),
        Box::new(move |_level, text| record.borrow_mut().push(text.to_owned())),
        ModuleConfig::disconnected(),
    )
    .expect("open");
    assert_eq!(
        logs.borrow().as_slice(),
        ["sync", "sync-after", "job"],
        "entry microtask runs during open; timers and raf wait for a frame"
    );
    session.frame(session::FrameTime::at(0.0));
    assert_eq!(
        logs.borrow().as_slice(),
        ["sync", "sync-after", "job", "timer", "raf"],
    );
    drop(session);
}

#[test]
fn module_commands_and_microtasks_keep_the_previous_frame_clock() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicI64, Ordering};

    let logs = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let record = std::rc::Rc::clone(&logs);
    let (send, completions) = std::sync::mpsc::channel();
    let clock = Arc::new(AtomicI64::new(100));
    let host_clock = Arc::clone(&clock);
    let config = ModuleConfig {
        registry: Arc::new(ModuleRegistry::new(vec![Module {
            name: "Draft",
            specifier: "@impasto/draft",
            exports: session::ModuleExports {
                events: &[EventType {
                    name: "command",
                    payload: "null",
                }],
                ..session::ModuleExports::default()
            },
            methods: vec![Method::promise("save", |_, settle, exec| {
                exec.now(settle, PluginResult::Done(PluginValue::Null));
            })],
        }])),
        transport_clock: Some(Arc::new(move || host_clock.load(Ordering::Relaxed))),
        script_execution: Default::default(),
        post_delivery: Some(Arc::new(move |delivery| {
            send.send(delivery).expect("post save completion");
        })),
        ..ModuleConfig::disconnected()
    };
    let mut session = super::open_with_config(
        r#"
                import {save, addEventListener} from '@impasto/draft';
                console.log("entry:" + performance.now());
                addEventListener("command", () => {
                    console.log("command:" + performance.now());
                    Promise.resolve().then(() => console.log("job:" + performance.now()));
                    setTimeout(() => console.log("timer:" + performance.now()), 0);
                    save().then(() => console.log("saved"), error => console.log(error.message));
                });
            "#,
        "module-clock.js",
        crate::text::MonoShaper::default(),
        Box::new(move |_level, text| record.borrow_mut().push(text.to_owned())),
        config,
    )
    .expect("open");

    session.frame(session::FrameTime::at(100.0));
    clock.store(60_100, Ordering::Relaxed);
    session.io_event(IoEvent::Host(javascript::host::HostEvent {
        module: "Draft".into(),
        event: "command".into(),
        payload: PluginValue::Null,
    }));
    assert_eq!(logs.borrow().as_slice(), ["entry:0", "command:0"]);
    session.frame(session::FrameTime::at(60_100.0));
    session.io_delivery(completions.try_recv().expect("save completed"));
    session.frame(session::FrameTime::at(60_116.0));
    assert_eq!(
        logs.borrow().as_slice(),
        ["entry:0", "command:0", "job:0", "timer:60000", "saved"]
    );
}

#[test]
fn routed_completion_requires_the_live_generation() {
    use std::sync::Arc;

    let logs = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let record = std::rc::Rc::clone(&logs);
    let (send, completions) = std::sync::mpsc::channel();
    let config = ModuleConfig {
        registry: Arc::new(ModuleRegistry::new(vec![Module {
            name: "Probe",
            specifier: "@impasto/probe",
            exports: Default::default(),
            methods: vec![Method::promise("ping", |_, settle, _| {
                settle.send(PluginResult::Done(PluginValue::Str("ok".into())));
            })],
        }])),
        post_delivery: Some(Arc::new(move |delivery| {
            send.send(delivery).expect("post delivery");
        })),
        ..ModuleConfig::disconnected()
    };
    let mut session = super::open_program(
        br#"
                import { ping } from "@impasto/probe";
                ping().then(value => console.log("opened:" + value));
            "#,
        "routed-plugin.js",
        crate::text::MonoShaper::default(),
        Box::new(move |_level, text| record.borrow_mut().push(text.to_owned())),
        config,
    )
    .expect("open");
    let valid = completions.try_recv().expect("posted completion");
    let wrong_gen = session::Delivery {
        runtime: session::RuntimeKey {
            instance: valid.runtime.instance,
            generation: valid.runtime.generation + 1,
        },
        event: IoEvent::Plugin {
            token: valid.event.id().expect("token"),
            result: PluginResult::Done(PluginValue::Str("stale".into())),
        },
        credit: None,
    };
    session.io_delivery(wrong_gen);
    session.frame(session::FrameTime::at(0.0));
    assert!(logs.borrow().is_empty());
    session.io_delivery(valid);
    session.frame(session::FrameTime::at(16.0));
    assert_eq!(logs.borrow().as_slice(), ["opened:ok"]);
}

#[test]
fn module_listener_mutations_take_effect_after_the_snapshot() {
    let logs = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let record = std::rc::Rc::clone(&logs);
    let (host, _pending) = test_host();
    let mut session = super::JsSession::open_with(
        r#"
                import { addEventListener, removeEventListener } from "@impasto/linking/native";
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
        crate::text::MonoShaper::default(),
        Box::new(move |_level, text| record.borrow_mut().push(text.to_owned())),
        host,
    )
    .expect("open");
    session.io_event(IoEvent::Host(javascript::host::HostEvent {
        module: "Linking".into(),
        event: "url".into(),
        payload: PluginValue::Map(vec![(
            "url".into(),
            PluginValue::Str("impasto://first".into()),
        )]),
    }));
    session.io_event(IoEvent::Host(javascript::host::HostEvent {
        module: "Linking".into(),
        event: "url".into(),
        payload: PluginValue::Map(vec![(
            "url".into(),
            PluginValue::Str("impasto://second".into()),
        )]),
    }));
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
fn unknown_io_completion_is_ignored_and_drop_discards_pending_promise() {
    let logs = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let record = std::rc::Rc::clone(&logs);
    let (host, _pending) = test_host();
    let mut session = super::JsSession::open_with(
        r#"
            import { later } from "@impasto/echo";
            later("pending").then(v => console.log("ok:" + v), err => console.log("fail:" + err));
        "#,
        "pending-promise.js",
        crate::text::MonoShaper::default(),
        Box::new(move |_level, text| record.borrow_mut().push(text.to_owned())),
        host,
    )
    .expect("open");
    assert!(logs.borrow().is_empty());
    session.io_event(IoEvent::Plugin {
        token: 99,
        result: PluginResult::Failed {
            message: "late".into(),
        },
    });
    session.io_event(IoEvent::NativeStream {
        id: 99,
        event: javascript::host::StreamEvent::Failed("late".into()),
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
            crate::text::MonoShaper::default(),
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
    let _session = super::open_with_config(
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
        crate::text::MonoShaper::default(),
        Box::new(move |_level, text| record.borrow_mut().push(text.to_owned())),
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
    let session = super::open_compiled(
        &bytes,
        "script.bin",
        crate::text::MonoShaper::default(),
        Box::new(move |_, text| record.borrow_mut().push_str(text)),
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
    assert!(
        open(
            source,
            "memory.js",
            crate::text::MonoShaper::default(),
            no_log()
        )
        .is_ok()
    );
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
            crate::text::MonoShaper::default(),
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
            import {createSignal} from 'reactive';
            const [read, write] = createSignal(0);
            if (echo({ready: true}).ready !== true) throw new Error('module object');
            write(1);
            if (read() !== 1) throw new Error('signal after module call');
            const source = new Int16Array([1, 258, -3]);
            const view = source.subarray(1, 2);
            const bytes = new Uint8Array(view.buffer, view.byteOffset, view.byteLength);
            const returned = echo(view);
            if (!(returned instanceof Uint8Array) || returned.length !== bytes.length || returned.some((b,i) => b !== bytes[i])) throw new Error('typed view bytes');
            const clamped = echo(new Uint8ClampedArray([300, -1]));
            if (clamped[0] !== 255 || clamped[1] !== 0) throw new Error('clamped bytes');
            const large = echo(new Uint8Array(4096).fill(7));
            if (!(large instanceof ArrayBuffer) || new Uint8Array(large)[4095] !== 7) throw new Error('owned buffer');
            write(2);
            if (read() !== 2) throw new Error('signal after byte conversion');
        "#;
    drop(
        super::JsSession::open_with(
            source,
            "bytes.js",
            crate::text::MonoShaper::default(),
            no_log(),
            config,
        )
        .expect("module bytes"),
    );
}

#[test]
fn the_same_host_binding_script_runs_in_headless_and_ui() {
    use javascript::runtime::ModuleOutcome;
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
        IoEvent::Host(javascript::host::HostEvent {
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
    let catalog = javascript::runtime::ModuleCatalog::new(
        ModuleRegistry::try_new(vec![module.clone()]).expect("registry"),
    )
    .expect("catalog");
    let headless = Rc::new(RefCell::new(Vec::new()));
    let record = Rc::clone(&headless);
    let mut runtime = javascript::runtime::Runtime::builder()
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
        crate::text::MonoShaper::default(),
        Box::new(move |_, line| record.borrow_mut().push(line.to_owned())),
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
        crate::text::MonoShaper::default(),
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
        crate::text::MonoShaper::default(),
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
        asset_reader: None,
        registry: Arc::new(ModuleRegistry::new(vec![
            Module {
                name: "Clipboard",
                exports: session::ModuleExports {
                    object: true,
                    ..Default::default()
                },
                specifier: "@impasto/clipboard/native",
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
                specifier: "@impasto/linking/native",
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
        transport_clock: None,
        script_execution: Default::default(),
    };
    (config, pending)
}

fn drain_plugin(session: &mut dyn Session, pending: &std::sync::Mutex<Vec<(u64, PluginResult)>>) {
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

#[test]
fn window_metrics_notify_once_per_distinct_viewport() {
    let logs = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let collected = std::rc::Rc::clone(&logs);
    let mut session = open(
        r#"
            import { windowMetrics, onWindowMetrics, offWindowMetrics } from "scene";
            const listener = onWindowMetrics((window) => {
                const current = windowMetrics();
                console.log([current.width, window.pixelWidth, window.padding.bottom,
                  window.viewInsets.bottom, window.systemGestureInsets.left].join(","));
                if (window.viewInsets.bottom > 0) offWindowMetrics(listener);
            });
        "#,
        "window-metrics.js",
        crate::text::MonoShaper::default(),
        Box::new(move |_, line| collected.borrow_mut().push(line.to_owned())),
    )
    .expect("open");
    let padding = session::Insets {
        bottom: 20.0,
        ..Default::default()
    };
    let mut viewport = Metrics::from_insets(360.0, 780.0, 3.0, padding, Default::default());
    viewport.system_gesture_insets.left = 16.0;
    session.metrics(&viewport);
    session.metrics(&viewport);
    viewport = Metrics::from_insets(
        360.0,
        780.0,
        3.0,
        padding,
        session::Insets {
            bottom: 250.0,
            ..Default::default()
        },
    );
    viewport.system_gesture_insets.left = 16.0;
    session.metrics(&viewport);
    session.metrics(&phone());
    assert_eq!(
        logs.borrow().as_slice(),
        ["360,1080,20,0,16", "360,1080,20,250,16"]
    );
}

fn visit_commands<'a>(
    frame: &'a session::FrameOutput,
    visit: &mut dyn FnMut(&'a Command, f32, f32),
) {
    let Some(tree) = frame.layers.as_deref() else {
        return;
    };
    fn walk<'a>(
        tree: &'a crate::scene::LayerTree,
        id: crate::scene::LayerId,
        tx: f32,
        ty: f32,
        visit: &mut dyn FnMut(&'a Command, f32, f32),
    ) {
        let Some(layer) = tree.layers.get(id.0 as usize) else {
            return;
        };
        let (mut next_x, mut next_y) = (tx, ty);
        match layer.kind {
            crate::scene::LayerKind::Offset { x, y } => {
                next_x += x;
                next_y += y;
            }
            crate::scene::LayerKind::Transform {
                translate_x,
                translate_y,
                ..
            } => {
                next_x += translate_x;
                next_y += translate_y;
            }
            crate::scene::LayerKind::Picture { picture } => {
                if let Some(picture) = tree.picture(picture) {
                    let origin_x = next_x + picture.origin.0;
                    let origin_y = next_y + picture.origin.1;
                    for command in picture.commands().iter() {
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

fn texts(frame: &session::FrameOutput) -> Vec<String> {
    let mut found = Vec::new();
    visit_commands(frame, &mut |command, _, _| {
        if let Command::Text {
            shaped: Some(shaped),
            ..
        } = command
        {
            found.push(mono_text(shaped));
        }
    });
    found
}

fn text_spot(frame: &session::FrameOutput, utf8: &str) -> (f32, f32) {
    let mut found = None;
    visit_commands(frame, &mut |command, origin_x, origin_y| {
        if let Command::Text {
            shaped: Some(shaped),
            x,
            y,
            ..
        } = command
            && mono_text(shaped) == utf8
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

/// Center of the first button whose accessible label matches exactly.
/// Icon controls have no text to spot, so the tests reach them by label.
fn label_spot(session: &mut super::JsSession, label: &str) -> Option<(f32, f32)> {
    let update = session.semantics()?;
    update
        .nodes
        .iter()
        .find(|node| node.role == crate::semantics::SemanticsRole::Button && node.label == label)
        .map(|node| {
            (
                node.rect.x + node.rect.w / 2.0,
                node.rect.y + node.rect.h / 2.0,
            )
        })
}

fn click(session: &mut dyn Session, x: f32, y: f32) {
    session.pointer(PointerEvent::new(Phase::Down, x, y, 0.0));
    session.pointer(PointerEvent::new(Phase::Up, x, y, 0.0));
}

#[test]
fn remove_discards_the_tap_and_the_id() {
    {
        let kept = r#"
import * as core from "scene";

                const root = core.createElement('view');
                const region = core.createElement('view');
                const label = core.createElement('text', { value: "Go", style: { fontSize: 16, color: [255, 255, 255, 255] } });
                core.append(region, label);
                core.append(root, region);
                core.onClick(region, () => {});
                core.remove(root, region);
            "#;
        let session = match super::JsSession::open(
            kept,
            "test.js",
            crate::text::MonoShaper::default(),
            no_log(),
        ) {
            Ok(session) => session,
            Err(error) => panic!("{error}"),
        };
        assert_eq!(session.tap_count(), 0);
        let dropped = r#"
import * as core from "scene";

                const root = core.createElement('view');
                const label = core.createElement('text', { value: "Go", style: { fontSize: 16, color: [255, 255, 255, 255] } });
                core.append(root, label);
                core.remove(root, label);
                core.setText(label, "still");
                core.append(root, label);
            "#;
        match super::JsSession::open(
            dropped,
            "test.js",
            crate::text::MonoShaper::default(),
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

const root = scene.createElement('view');
const surviving = scene.createElement('view');
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
        crate::text::MonoShaper::default(),
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

                const root = core.createElement('view');
                const region = core.createElement('view');
                core.append(root, region);
                core.onClick(region, () => {});
                core.detach(root, region);
            "#;
        let mut session = match super::JsSession::open(
            kept,
            "test.js",
            crate::text::MonoShaper::default(),
            no_log(),
        ) {
            Ok(session) => session,
            Err(error) => panic!("{error}"),
        };
        assert_eq!(session.tap_count(), 1);
        let dropped = r#"
import * as core from "scene";

                const root = core.createElement('view');
                const region = core.createElement('view');
                core.append(root, region);
                core.onClick(region, () => {});
                core.detach(root, region);
                core.discard(region);
            "#;
        session = match super::JsSession::open(
            dropped,
            "test.js",
            crate::text::MonoShaper::default(),
            no_log(),
        ) {
            Ok(session) => session,
            Err(error) => panic!("{error}"),
        };
        assert_eq!(session.tap_count(), 0);
    }
}

#[test]
fn press_then_release_then_click_rewrites_the_label() {
    {
        let source = r#"
import * as core from "scene";

                let log = "";
                const root = core.createElement('view');
                const region = core.createElement('view');
                const label = core.createElement('text', { value: "idle", style: { fontSize: 16, color: [0, 0, 0, 255] } });
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
            crate::text::MonoShaper::default(),
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

                const label = core.createElement('text', { value: "Hi", style: { fontSize: 16, color: [0, 0, 0, 255] } });
            "#;
        let mut session = match super::JsSession::open(
            source,
            "test.js",
            crate::text::MonoShaper::default(),
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
    match open("{", "test.js", crate::text::MonoShaper::default(), no_log()) {
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

                const root = core.createElement('view');
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
            crate::text::MonoShaper::default(),
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

                const root = core.createElement('view');
                const region = core.createElement('view');
                const label = core.createElement('text', { value: "Go", style: { fontSize: 16, color: [255, 255, 255, 255] } });
                core.append(region, label);
                core.append(root, region);
                core.setListenerErrorPolicy("freeze");
                core.onClick(region, () => { throw new Error("boom"); });
            "#;
        let mut session = match super::JsSession::open(
            source,
            "test.js",
            crate::text::MonoShaper::default(),
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
        assert!(texts(&first).iter().any(|text| text == "Go"));
    }
}

#[test]
fn a_reported_listener_error_lets_the_next_listener_run() {
    {
        let source = r#"
import * as core from "scene";

                const root = core.createElement('view');
                const region = core.createElement('view');
                const label = core.createElement('text', { value: "Go", style: { fontSize: 16, color: [255, 255, 255, 255] } });
                core.append(region, label);
                core.append(root, region);
                core.onClick(region, () => { throw new Error("boom"); });
                core.onClick(region, () => core.setText(label, "later"));
            "#;
        let mut session = match super::JsSession::open(
            source,
            "report.js",
            crate::text::MonoShaper::default(),
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
        assert!(
            texts(&second).iter().any(|text| text == "later"),
            "{:?}",
            texts(&second)
        );
    }
}

#[test]
fn a_shortcut_consumes_the_modified_key() {
    {
        let source = r#"
import * as core from "scene";

                const root = core.createElement('view');
                const label = core.createElement('text', { value: "none", style: { fontSize: 16, color: [0, 0, 0, 255] } });
                const field = core.createElement('view');
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
            crate::text::MonoShaper::default(),
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
            modifiers: crate::input::Modifiers::empty().with(crate::input::Modifiers::META),
        }));
        let frame = session.frame(session::FrameTime::at(16.0));
        assert!(
            texts(&frame).iter().any(|text| text == "cut"),
            "{:?}",
            texts(&frame)
        );
    }
}

#[test]
fn host_motion_and_semantics_hints_reach_core() {
    {
        let source = r#"
import * as core from "scene";

                const root = core.createElement('view');
                const scroll = core.createElement('scroll');
                const label = core.createElement('text', { value: "row", style: { fontSize: 16, color: [0, 0, 0, 255] } });
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
            crate::text::MonoShaper::default(),
            no_log(),
        ) {
            Ok(session) => session,
            Err(error) => panic!("{error}"),
        };
        session.metrics(&metrics());
        let frame = session.frame(session::FrameTime::at(0.0));
        assert!(session.take_script_error().is_none());
        assert!(
            texts(&frame).iter().any(|text| text == "row"),
            "{:?}",
            texts(&frame)
        );
        let later = session.frame(session::FrameTime::at(48.0));
        assert!(later.layers.is_some() || frame.layers.is_some());
    }
}

#[test]
fn motion_end_survives_unregistering_its_own_callback() {
    {
        let source = r#"
import * as core from "scene";

                const root = core.createElement('view');
                const fading = core.createElement('stack', { style: { opacity: 255/255 } });
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
            crate::text::MonoShaper::default(),
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

                const root = core.createElement('view');
                const fading = core.createElement('stack', { style: { opacity: 255/255 } });
                const label = core.createElement('text', { value: "idle", style: { fontSize: 16, color: [0, 0, 0, 255] } });
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
            crate::text::MonoShaper::default(),
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

                const frame = core.createElement('view', { style: { alignItems: "center", justifyContent: "center" } });
                core.patchStyle(frame, { alignItems: "end", justifyContent: "end" });
                const column = core.createElement('view');
                core.patchStyle(column, { alignItems: "center", justifyContent: "center" });
                core.patchStyle(column, { alignItems: "center" });
                let stackRejected = false;
                try {
                    core.patchStyle(core.createElement('stack'), { alignItems: "invalid" });
                } catch (error) {
                    stackRejected = true;
                }
                if (!stackRejected) {
                    throw new Error("style accepted invalid alignment");
                }
                const label = core.createElement('text', { value: "idle", style: { fontSize: 16, color: [0, 0, 0, 255] } });
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
            crate::text::MonoShaper::default(),
            no_log(),
        ) {
            Ok(session) => session,
            Err(error) => panic!("{error}"),
        };
        session.metrics(&metrics());
        let frame = session.frame(session::FrameTime::at(0.0));
        assert!(session.take_script_error().is_none());
        assert!(
            texts(&frame).iter().any(|text| text == "w:320"),
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

                const root = core.createElement('view');
                const region = core.createElement('view');
                const label = core.createElement('text', { value: "Go", style: { fontSize: 16, color: [255, 255, 255, 255] } });
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
            crate::text::MonoShaper::default(),
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
        assert!(texts(&second).iter().any(|text| text == "Done"));
    }
}

#[test]
fn a_runaway_click_is_interrupted() {
    {
        let source = r#"
import * as core from "scene";

                const root = core.createElement('view');
                const region = core.createElement('view');
                const label = core.createElement('text', { value: "Go", style: { fontSize: 16, color: [255, 255, 255, 255] } });
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
            crate::text::MonoShaper::default(),
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
        assert!(texts(&first).iter().any(|text| text == "Go"));
    }
}

#[test]
fn a_runaway_timer_names_its_entry() {
    {
        let source = r#"
import * as core from "scene";

                const label = core.createElement('text', { value: "idle", style: { fontSize: 16, color: [0, 0, 0, 255] } });
                setTimeout(() => {
                    const start = Date.now();
                    while (Date.now() - start < 400) {}
                }, 0);
            "#;
        let mut session = match super::JsSession::open(
            source,
            "timer.js",
            crate::text::MonoShaper::default(),
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
            crate::text::MonoShaper::default(),
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
            crate::text::MonoShaper::default(),
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
            crate::text::MonoShaper::default(),
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
            crate::text::MonoShaper::default(),
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
            crate::text::MonoShaper::default(),
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

                const root = core.createElement('view');
                core.patchStyle(root, { width: 40, height: 20 });
                const a = core.createElement('text', { value: "hello", style: { fontSize: 16, color: [0, 0, 0, 255] } });
                const b = core.createElement('text', { value: "row", style: { fontSize: 16, color: [0, 0, 0, 255] } });
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
            crate::text::MonoShaper::default(),
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

                const label = core.createElement('text', { value: "0", style: { fontSize: 16, color: [0, 0, 0, 255] } });
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
            crate::text::MonoShaper::default(),
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

                const root = core.createElement('view');
                const fading = core.createElement('stack', { style: { opacity: 255/255 } });
                core.append(root, fading);
                core.animate(fading, "opacity", 255, 0, 170, 26);
            "#;
        let mut session = match super::JsSession::open(
            source,
            "spring.js",
            crate::text::MonoShaper::default(),
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

                const label = core.createElement('text', { value: "old", style: { fontSize: 16, color: [0, 0, 0, 255] } });
                const region = core.createElement('view');
                core.append(region, label);
                core.onClick(region, () => {
                    queueMicrotask(() => core.setText(label, "new"));
                });
            "#;
        let mut session = match super::JsSession::open(
            source,
            "test.js",
            crate::text::MonoShaper::default(),
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
        assert!(
            texts(&second).iter().any(|text| text == "new"),
            "{:?}",
            texts(&second)
        );
        assert!(!session.wants_frame());
    }
}

#[test]
fn a_throwing_frame_callback_does_not_ask_for_another_frame() {
    {
        let source = r#"
import * as core from "scene";

                const label = core.createElement('text', { value: "stay", style: { fontSize: 16, color: [0, 0, 0, 255] } });
                core.onFrame(() => {
                    core.requestFrame();
                    core.setText(label, "nope");
                    throw new Error("boom");
                });
            "#;
        let mut session = match super::JsSession::open(
            source,
            "test.js",
            crate::text::MonoShaper::default(),
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
                const root = core.createElement('view');
                core.patchStyle(root, { width: '100%', height: '100%', flexGrow: 1, flexShrink: 1 });
                const scroll = core.createElement('scroll');
                const region = core.createElement('view');
                const label = core.createElement('text', { value: "idle", style: { fontSize: 16, color: [0, 0, 0, 255] } });
                core.append(region, label);
                core.append(scroll, region);
                const tall = core.createElement('view', { style: { width: 20, height: 400 } });
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
            crate::text::MonoShaper::default(),
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

                const root = core.createElement('view');
                core.patchStyle(root, { width: '100%', height: '100%', flexGrow: 1, flexShrink: 1 });
                const label = core.createElement('text', { value: "idle", style: { fontSize: 16, color: [0, 0, 0, 255] } });
                core.append(root, label);
                const scroll = core.createElement('scroll');
                core.patchStyle(scroll, { flexGrow: 1 });
                core.append(scroll, core.createElement('view', { style: { width: 80, height: 800 } }));
                core.append(root, scroll);
                core.onScrollState((_, phase) => core.setText(label, String(phase)));
                core.onDragBy(scroll, () => core.setScrollOffset(scroll, 0, -48));
                core.onDragEnd(scroll, () => core.stopScroll(scroll));
            "#;
        let mut session = match super::JsSession::open(
            source,
            "test.js",
            crate::text::MonoShaper::default(),
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

                const root = core.createElement('view');
                core.patchStyle(root, { width: '100%', height: '100%', flexGrow: 1, flexShrink: 1 });
                const label = core.createElement('text', { value: "x", style: { fontSize: 16, color: [0, 0, 0, 255] } });
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
            crate::text::MonoShaper::default(),
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
            let Command::Text {
                shaped: Some(shaped),
                span_paints,
                ..
            } = command
            else {
                return;
            };
            if mono_text(shaped) != "abc" {
                return;
            }
            found = matches!(
                span_paints.as_ref(),
                [
                    crate::scene::SpanPaint::Color([1, 2, 3, 255]),
                    crate::scene::SpanPaint::Gradient(_),
                ]
            );
        });
        assert!(found, "styled runs were not recorded");
    }
}

#[test]
fn module_loader_accepts_a_sibling_script_and_rejects_escape() {
    {
        let dir = std::env::temp_dir().join(format!("quickjs-mod-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp app dir");
        std::fs::write(dir.join("util.js"), "export const label = \"from-util\";\n").expect("util");
        let app = dir.join("app.js");
        std::fs::write(
                &app,
                "import * as core from 'scene'; import { label } from \"./util.js\";\ncore.createElement('text', {value: label, style: {color: [0, 0, 0, 255]}});\n",
            )
            .expect("app");
        let name = app.to_string_lossy().into_owned();
        let source = std::fs::read_to_string(&app).expect("read app");
        let mut session = match super::JsSession::open(
            &source,
            &name,
            crate::text::MonoShaper::default(),
            no_log(),
        ) {
            Ok(session) => session,
            Err(error) => panic!("{error}"),
        };
        session.metrics(&metrics());
        let frame = session.frame(session::FrameTime::at(0.0));
        assert!(
            texts(&frame).iter().any(|text| text == "from-util"),
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
            match super::JsSession::open(
                source,
                &name,
                crate::text::MonoShaper::default(),
                no_log(),
            ) {
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
        crate::text::MonoShaper::default(),
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
            "import * as core from 'scene'; import { label } from \"./panels/page.js\";\ncore.createElement('text', {value: label, style: {color: [0, 0, 0, 255]}});\n",
        )
        .expect("app");
    let name = app.to_string_lossy().into_owned();
    let source = std::fs::read_to_string(&app).expect("read app");
    let mut session = match super::JsSession::open(
        &source,
        &name,
        crate::text::MonoShaper::default(),
        no_log(),
    ) {
        Ok(session) => session,
        Err(error) => panic!("{error}"),
    };
    session.metrics(&metrics());
    let frame = session.frame(session::FrameTime::at(0.0));
    assert!(
        texts(&frame).iter().any(|text| text == "shared-ok"),
        "{:?} {}",
        texts(&frame),
        session.take_script_error().unwrap_or_default()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn text_input_follows_focus_and_clears_when_the_node_is_removed() {
    {
        let source = r#"
import * as core from "scene";

                let value = "";
                let composing = "";
                const root = core.createElement('view');
                const field = core.createElement('view');
                const elsewhere = core.createElement('view');
                const label = core.createElement('text', { value: "field", style: { fontSize: 16, color: [255, 255, 255, 255] } });
                const other = core.createElement('view', { style: { width: 40, height: 20 } });
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
            crate::text::MonoShaper::default(),
            no_log(),
        ) {
            Ok(session) => session,
            Err(error) => panic!("{error}"),
        };
        session.metrics(&metrics());
        let first = session.frame(session::FrameTime::at(0.0));
        assert!(texts(&first).iter().any(|text| text == "field"));
        session.input(InputEvent::Text("你".to_string()));
        let ignored = session.frame(session::FrameTime::at(16.0));
        assert!(ignored.layers.is_none());
        assert!(texts(&first).iter().any(|text| text == "field"));
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
        assert!(texts(&composed).iter().any(|text| text == "你ni"));
        session.input(InputEvent::Composing(None));
        session.input(InputEvent::Key(session::KeyPress::new(Key::Backspace)));
        let cleared = session.frame(session::FrameTime::at(48.0));
        assert!(!texts(&cleared).iter().any(|text| text == "你"));
        assert!(!texts(&cleared).iter().any(|text| text == "你ni"));
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
        assert!(!texts(&after).iter().any(|text| text == "no"));
    }
}

#[test]
fn dismissing_the_keyboard_lets_blur_clear_ime_state() {
    {
        let source = r#"
import * as core from "scene";

                const root = core.createElement('view');
                const field = core.createElement('view');
                const label = core.createElement('text', { value: "field", style: { fontSize: 16, color: [255, 255, 255, 255] } });
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
            crate::text::MonoShaper::default(),
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
fn timers_and_modules_run_inside_the_frame() {
    {
        use std::sync::{Arc, Mutex};

        let lines = Arc::new(Mutex::new(Vec::new()));
        let log_lines = Arc::clone(&lines);
        let log = Box::new(move |_level: u8, text: &str| {
            log_lines
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(text.to_owned());
        });
        let source = r#"
                import * as clipboard from "@impasto/clipboard/native";
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
                addEventListener("change", (state) => console.log("state:" + state));
                clipboard.setString("hi");
                console.log("clip:" + clipboard.getString());
            "#;
        let (host, _pending) = test_host();
        let mut session = match super::JsSession::open_with(
            source,
            "host.js",
            crate::text::MonoShaper::default(),
            log,
            host,
        ) {
            Ok(session) => session,
            Err(error) => panic!("{error}"),
        };
        session.metrics(&metrics());
        let _ = session.frame(session::FrameTime::at(0.0));
        let _ = session.frame(session::FrameTime::at(20.0));
        session.lifecycle(session::Lifecycle::Paused);
        session.io_event(IoEvent::Host(javascript::host::HostEvent {
            module: "AppState".into(),
            event: "change".into(),
            payload: PluginValue::Str("background".into()),
        }));
        let text = lines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .join("\n");
        assert!(text.contains('1'), "{text}");
        assert!(text.contains("soon"), "{text}");
        assert!(text.contains("raf"), "{text}");
        assert!(text.contains("clip:hi"), "{text}");
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
                import { getInitialURL } from "@impasto/linking/native";
                console.log("initial:" + String(getInitialURL()));
            "#;
        let (host, _pending) =
            test_host_with_launch_url(Some("https://app.example/item".to_owned()));
        match super::JsSession::open_with(
            source,
            "launch.js",
            crate::text::MonoShaper::default(),
            log,
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
            crate::text::MonoShaper::default(),
            log,
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
            crate::text::MonoShaper::default(),
            log,
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
            crate::text::MonoShaper::default(),
            no_log(),
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
            crate::text::MonoShaper::default(),
            no_log(),
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
            crate::text::MonoShaper::default(),
            no_log(),
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
fn scene_insert_distinguishes_reused_node_slots_from_numeric_children() {
    let source = r#"
        import * as scene from "scene";
        const root = scene.createElement("view");
        const retired = scene.createElement("text", { value: "retired" });
        scene.discard(retired);
        const live = scene.createElement("view");
        if ((retired >>> 0) !== (live >>> 0) || retired === live) throw Error("slot not reused");

        scene.insert(root, [live, retired, 42, String(live)]);
        const first = scene.firstChild(root);
        const second = scene.nextSibling(first);
        const third = scene.nextSibling(second);
        const fourth = scene.nextSibling(third);
        if (scene._isNode !== undefined || scene.scene._isNode !== undefined) throw Error("private binding exported");
        if (first !== live || scene.isText(first) || !scene.isText(second)
            || !scene.isText(third) || !scene.isText(fourth) || scene.nextSibling(fourth) !== 0) {
            throw Error("unexpected child order or type");
        }
        if (scene.parent(second) !== root || scene.flingTarget(root) !== null) throw Error("query result");
        try {
            scene.isText(retired);
            throw Error("stale node accepted");
        } catch (error) {
            if (error.message !== "isText failed") throw error;
        }
    "#;
    let mut host = super::JsSession::open(
        source,
        "scene-insert.js",
        crate::text::MonoShaper::default(),
        no_log(),
    )
    .expect("session");
    host.metrics(&metrics());
    let output = host.frame(session::FrameTime::at(0.0));
    assert!(texts(&output).iter().any(|text| text == "42"));
}

#[test]
fn scene_selection_rects_return_the_entire_multiline_selection() {
    let source = r#"
        import * as scene from "scene";
        const root = scene.createElement("view");
        const text = Array(40).fill("x").join("\n");
        const paragraph = scene.createElement("text", { value: text });
        scene.append(root, paragraph);
        scene.rect(paragraph);
        const rects = scene.selectionRects(paragraph, 0, text.length);
        if (rects.length !== 40 || rects.some(rect => rect.w <= 0 || rect.h <= 0)) {
            throw Error("selection rectangles: " + rects.length);
        }
        if (scene.selectionRects(paragraph, 1, 1).length !== 0) throw Error("empty selection");
    "#;
    drop(
        super::JsSession::open(
            source,
            "scene-selection.js",
            crate::text::MonoShaper::default(),
            no_log(),
        )
        .expect("session"),
    );
}

#[test]
fn semantics_fields_keep_embedded_nul_characters() {
    let source = r#"
        import * as scene from "scene";
        const root = scene.createElement("view", { style: { width: 96, height: 24 } });
        scene.setSemantics(root, 6, "first\0last", "a\0b", -1, -1, -1, 0, 1, 0, "help\0text", {
            autofill: { hint: "username", value: "user\0name" },
        });
    "#;
    let mut host = super::JsSession::open(
        source,
        "semantics-text.js",
        crate::text::MonoShaper::default(),
        no_log(),
    )
    .expect("session");
    host.set_autofill_enabled(true);
    host.metrics(&metrics());
    host.frame(session::FrameTime::at(0.0));
    let update = host.semantics().expect("semantics");
    let field = update
        .nodes
        .iter()
        .find(|node| node.role == crate::semantics::SemanticsRole::TextField)
        .expect("text field");
    assert_eq!(field.label, "first\0last");
    assert_eq!(field.value.as_deref(), Some("a\0b"));
    assert_eq!(field.description.as_deref(), Some("help\0text"));
    assert_eq!(
        field.autofill.as_ref().map(|hint| hint.value.as_str()),
        Some("user\0name")
    );
}

#[test]
fn an_object_handle_is_rejected_and_a_switch_hint_sticks() {
    {
        let source = r#"
import * as core from "scene";

                const region = core.createElement('view');
                const label = core.createElement('text', { value: "idle", style: { fontSize: 16, color: [0, 0, 0, 255] } });
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
            crate::text::MonoShaper::default(),
            no_log(),
        ) {
            Ok(session) => session,
            Err(error) => panic!("{error}"),
        };
        session.metrics(&metrics());
        let frame = session.frame(session::FrameTime::at(0.0));
        assert!(
            texts(&frame).iter().any(|text| text == "bad handle"),
            "{:?}",
            texts(&frame)
        );
        let update = session.semantics().expect("semantics");
        let switch = update
            .nodes
            .iter()
            .find(|node| node.role == crate::semantics::SemanticsRole::Switch)
            .expect("switch");
        assert_eq!(switch.value.as_deref(), Some("On"));
        assert!(
            switch
                .flags
                .contains(crate::semantics::SemanticsFlags::CHECKED)
        );
    }
}

#[test]
fn settings_reach_the_script() {
    {
        let source = r#"
import * as core from "scene";

                const label = core.createElement('text', { value: "boot", style: { fontSize: 16, color: [0, 0, 0, 255] } });
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
            crate::text::MonoShaper::default(),
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
            texts(&frame).iter().any(|text| text == "zh-CN:dark:1"),
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

                const column = core.createElement('view');
                const label = core.createElement('text', { value: "none", style: { fontSize: 16, color: [0, 0, 0, 255] } });
                const first = core.createElement('view');
                const second = core.createElement('view');
                core.append(column, label);
                core.append(column, first);
                core.append(column, second);
                core.setFocusable(first, 1);
                core.setFocusable(second, 1);
                core.append(first, core.createElement('view', { style: { width: 20, height: 20 } }));
                core.append(second, core.createElement('view', { style: { width: 20, height: 20 } }));
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
            crate::text::MonoShaper::default(),
            no_log(),
        ) {
            Ok(session) => session,
            Err(error) => panic!("{error}"),
        };
        session.metrics(&metrics());
        let _ = session.frame(session::FrameTime::at(0.0));
        session.input(InputEvent::Key(session::KeyPress::new(Key::Tab)));
        let next = session.frame(session::FrameTime::at(16.0));
        assert!(
            texts(&next).iter().any(|text| text == "first"),
            "{:?}",
            texts(&next)
        );
        session.input(InputEvent::Key(session::KeyPress::new(Key::Tab)));
        let after = session.frame(session::FrameTime::at(32.0));
        assert!(
            texts(&after).iter().any(|text| text == "second"),
            "{:?}",
            texts(&after)
        );
        session.input(InputEvent::Key(session::KeyPress::new(Key::Escape)));
        let closed = session.frame(session::FrameTime::at(48.0));
        assert!(
            texts(&closed).iter().any(|text| text == "back"),
            "{:?}",
            texts(&closed)
        );
        assert!(!session.take_exit());
    }
}

#[test]
fn text_replaced_delivers_the_loaded_string() {
    {
        let source = r#"
import * as core from "scene";

                const root = core.createElement('view');
                const label = core.createElement('text', { value: "idle", style: { fontSize: 16, color: [0, 0, 0, 255] } });
                const field = core.createElement('text', { value: "old", style: { fontSize: 16, color: [0, 0, 0, 255] } });
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
            crate::text::MonoShaper::default(),
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
            .find(|node| node.role == crate::semantics::SemanticsRole::TextField)
            .expect("field");
        session.semantics_action(crate::semantics::SemanticsAction::SetText {
            id: field.id,
            text: "Hello".to_owned(),
        });
        let frame = session.frame(session::FrameTime::at(16.0));
        assert!(
            texts(&frame).iter().any(|text| text == "replaced:Hello"),
            "{:?}",
            texts(&frame)
        );
    }
}

/// Records every face the session hands to its shaper, in order.
struct FontRecorder {
    inner: crate::text::MonoShaper,
}

thread_local! {
    static REGISTERED_FONTS: std::cell::RefCell<Vec<(String, std::sync::Arc<[u8]>)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

fn registered_fonts() -> Vec<(String, std::sync::Arc<[u8]>)> {
    REGISTERED_FONTS.with(|fonts| std::mem::take(&mut *fonts.borrow_mut()))
}

impl crate::text::Shaper for FontRecorder {
    fn register_shared_font(&mut self, family: &str, bytes: std::sync::Arc<[u8]>) -> bool {
        REGISTERED_FONTS.with(|fonts| fonts.borrow_mut().push((family.to_owned(), bytes)));
        true
    }
    fn metrics(
        &mut self,
        key: &crate::text::MetricsKey,
    ) -> std::sync::Arc<crate::text::ParagraphMetrics> {
        self.inner.metrics(key)
    }
    fn caret_at_unit(
        &mut self,
        key: &crate::text::MetricsKey,
        unit: u32,
        affinity: crate::text::Affinity,
    ) -> crate::text::Caret {
        self.inner.caret_at_unit(key, unit, affinity)
    }
    fn caret_at_point(
        &mut self,
        key: &crate::text::MetricsKey,
        x: f32,
        y: f32,
    ) -> crate::text::Caret {
        self.inner.caret_at_point(key, x, y)
    }
    fn word_range(&mut self, key: &crate::text::MetricsKey, unit: u32) -> (u32, u32) {
        self.inner.word_range(key, unit)
    }
}

#[test]
fn register_font_queues_replaces_and_caps_names() {
    {
        let source = r#"
import * as core from "scene";

                const root = core.createElement('view');
                const label = core.createElement('text', { value: "boot", style: { fontSize: 16, color: [0, 0, 0, 255] } });
                const empty = core.createElement('text', { value: "empty-pending", style: { fontSize: 16, color: [0, 0, 0, 255] } });
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
            FontRecorder {
                inner: crate::text::MonoShaper::default(),
            },
            no_log(),
        ) {
            Ok(session) => session,
            Err(error) => panic!("{error}"),
        };
        // Nothing reaches the shaper until the first UI frame drains the queue.
        assert!(registered_fonts().is_empty());
        let listed = session.frame(session::FrameTime::at(0.0));
        let queued = registered_fonts();
        assert_eq!(queued.len(), 9, "{queued:?}");
        let replaced = queued.iter().find(|(name, _)| name == "F0").expect("F0");
        // Script callbacks run before layout, so only the final F0 bytes
        // for this frame reach the shaper.
        assert_eq!(replaced.1.as_ref(), &[8]);
        assert!(queued.iter().any(|(name, _)| name == "Sans"));
        assert!(queued.iter().all(|(name, _)| name != "Extra"));
        assert!(
            texts(&listed).iter().any(|text| text == "handed-rejected"),
            "{:?}",
            texts(&listed)
        );
        assert!(
            texts(&listed).iter().any(|text| text == "empty-rejected"),
            "{:?}",
            texts(&listed)
        );
        session.frame(session::FrameTime::at(16.0));
        assert!(registered_fonts().is_empty());
    }
}

#[test]
fn register_font_rejects_an_oversize_face_without_touching_the_queue() {
    {
        let source = r#"
import * as core from "scene";

                const label = core.createElement('text', { value: "boot", style: { fontSize: 16, color: [0, 0, 0, 255] } });
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
            FontRecorder {
                inner: crate::text::MonoShaper::default(),
            },
            no_log(),
        ) {
            Ok(session) => session,
            Err(error) => panic!("{error}"),
        };
        session.metrics(&metrics());
        let listed = session.frame(session::FrameTime::at(0.0));
        assert!(
            texts(&listed).iter().any(|text| text == "rejected"),
            "{:?}",
            texts(&listed)
        );
        let queued = registered_fonts();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].0, "Keep");
        assert_eq!(queued[0].1.as_ref(), &[7]);
    }
}

#[test]
fn reload_queues_a_source_and_rejects_an_empty_one() {
    let source = r#"
import * as core from "scene";

            const root = core.createElement('view');
            const label = core.createElement('text', { value: "boot", style: { fontSize: 16, color: [0, 0, 0, 255] } });
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
    let mut session = match open(
        source,
        "reload.js",
        crate::text::MonoShaper::default(),
        no_log(),
    ) {
        Ok(session) => session,
        Err(error) => panic!("{error}"),
    };
    let queued = session.take_reloads();
    assert_eq!(queued, vec!["asset:logo".to_owned()]);
    assert!(session.take_reloads().is_empty());
    session.metrics(&metrics());
    let frame = session.frame(session::FrameTime::at(0.0));
    assert!(
        texts(&frame).iter().any(|text| text == "rejected"),
        "{:?}",
        texts(&frame)
    );
}

#[test]
fn axis_percent_sizes_the_child_and_fill_is_rejected() {
    let source = r#"
import * as core from "scene";

            const parent = core.createElement('view');
            const child = core.createElement('view', { style: { backgroundColor: [0, 9, 0, 255] } });
            const label = core.createElement('text', { value: "boot", style: { fontSize: 16, color: [0, 0, 0, 255] } });
            core.append(parent, child);
            core.append(parent, label);
            core.patchStyle(parent, { width: 200, height: 80 });
            core.patchStyle(child, { width: "50%", height: 20 });
            try {
                core.patchStyle(child, { width: "fill", height: 20 });
                core.setText(label, "accepted");
            } catch (error) {
                const message = String(error);
                core.setText(label, error instanceof TypeError ? "bad-axis" : message);
            }
        "#;
    let mut session = match open(
        source,
        "axis.js",
        crate::text::MonoShaper::default(),
        no_log(),
    ) {
        Ok(session) => session,
        Err(error) => panic!("{error}"),
    };
    session.metrics(&metrics());
    let frame = session.frame(session::FrameTime::at(0.0));
    assert!(
        texts(&frame).iter().any(|text| text == "bad-axis"),
        "{:?}",
        texts(&frame)
    );
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

            const root = core.createElement('view');
            const picture = core.createElement('image', { source: "asset:logo", intrinsicSize: { width: 40, height: 20 }, required: true, style: {objectFit: 'contain'} });
            const label = core.createElement('text', { value: "boot", style: { fontSize: 16, color: [0, 0, 0, 255] } });
            core.append(root, picture);
            core.append(root, label);
            core.patchStyle(picture, { objectFit: 'cover' });
            try {
                core.patchStyle(picture, { objectFit: "stretch" });
                core.setText(label, "accepted");
            } catch (error) {
                const message = String(error);
                core.setText(label, error instanceof TypeError ? "bad-image" : message);
            }
        "#;
    let mut session = match open(
        source,
        "image.js",
        crate::text::MonoShaper::default(),
        no_log(),
    ) {
        Ok(session) => session,
        Err(error) => panic!("{error}"),
    };
    session.metrics(&metrics());
    let frame = session.frame(session::FrameTime::at(0.0));
    assert!(
        texts(&frame).iter().any(|text| text == "bad-image"),
        "{:?}",
        texts(&frame)
    );
    let mut image = None;
    visit_commands(&frame, &mut |command, _, _| {
        if let Command::Image { mode, required, .. } = command {
            image = Some((*mode, *required));
        }
    });
    assert_eq!(
        image,
        Some((crate::scene::ContentMode::Crop, true)),
        "{image:?}"
    );
}

#[test]
fn rect_flushes_pending_styles() {
    let source = r#"
import * as core from "scene";

            const root = core.createElement('view');
            const box = core.createElement('view', { style: { width: 8, height: 8 } });
            core.append(root, box);
            const label = core.createElement('text', { value: "stale", style: { fontSize: 16, color: [0, 0, 0, 255] } });
            core.append(root, label);
            core.patchStyle(box, { width: 30 });
            const rect = core.rect(box);
            core.setText(label, rect.w === 30 ? "applied" : "stale");
        "#;
    let mut session = match open(
        source,
        "mutate.js",
        crate::text::MonoShaper::default(),
        no_log(),
    ) {
        Ok(session) => session,
        Err(error) => panic!("{error}"),
    };
    session.metrics(&metrics());
    let frame = session.frame(session::FrameTime::at(0.0));
    assert!(
        texts(&frame).iter().any(|text| text == "applied"),
        "{:?}",
        texts(&frame)
    );
}

fn canvas_fills(frame: &session::FrameOutput) -> usize {
    let mut count = 0;
    visit_commands(frame, &mut |command, _, _| {
        if let Command::Canvas { list, .. } = command {
            count += list
                .ops()
                .iter()
                .filter(|op| {
                    matches!(
                        op,
                        crate::scene::CanvasOp::Fill { .. }
                            | crate::scene::CanvasOp::SolidRect { .. }
                    )
                })
                .count();
        }
    });
    count
}

fn open_canvas(source: &str, name: &str) -> Box<dyn Session> {
    match open(source, name, crate::text::MonoShaper::default(), no_log()) {
        Ok(session) => session,
        Err(error) => panic!("{error}"),
    }
}

#[test]
fn canvas_fill_is_recorded_in_the_content_box() {
    let source = r#"
import * as core from "scene";

            const root = core.createElement('view');
            const box = core.createElement('view');
            core.patchStyle(box, { width: 80, height: 40 });
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

const root = core.createElement('view');
const scroll = core.createElement('scroll');
const content = core.createElement('view');
const canvas = core.createElement('view', { style: { width: 80, height: 40 } });
const spacer = core.createElement('view', { style: { width: 80, height: 200 } });
const [value, set] = createSignal(0);
core.patchStyle(scroll, { width: 100, height: 80 });
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
                |op| matches!(op, crate::scene::CanvasOp::Text(text) if text.utf8.as_ref() == "100"),
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

const root = core.createElement('view');
for (const bulk of [false, true]) {
  const box = core.createElement('view', { style: { width: 80, height: 40 } });
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

            const root = core.createElement('view');
            const box = core.createElement('view');
            core.patchStyle(box, { width: 80, height: 40 });
            const label = core.createElement('text', { value: "ready", style: { fontSize: 16, color: [0, 0, 0, 255] } });
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

            const root = core.createElement('view');
            const box = core.createElement('view');
            core.patchStyle(box, { width: 40, height: 40 });
            const label = core.createElement('text', { value: "wait", style: { fontSize: 16, color: [0, 0, 0, 255] } });
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

            const root = core.createElement('view');
            const box = core.createElement('view');
            core.patchStyle(box, { width: 40, height: 40 });
            const label = core.createElement('text', { value: "wait", style: { fontSize: 16, color: [0, 0, 0, 255] } });
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
            const root = core.createElement('view');
            const box = core.createElement('view');
            core.patchStyle(box, { width: 40, height: 40 });
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
            const root = core.createElement('view');
            core.patchStyle(root, { width: 40, height: 40 });
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
            const root = core.createElement('view');
            const box = core.createElement('view');
            core.patchStyle(box, { width: 40, height: 40 });
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
                if let crate::scene::CanvasOp::Fill { paint, .. } = op
                    && let crate::scene::CanvasBrush::Linear { stops, .. } = &paint.brush
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
            const root = core.createElement('view');
            const box = core.createElement('view');
            core.patchStyle(box, { width: 40, height: 40 });
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
                    crate::scene::CanvasOp::Clip { path } if path.verbs.is_empty() => {
                        saw_clip = true;
                    }
                    crate::scene::CanvasOp::SolidRect { .. } if saw_clip => fill_after = true,
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
            const root = core.createElement('view');
            const box = core.createElement('view');
            core.patchStyle(box, { width: 40, height: 40 });
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
            const root = core.createElement('view');
            const box = core.createElement('view');
            core.patchStyle(box, { width: 40, height: 40 });
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
            const root = core.createElement('view');
            const label = core.createElement('text', { value: "ready", style: { fontSize: 16, color: [0, 0, 0, 255] } });
            const a = core.createElement('view');
            const b = core.createElement('view');
            core.patchStyle(a, { width: 40, height: 40 });
            core.patchStyle(b, { width: 40, height: 40 });
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
            const root = core.createElement('view');
            const box = core.createElement('view');
            core.patchStyle(box, { width: 40, height: 40 });
            const label = core.createElement('text', { value: "ready", style: { fontSize: 16, color: [0, 0, 0, 255] } });
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
        crate::text::MonoShaper::default(),
        no_log(),
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

#[test]
fn autofill_replaces_a_wrapper_field_without_exposing_the_password_in_semantics() {
    let source = r#"
        import * as scene from "scene";
        const root = scene.createElement('view');
        const editor = scene.createElement('view');
        scene.patchStyle(editor, { width: 96, height: 24 });
        scene.append(root, editor);
        const label = scene.createElement('text', { value: "unfilled", style: { fontSize: 16, color: [0, 0, 0, 255] } });
        scene.append(root, label);
        const publish = (value) => scene.setSemantics(
            editor, 6, "Password", undefined, -1, -1, -1, 0, 1, 0, "",
            { password: true, autofill: { hint: "password", value } },
        );
        publish("original");
        scene.onTextReplaced(editor, value => {
            publish(value);
            scene.setText(label, value === "new😀" ? "filled" : "wrong");
        });
    "#;
    let mut host = super::JsSession::open(
        source,
        "autofill.js",
        crate::text::MonoShaper::default(),
        no_log(),
    )
    .expect("session");
    host.set_autofill_enabled(true);
    host.metrics(&metrics());
    host.frame(session::FrameTime::at(0.0));
    let snapshot = host.semantics().expect("initial semantics");
    let field = snapshot
        .nodes
        .iter()
        .find(|node| node.autofill.is_some())
        .expect("field");
    assert_eq!(field.value, None);
    assert_eq!(
        field.autofill.as_ref().map(|entry| entry.value.as_str()),
        Some("original")
    );
    assert!(!format!("{field:?}").contains("original"));
    host.semantics_action(crate::semantics::SemanticsAction::Autofill {
        id: field.id,
        text: "new😀".to_owned(),
    });
    let output = host.frame(session::FrameTime::at(16.0));
    assert!(texts(&output).iter().any(|text| text == "filled"));
}

#[test]
fn debug_update_checks_the_candidate_first_frame_before_replacing_the_program() {
    let mut host = super::JsSession::open(
        r#"import { createElement } from "scene"; createElement("text", { value: "initial", style: { color: [0, 0, 0, 255] } });"#,
        "app.js",
        crate::text::MonoShaper::default(),
        no_log(),
    )
    .expect("initial session");
    host.metrics(&metrics());
    assert!(
        texts(&host.frame(session::FrameTime::at(0.0)))
            .iter()
            .any(|text| text == "initial")
    );
    let attempted = host.update_source(
        r#"import { createElement } from "scene"; createElement("text", { value: "bad", style: { color: [0, 0, 0, 255] } }); requestAnimationFrame(() => { throw new Error("first frame"); });"#,
        Box::new(crate::text::MonoShaper::default()),
        session::FrameTime::at(16.0),
    );
    assert!(attempted.unwrap_err().contains("first frame"));
    host.update_source(
        r#"import { createElement } from "scene"; createElement("text", { value: "updated", style: { color: [0, 0, 0, 255] } });"#,
        Box::new(crate::text::MonoShaper::default()),
        session::FrameTime::at(32.0),
    )
    .expect("successful update");
    assert!(
        texts(&host.frame(session::FrameTime::at(48.0)))
            .iter()
            .any(|text| text == "updated")
    );
}

#[cfg(feature = "gallery-tests")]
#[path = "../../../../examples/gallery-solid/tools/engine_tests.rs"]
mod gallery_tests;

const FRAME_REPLAY: &str = include_str!("../../../testing/fixtures/frame-replay.js");

#[test]
fn script_deadline_distinguishes_an_empty_tick_from_a_layout_change() {
    use crate::session::ScriptInvalidation;

    let mut session = super::JsSession::open(
        r#"
            import * as scene from 'scene';
            const root = scene.createElement('view');
            const label = scene.createElement('text', { value: 'before' });
            scene.append(root, label);
            let elapsed = false;
            setTimeout(() => { elapsed = true; }, 50);
            setTimeout(() => {
                if (!elapsed) throw new Error('first timer did not advance');
                scene.setText(label, 'after');
            }, 100);
        "#,
        "deadline.js",
        crate::text::MonoShaper::default(),
        no_log(),
    )
    .expect("open fixture");
    session.metrics(&phone());
    session.frame(session::FrameTime::at(0.0));
    assert_eq!(
        session.advance_script_deadline(50),
        ScriptInvalidation::Unchanged
    );
    assert_eq!(session.advance_script_deadline(100), ScriptInvalidation::Ui);
    let changed = session.frame(session::FrameTime::at(116.0));
    assert!(texts(&changed).contains(&"after".to_owned()));
    assert!(session.take_faults().is_empty());
}

#[test]
fn gesture_deadlines_use_one_wake_on_the_host_clock() {
    use crate::session::ScriptInvalidation;

    let mut session = super::JsSession::open(
        r#"
            import * as scene from 'scene';
            const root = scene.createElement('view');
            const region = scene.createElement('view');
            const label = scene.createElement('text', { value: 'idle' });
            scene.append(root, region);
            scene.append(region, label);
            scene.setRecognizers(region, { tap: true, doubleTapMs: 300, longPress: true });
            scene.onClick(region, () => scene.setText(label, 'single'));
            scene.onDoubleTap(region, () => scene.setText(label, 'double'));
            scene.onLongPress(region, () => scene.setText(label, 'long'));
            scene.onZoom(region, (scale) => scene.setText(label, 'zoom ' + scale));
        "#,
        "gesture-deadline.js",
        crate::text::MonoShaper::default(),
        no_log(),
    )
    .expect("open fixture");
    session.metrics(&phone());
    let epoch = 80_000.0;
    let time = |mono_ms| session::FrameTime {
        mono_ms,
        ..session::FrameTime::at(epoch + mono_ms)
    };
    let initial = session.frame(time(0.0));
    let (x, y) = text_spot(&initial, "idle");
    let pointer = |session: &mut super::JsSession, phase, offset| {
        session.pointer(PointerEvent::new(phase, x, y, epoch + offset));
    };

    pointer(&mut session, Phase::Down, 10.0);
    assert!(!session.frame(time(16.0)).again);
    assert_eq!(session.next_wake_ms(), Some(510.0));
    pointer(&mut session, Phase::Up, 20.0);
    assert!(!session.frame(time(32.0)).again);
    assert_eq!(session.next_wake_ms(), Some(320.0));
    assert_eq!(
        session.advance_script_deadline(319),
        ScriptInvalidation::Unchanged
    );
    assert_eq!(session.advance_script_deadline(320), ScriptInvalidation::Ui);
    assert_eq!(texts(&session.frame(time(320.0))), ["single"]);
    assert_eq!(session.next_wake_ms(), None);

    pointer(&mut session, Phase::Down, 400.0);
    pointer(&mut session, Phase::Up, 410.0);
    pointer(&mut session, Phase::Down, 470.0);
    assert!(!session.frame(time(474.0)).again);
    assert_eq!(session.next_wake_ms(), Some(710.0));
    pointer(&mut session, Phase::Up, 480.0);
    assert_eq!(texts(&session.frame(time(486.0))), ["double"]);
    assert_eq!(session.next_wake_ms(), None);

    pointer(&mut session, Phase::Down, 600.0);
    assert!(!session.frame(time(608.0)).again);
    assert_eq!(session.next_wake_ms(), Some(1_100.0));
    assert_eq!(
        session.advance_script_deadline(1_100),
        ScriptInvalidation::Ui
    );
    assert_eq!(texts(&session.frame(time(1_100.0))), ["long"]);
    pointer(&mut session, Phase::Up, 1_120.0);
    session.frame(time(1_128.0));
    assert_eq!(session.next_wake_ms(), None);

    pointer(&mut session, Phase::Down, 1_200.0);
    pointer(&mut session, Phase::Up, 1_210.0);
    pointer(&mut session, Phase::Down, 1_270.0);
    pointer(&mut session, Phase::Cancel, 1_280.0);
    assert!(!session.frame(time(1_286.0)).again);
    assert_eq!(session.next_wake_ms(), Some(1_510.0));
    assert_eq!(texts(&session.frame(time(1_510.0))), ["single"]);
    assert_eq!(session.next_wake_ms(), None);

    session.pointer(PointerEvent::zoom(x, y, epoch + 1_520.0, 1.25));
    assert_eq!(texts(&session.frame(time(1_528.0))), ["zoom 1.25"]);
    assert!(session.take_faults().is_empty());
}

#[test]
fn frame_replay_cancels_taps_and_cleans_jobs_after_navigation() {
    let mut session = super::JsSession::open(
        FRAME_REPLAY,
        "frame-replay.js",
        crate::text::MonoShaper::default(),
        no_log(),
    )
    .expect("open frame replay");
    session.metrics(&phone());
    session.set_semantics_enabled(true);
    pump_first(&mut session);
    let (x, y) = label_spot(&mut session, "Open detail").expect("visible launcher");
    session.pointer(PointerEvent::new(Phase::Down, x, y, 1.0));
    session.pointer(PointerEvent::new(Phase::Move, x, -20.0, 8.0));
    session.pointer(PointerEvent::new(Phase::Cancel, x, -20.0, 9.0));
    session.frame(session::FrameTime::at(16.0));
    assert!(label_spot(&mut session, "Back").is_none());
    for index in 0..3 {
        let time = 32.0 + f64::from(index) * 64.0;
        click(&mut session, x, y);
        let detail = session.frame(session::FrameTime::at(time));
        assert!(texts(&detail).contains(&format!("Detail {}", index + 1)));
        assert!(!session.jobs_pending());
        assert!(label_spot(&mut session, "Close dialog").is_none());
        let (back_x, back_y) = label_spot(&mut session, "Back").expect("visible back");
        click(&mut session, back_x, back_y);
        session.frame(session::FrameTime::at(time + 16.0));
        assert!(label_spot(&mut session, "Open detail").is_some());
        assert!(session.take_faults().is_empty());
    }
}

#[test]
fn frame_replay_scroll_updates_the_visible_window_and_keeps_recorded_rows() {
    let mut session = super::JsSession::open(
        FRAME_REPLAY,
        "frame-replay.js",
        crate::text::MonoShaper::default(),
        no_log(),
    )
    .expect("open frame replay");
    session.metrics(&phone());
    let initial = pump_first(&mut session);
    let before = initial.layers.expect("initial list");
    let mut down = PointerEvent::new(Phase::Down, 120.0, 300.0, 8.0);
    down.kind = crate::input::PointerKind::Touch;
    session.pointer(down);
    let mut drag = PointerEvent::new(Phase::Move, 120.0, 180.0, 16.0);
    drag.kind = down.kind;
    session.pointer(drag);
    let next = session.frame(session::FrameTime::at(32.0));
    let after = next.layers.expect("scroll layers");
    assert_ne!(before.epoch, after.epoch);
    assert_eq!(session.picture_writes(), 0);
    assert!(before.pictures.iter().any(|old| {
        after
            .pictures
            .iter()
            .any(|new| old.id == new.id && old.generation == new.generation)
    }));
    assert!(session.take_faults().is_empty());
}

#[test]
fn frame_replay_reports_composition_and_clears_focus_after_return() {
    use crate::session::ImeUpdate;

    let mut session = super::JsSession::open(
        FRAME_REPLAY,
        "frame-replay.js",
        crate::text::MonoShaper::default(),
        no_log(),
    )
    .expect("open frame replay");
    session.metrics(&phone());
    session.set_semantics_enabled(true);
    pump_first(&mut session);
    let (x, y) = label_spot(&mut session, "Open detail").expect("visible launcher");
    click(&mut session, x, y);
    session.frame(session::FrameTime::at(16.0));
    let (x, y) = label_spot(&mut session, "Show dialog").expect("dialog launcher");
    click(&mut session, x, y);
    session.frame(session::FrameTime::at(32.0));
    let (x, y) = label_spot(&mut session, "Close dialog").expect("visible dialog");
    click(&mut session, x, y);
    let closed = session.frame(session::FrameTime::at(48.0));
    let (x, y) = text_spot(&closed, "Edit text");
    click(&mut session, x, y);
    let focused = session.frame(session::FrameTime::at(64.0));
    assert!(matches!(focused.ime, Some(ImeUpdate::Show(_))));
    session.input(InputEvent::Replace {
        text: "拼音👩‍🚀".to_owned(),
        start: 2,
        end: 2,
        compose_start: 0,
        compose_end: 2,
    });
    let edited = session.frame(session::FrameTime::at(80.0));
    assert!(texts(&edited).contains(&"拼音👩‍🚀".to_owned()));
    assert!(matches!(edited.ime, Some(ImeUpdate::Update(_))));
    let (x, y) = label_spot(&mut session, "Back").expect("visible back");
    click(&mut session, x, y);
    let home = session.frame(session::FrameTime::at(96.0));
    assert!(matches!(home.ime, Some(ImeUpdate::Hide)));
    assert!(label_spot(&mut session, "Open detail").is_some());
    assert!(session.take_faults().is_empty());
}
