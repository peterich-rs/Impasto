#![cfg(feature = "runtime")]
#![allow(
    clippy::expect_used,
    reason = "test assertions identify the failed step"
)]

use javascript::host::{Method, Module, ModuleRegistry, PluginValue};
use javascript::runtime::{ModuleCatalog, Runtime};
use javascript::standard::StandardProfile;

fn portable() -> Runtime {
    Runtime::builder()
        .standard(StandardProfile)
        .build()
        .unwrap_or_else(|error| panic!("portable runtime failed: {error}"))
}

fn eval(runtime: &mut Runtime, source: &str) {
    runtime
        .eval_script(source, "web-tests.js")
        .unwrap_or_else(|error| panic!("script failed: {error}"));
}

#[test]
fn standard_manifest_tracks_runtime_globals_and_dependencies() {
    let profile = StandardProfile;
    let mut runtime = Runtime::builder()
        .standard(profile)
        .build()
        .expect("runtime");
    let mut installed = std::collections::HashSet::new();
    for module in profile.modules() {
        for dependency in module.dependencies {
            assert!(
                installed.contains(dependency),
                "{dependency} before {}",
                module.name
            );
        }
        assert!(installed.insert(module.name));
    }
    for global in profile.globals() {
        let test = format!(
            "var descriptor = Object.getOwnPropertyDescriptor(globalThis, {global:?}); \
             if (!descriptor || !descriptor.writable || !descriptor.configurable) \
             throw Error({global:?});"
        );
        eval(&mut runtime, &test);
    }
    for removed in [
        "fetch",
        "fetchStream",
        "WebSocket",
        "Headers",
        "Request",
        "Response",
        "__host",
    ] {
        eval(
            &mut runtime,
            &format!("if (Object.hasOwn(globalThis, {removed:?})) throw Error({removed:?});"),
        );
    }
}

#[test]
fn event_snapshot_respects_removal_once_reentry_and_abort() {
    let mut runtime = portable();
    eval(
        &mut runtime,
        r#"
const trace = [];
const target = new EventTarget();
const event = new Event('ping', { cancelable: true });
const removed = () => trace.push('removed');
const late = () => trace.push('late');
target.addEventListener('ping', () => {
  trace.push('once');
  target.removeEventListener('ping', removed);
  target.addEventListener('ping', late);
  try {
    target.dispatchEvent(event);
    throw Error('reentrant dispatch succeeded');
  } catch (error) {
    if (error.name !== 'InvalidStateError') trace.push('invalid: ' + error);
  }
  target.dispatchEvent(new Event('other'));
}, { once: true });
target.addEventListener('ping', removed);
target.addEventListener('ping', () => { throw Error('reported listener error'); });
target.addEventListener('ping', function (event) {
  if (this !== target || event.currentTarget !== target || event.eventPhase !== 2) throw Error('event target');
  event.preventDefault();
  trace.push('ping');
});
target.addEventListener('other', () => trace.push('nested'));
if (target.dispatchEvent(event) || !event.defaultPrevented || event.currentTarget !== null) throw Error('dispatch result');
target.dispatchEvent(new Event('ping'));
if (trace.join(',') !== 'once,nested,ping,ping,late') throw Error(trace.join(','));

const controller = new AbortController();
target.addEventListener('aborted', () => trace.push('stale target listener'), { signal: controller.signal });
let aborts = 0;
controller.signal.addEventListener('abort', () => aborts++, { once: true });
controller.signal.onabort = () => aborts++;
const reason = { why: 'stop' };
controller.abort(reason);
controller.abort('again');
target.dispatchEvent(new Event('aborted'));
if (trace.includes('stale target listener')) throw Error(trace);
try {
  controller.signal.throwIfAborted();
  throw Error('no abort');
} catch (error) {
  if (error !== reason || !controller.signal.aborted || aborts !== 2) throw Error('abort state');
}
if (AbortSignal.abort().reason.name !== 'AbortError') throw Error('default abort reason');
"#,
    );
}

#[test]
fn encoding_and_url_match_web_subset_fixtures() {
    let mut runtime = portable();
    eval(
        &mut runtime,
        r#"
const encode = new TextEncoder();
const decoder = new TextDecoder(' UTF8 ');
const scalar = 'A\ud800\udc00\ud800Z';
if (decoder.decode(encode.encode(scalar)) !== 'A\ud800\udc00\ufffdZ') throw Error('USVString');
const data = new Uint8Array([255, 65, 66, 255]);
if (decoder.decode(data.subarray(1, 3)) !== 'AB' || decoder.decode(new DataView(data.buffer, 1, 2)) !== 'AB') {
  throw Error('view range');
}
if (decoder.decode(new Uint8Array([0xef]), { stream: true }) !== '') throw Error('BOM prefix');
if (decoder.decode(new Uint8Array([0xbb, 0xbf, 0xf0, 0x9f]), { stream: true }) !== '') throw Error('BOM split');
if (decoder.decode(new Uint8Array([0x8c, 0x88])) !== '🌈') throw Error('code point split');
if (decoder.decode(new Uint8Array([0xe1, 0x80, 65])) !== '\ufffdA') throw Error('invalid byte recovery');
if (decoder.decode() !== '') throw Error('empty decode');
if (new TextDecoder('utf-8', { ignoreBOM: true }).decode(new Uint8Array([0xef, 0xbb, 0xbf])) !== '\ufeff') throw Error('BOM');
try {
  new TextDecoder('utf8', { fatal: true }).decode(new Uint8Array([0xc0, 0xaf]));
  throw Error('fatal decode succeeded');
} catch (error) {
  if (!(error instanceof TypeError)) throw error;
}

const url = new URL('../café?k=1&k=2&empty&bad=%FF', 'HTTPS://éxample.test:443/dir/sub/');
if (url.origin !== 'https://xn--xample-9ua.test' || url.pathname !== '/dir/caf%C3%A9' || url.port !== '') throw Error(url.href);
const params = url.searchParams;
if (params.getAll('k').join(',') !== '1,2' || params.get('empty') !== '' || params.get('bad') !== '\ufffd') throw Error('parse form');
params.delete('k', '1');
params.append('a b', '~+💡');
if (params.toString() !== 'k=2&empty=&bad=%EF%BF%BD&a+b=%7E%2B%F0%9F%92%A1') throw Error(params.toString());
if (!url.search.includes('a+b=') || url.searchParams !== params) throw Error('live URLSearchParams');
url.search = '?z=5';
url.hash = 'line one';
if (params.get('z') !== '5' || params.size !== 1 || url.hash !== '#line%20one') throw Error(url.href);
if (Array.from(new URLSearchParams({ item: 'a b' }))[0][0] !== 'item') throw Error('record init');
if (URL.canParse('relative') || !URL.canParse('relative', url)) throw Error('canParse');
"#,
    );
}

#[test]
fn utf8_decoder_rejects_detached_input_and_flushes_partial_bytes() {
    let mut runtime = portable();
    eval(
        &mut runtime,
        r#"
const decoder = new TextDecoder();
const bytes = new Uint8Array([65]);
bytes.buffer.transfer();
try {
  decoder.decode(bytes);
  throw Error('decoded detached bytes');
} catch (error) {
  if (!(error instanceof TypeError)) throw error;
}
if (decoder.decode(new Uint8Array([0xf0, 0x90]), { stream: true }) !== '' ||
    decoder.decode() !== '\ufffd' || decoder.decode() !== '') throw Error('flush partial input');
"#,
    );
}

#[test]
fn native_web_constructors_keep_subclasses() {
    let mut runtime = portable();
    eval(
        &mut runtime,
        r#"
class ExtendedTarget extends EventTarget {}
class ExtendedEvent extends Event {}
class ExtendedUrl extends URL {}
class ExtendedParams extends URLSearchParams {}
class ExtendedEncoder extends TextEncoder {}
class ExtendedDecoder extends TextDecoder {}
class ExtendedStream extends ReadableStream {}
class ExtendedError extends DOMException {}
if (!(new ExtendedTarget() instanceof EventTarget)) throw Error('EventTarget prototype');
if (!(new ExtendedEvent('x') instanceof Event)) throw Error('Event prototype');
if (!(new ExtendedUrl('https://example.test') instanceof URL)) throw Error('URL prototype');
if (!(new ExtendedParams() instanceof URLSearchParams)) throw Error('URLSearchParams prototype');
if (!(new ExtendedEncoder() instanceof TextEncoder)) throw Error('TextEncoder prototype');
if (!(new ExtendedDecoder() instanceof TextDecoder)) throw Error('TextDecoder prototype');
if (!(new ExtendedStream() instanceof ReadableStream)) throw Error('ReadableStream prototype');
if (!(new ExtendedError('msg', 'AbortError') instanceof DOMException)) throw Error('DOMException prototype');
for (const [ctor, args] of [[EventTarget, []], [Event, ['x']], [URL, ['https://example.test']], [TextEncoder, []], [TextDecoder, []], [ReadableStream, []]]) {
  try { ctor(...args); throw Error('constructor called without new'); }
  catch (error) { if (!(error instanceof TypeError)) throw error; }
}
"#,
    );
}

#[test]
fn native_console_time_uses_the_same_manual_clock() {
    let lines = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let mut runtime = Runtime::builder()
        .log({
            let lines = std::rc::Rc::clone(&lines);
            move |level, text| lines.borrow_mut().push((level, text.to_owned()))
        })
        .build()
        .expect("runtime");
    runtime.set_clock(100);
    eval(&mut runtime, "console.time(undefined);");
    runtime.set_clock(112);
    eval(
        &mut runtime,
        "console.timeLog(); console.timeEnd(); console.timeLog();",
    );
    assert_eq!(
        *lines.borrow(),
        [
            (1, "default: 12.0 ms".into()),
            (1, "default: 12.0 ms".into()),
            (2, "Timer \"default\" does not exist".into()),
        ]
    );
}

#[test]
fn host_byte_codec_uses_current_length_tracking_view_bounds() {
    let captured = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let module = Module {
        name: "BytesProbe",
        specifier: "@impasto/bytes-probe",
        exports: Default::default(),
        methods: vec![Method::sync("send", {
            let captured = std::sync::Arc::clone(&captured);
            move |args| {
                let Some(PluginValue::Bytes(bytes)) = args.first() else {
                    return Err("expected bytes".into());
                };
                *captured.lock().expect("lock") = bytes.clone();
                Ok(PluginValue::Null)
            }
        })],
    };
    let catalog = ModuleCatalog::new(ModuleRegistry::try_new(vec![module]).expect("registry"))
        .expect("catalog");
    let mut runtime = Runtime::builder()
        .modules(catalog)
        .module(
            "app:bytes",
            r#"
import { send } from '@impasto/bytes-probe';
const buffer = new ArrayBuffer(2, { maxByteLength: 8 });
const view = new Uint8Array(buffer);
buffer.resize(4);
view.set([4, 3, 2, 1]);
send(view);
"#,
        )
        .build()
        .expect("runtime");
    let ticket = runtime.start_module("app:bytes").expect("start");
    runtime.drive(0).expect("drive");
    assert_eq!(
        runtime.take_module_result(ticket).expect("result"),
        Some(javascript::runtime::ModuleOutcome::Fulfilled)
    );
    assert_eq!(*captured.lock().expect("lock"), vec![4, 3, 2, 1]);
}
