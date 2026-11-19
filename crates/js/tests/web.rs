#![allow(
    clippy::expect_used,
    reason = "test assertions identify the failed step"
)]

use js::Runtime;
use js::standard::StandardProfile;
use js_host::{HttpError, HttpFailure, IoEvent, IoJob};

fn portable() -> Runtime {
    Runtime::builder()
        .standard(StandardProfile::portable())
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
    for profile in [
        StandardProfile::legacy(),
        StandardProfile::portable(),
        StandardProfile::portable().with_transports(false, false),
    ] {
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
                 if (!descriptor || !descriptor.writable || !descriptor.configurable || !descriptor.enumerable) \
                 throw Error({global:?});"
            );
            eval(&mut runtime, &test);
        }
        for optional in ["fetch", "fetchStream", "WebSocket"] {
            let expected = profile.globals().any(|name| name == optional);
            eval(
                &mut runtime,
                &format!(
                    "if (Object.hasOwn(globalThis, {optional:?}) !== {expected}) throw Error({optional:?});"
                ),
            );
        }
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
fn static_body_tracks_null_empty_lock_and_disturbance() {
    let mut runtime = portable();
    eval(
        &mut runtime,
        r#"
const headers = new Headers([['X-Test', '  a '], ['x-test', 'b'], ['Set-Cookie', 'a=1'], ['set-cookie', 'b=2']]);
if (headers.get('X-TEST') !== 'a, b' || headers.getSetCookie().join('/') !== 'a=1/b=2') throw Error('headers');
globalThis.bodyDone = false;
globalThis.bodyError = undefined;
(async () => {
  const absent = new Response(null);
  const empty = new Response('');
  if (absent.body !== null || absent.bodyUsed || empty.body === null || empty.bodyUsed) throw Error('empty vs null');
  if (await absent.text() !== '' || (await absent.arrayBuffer()).byteLength !== 0 || absent.bodyUsed) throw Error('null consume');
  const held = empty.body.getReader();
  if (!empty.body.locked || empty.bodyUsed) throw Error('lock disturbed body');
  try {
    await empty.text();
    throw Error('locked body consumed');
  } catch (error) {
    if (!(error instanceof TypeError)) throw error;
  }
  held.releaseLock();
  if (empty.body.locked || empty.bodyUsed || await empty.text() !== '' || !empty.bodyUsed) throw Error('released body');
  try {
    await empty.arrayBuffer();
    throw Error('disturbed body consumed');
  } catch (error) {
    if (!(error instanceof TypeError)) throw error;
  }
  const request = new Request('https://example.test/post', { method: 'post', body: 'q=abc' });
  const copied = new Request(request);
  if (!request.bodyUsed || copied.bodyUsed || await copied.text() !== 'q=abc') throw Error('body transfer');
  globalThis.bodyDone = true;
})().catch(error => { globalThis.bodyError = error; });
"#,
    );
    runtime.drive(0).expect("consume body");
    eval(
        &mut runtime,
        "if (!bodyDone) throw bodyError || Error('pending body');",
    );
}

fn stream_token(runtime: &mut Runtime) -> u64 {
    match runtime.take_jobs().as_slice() {
        [IoJob::HttpStream(request)] => request.token,
        other => panic!("expected one stream job, got {other:?}"),
    }
}

fn head(runtime: &Runtime, token: u64) {
    assert!(
        runtime
            .enqueue(IoEvent::HttpHead {
                token,
                status: 200,
                headers: vec![("x-fixture".into(), "yes".into())],
            })
            .is_ok()
    );
}

fn chunk(runtime: &Runtime, token: u64, bytes: &[u8], done: bool) {
    assert!(
        runtime
            .enqueue(IoEvent::HttpChunk {
                token,
                bytes: bytes.to_vec(),
                done,
                error: None,
            })
            .is_ok()
    );
}

#[test]
fn portable_fetch_resolves_at_head_and_reads_across_chunks() {
    let mut runtime = portable();
    eval(
        &mut runtime,
        r#"
globalThis.status = 0;
globalThis.text = null;
fetch('https://example.test/stream').then(response => {
  status = response.status;
  if (response.headers.get('X-Fixture') !== 'yes') throw Error('response headers');
  return response.text();
}).then(result => { text = result; });
"#,
    );
    let token = stream_token(&mut runtime);
    head(&runtime, token);
    runtime.drive(1).expect("response head");
    eval(
        &mut runtime,
        "if (status !== 200 || text !== null) throw Error('head checkpoint');",
    );
    chunk(&runtime, token, &[0xe7, 0x95], false);
    runtime.drive(2).expect("first bytes");
    chunk(&runtime, token, &[0x8c], true);
    runtime.drive(3).expect("last bytes");
    eval(
        &mut runtime,
        "if (text !== '界') throw Error(String(text));",
    );
    assert_eq!(runtime.counts().requests, 0);
    assert_eq!(runtime.counts().queued_bytes, 0);
}

#[test]
fn reader_release_abort_and_terminal_errors_settle_the_right_promises() {
    let mut runtime = portable();
    eval(
        &mut runtime,
        r#"
globalThis.controller = new AbortController();
globalThis.outcome = [];
fetch('https://example.test/stream', { signal: controller.signal }).then(response => {
  globalThis.result = response;
  const reader = response.body.getReader();
  reader.read().catch(error => outcome.push(error.name));
  reader.releaseLock();
  globalThis.finalReader = response.body.getReader();
  finalReader.read().catch(error => outcome.push(error));
});
"#,
    );
    let token = stream_token(&mut runtime);
    head(&runtime, token);
    runtime.drive(0).expect("head and first reader release");
    eval(
        &mut runtime,
        "if (!result.bodyUsed || outcome.join(',') !== 'TypeError') throw Error(outcome); controller.abort('stop');",
    );
    runtime.drive(1).expect("abort jobs");
    eval(
        &mut runtime,
        "if (outcome.join(',') !== 'TypeError,stop') throw Error(outcome);",
    );
    assert_eq!(runtime.counts().requests, 0);
    chunk(&runtime, token, b"ignored", true);
    runtime.drive(2).expect("ignore stale transport");
    assert_eq!(runtime.counts().queued_bytes, 0);

    eval(
        &mut runtime,
        "fetch('https://example.test/down').catch(error => { globalThis.failure = error.name; });",
    );
    let token = match runtime.take_jobs().last() {
        Some(IoJob::HttpStream(request)) => request.token,
        other => panic!("expected retry job, got {other:?}"),
    };
    assert!(
        runtime
            .enqueue(IoEvent::HttpChunk {
                token,
                bytes: Vec::new(),
                done: true,
                error: Some(HttpError::new(HttpFailure::Timeout, "down")),
            })
            .is_ok()
    );
    runtime.drive(3).expect("transport failure");
    eval(
        &mut runtime,
        "if (failure !== 'TimeoutError') throw Error(failure);",
    );
    assert_eq!(runtime.counts().requests, 0);
}

#[test]
fn queued_credit_follows_the_body_past_transport_end() {
    let mut runtime = portable();
    eval(
        &mut runtime,
        "fetch('https://example.test/unread').then(response => { globalThis.unread = response; });",
    );
    let token = stream_token(&mut runtime);
    head(&runtime, token);
    chunk(&runtime, token, b"held", true);
    runtime.drive(1).expect("enqueue body chunk");
    assert_eq!(runtime.counts().requests, 0);
    assert_eq!(runtime.counts().queued_bytes, 4);
    eval(
        &mut runtime,
        "unread.body.getReader().read().then(part => { globalThis.held = new TextDecoder().decode(part.value); });",
    );
    assert_eq!(runtime.counts().queued_bytes, 0);
    runtime.drive(2).expect("read buffered bytes");
    eval(&mut runtime, "if (held !== 'held') throw Error(held);");
}

#[test]
fn socket_terminal_releases_the_connection_and_rejects_late_events() {
    let mut runtime = portable();
    eval(
        &mut runtime,
        r#"
globalThis.trace = [];
globalThis.socket = new WebSocket('https://example.test/socket', ['probe']);
try {
  socket.send('early');
  throw Error('connecting send succeeded');
} catch (error) {
  if (error.name !== 'InvalidStateError') throw error;
}
socket.onopen = () => {
  if (socket.readyState !== WebSocket.OPEN) throw Error('bad open state');
  socket.send(new Uint8Array([1, 2, 3, 4]).subarray(1, 3));
  trace.push(socket.protocol);
};
socket.onerror = () => trace.push('error');
socket.onclose = event => trace.push(String(event.code));
"#,
    );
    let id = match runtime.take_jobs().as_slice() {
        [IoJob::SocketOpen { id, url, protocols }] => {
            assert_eq!(url, "wss://example.test/socket");
            assert_eq!(protocols, &["probe"]);
            *id
        }
        other => panic!("expected socket job, got {other:?}"),
    };
    let post = |event: &str, text: &str| IoEvent::Socket {
        id,
        event: event.into(),
        text: true,
        data: text.as_bytes().to_vec(),
    };
    assert!(runtime.enqueue(post("open", "probe")).is_ok());
    runtime.drive(0).expect("open socket");
    assert!(
        matches!(runtime.take_jobs().as_slice(), [IoJob::SocketSend { text: false, data, .. }] if data == &[2, 3])
    );
    assert!(runtime.enqueue(post("error", "down")).is_ok());
    assert!(runtime.enqueue(post("close", "")).is_ok());
    assert!(runtime.enqueue(post("open", "late")).is_ok());
    runtime.drive(1).expect("close socket");
    eval(
        &mut runtime,
        "if (trace.join(',') !== 'probe,error,1006' || socket.readyState !== WebSocket.CLOSED) throw Error(trace); socket.send('ignored');",
    );
    assert!(runtime.take_jobs().is_empty());
    assert_eq!(runtime.counts().sockets, 0);
}

#[test]
fn legacy_stream_empty_end_closes_without_a_spurious_piece() {
    let mut runtime = Runtime::builder().build().expect("legacy runtime");
    eval(
        &mut runtime,
        r#"
fetchStream('https://example.test/empty').then(response => {
  const reader = response.getReader();
  if (response.getReader() !== reader) throw Error('compatibility reader identity');
  return reader.read();
}).then(value => { globalThis.empty = value; });
"#,
    );
    let token = stream_token(&mut runtime);
    head(&runtime, token);
    chunk(&runtime, token, &[], true);
    runtime.drive(0).expect("empty stream");
    eval(
        &mut runtime,
        "if (!empty.done || empty.value !== undefined) throw Error('spurious empty chunk');",
    );
    assert_eq!(runtime.counts().requests, 0);
}

#[test]
fn unread_stream_does_not_hold_the_other_streams_credit() {
    let mut runtime = Runtime::builder()
        .standard(StandardProfile::portable())
        .limits(js::RuntimeLimits {
            stream_bytes: 4,
            ..js::RuntimeLimits::default()
        })
        .build()
        .expect("limited runtime");
    eval(
        &mut runtime,
        r#"
fetch('https://example.test/slow').then(response => { globalThis.slow = response; });
fetch('https://example.test/fast').then(response => response.text()).then(text => { globalThis.fast = text; });
"#,
    );
    let (slow, fast) = match runtime.take_jobs().as_slice() {
        [IoJob::HttpStream(slow), IoJob::HttpStream(fast)] => (slow.token, fast.token),
        other => panic!("expected parallel stream jobs, got {other:?}"),
    };
    for token in [slow, fast] {
        assert!(
            runtime
                .enqueue(IoEvent::HttpHead {
                    token,
                    status: 200,
                    headers: Vec::new(),
                })
                .is_ok()
        );
    }
    chunk(&runtime, slow, b"slow", false);
    chunk(&runtime, fast, b"fast", true);
    runtime.drive(0).expect("parallel streams");
    assert_eq!(runtime.counts().queued_bytes, 4);
    assert!(
        runtime
            .enqueue(IoEvent::HttpChunk {
                token: slow,
                bytes: Vec::from(&b"more"[..]),
                done: false,
                error: None,
            })
            .is_err()
    );
    eval(
        &mut runtime,
        "if (fast !== 'fast' || slow.bodyUsed) throw Error('slow stream blocked the fast one'); slow.body.cancel('done');",
    );
    assert_eq!(runtime.counts().queued_bytes, 0);
    assert_eq!(runtime.counts().requests, 0);
}

#[test]
fn remote_error_after_head_rejects_reads_instead_of_the_response() {
    let mut runtime = portable();
    eval(
        &mut runtime,
        r#"
globalThis.trace = [];
fetch('https://example.test/broken').then(response => {
  trace.push('head');
  const reader = response.body.getReader();
  reader.read().catch(error => trace.push(error.name));
  reader.closed.catch(error => trace.push('closed:' + error.name));
}, error => trace.push('fetch:' + error.name));
"#,
    );
    let token = stream_token(&mut runtime);
    head(&runtime, token);
    runtime.drive(0).expect("head available");
    assert!(
        runtime
            .enqueue(IoEvent::HttpChunk {
                token,
                bytes: Vec::new(),
                done: true,
                error: Some(HttpError::new(HttpFailure::Network, "disconnected")),
            })
            .is_ok()
    );
    runtime.drive(1).expect("readers reject");
    eval(
        &mut runtime,
        "if (trace.join(',') !== 'head,closed:TypeError,TypeError') throw Error(trace);",
    );
}

#[test]
fn portable_does_not_publish_fetch_for_an_inline_transport() {
    struct UnusedClient;

    impl js_host::HttpClient for UnusedClient {
        fn perform(&self, _: &js_host::HttpRequest) -> Result<js_host::HttpResponse, HttpError> {
            panic!("no portable fetch should reach an inline client")
        }
    }

    let mut runtime = Runtime::builder()
        .standard(StandardProfile::portable())
        .http_client(std::sync::Arc::new(UnusedClient))
        .build()
        .expect("runtime");
    eval(
        &mut runtime,
        "if (typeof fetch !== 'undefined' || typeof fetchStream !== 'undefined' || typeof Response !== 'function') throw Error('profile capability');",
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
fn native_web_constructors_keep_subclasses_and_reentrant_headers() {
    let mut runtime = portable();
    eval(
        &mut runtime,
        r#"
class ExtendedHeaders extends Headers {}
const headers = new ExtendedHeaders([['a', 'one'], ['b', 'two']]);
if (!(headers instanceof ExtendedHeaders) || !(headers instanceof Headers)) throw Error('Headers prototype');
if (headers.get('absent') !== null) throw Error('missing header');
const pairs = headers.entries();
if (pairs[Symbol.iterator]() !== pairs || String(pairs.next().value) !== 'a,one') throw Error('header iterator');
headers.set('b', 'updated');
if (String(pairs.next().value) !== 'b,updated' || !pairs.next().done) throw Error('header iterator values');
headers.set({
  toString() {
    headers.append('nested', 'present');
    return 'outer';
  }
}, 'present');
if (!headers.has('outer') || !headers.has('nested')) throw Error('coercion reentry');
class ExtendedRequest extends Request {}
class ExtendedResponse extends Response {}
if (!(new ExtendedRequest('https://example.test') instanceof ExtendedRequest)) throw Error('Request prototype');
if (!(new ExtendedResponse() instanceof ExtendedResponse)) throw Error('Response prototype');
for (const [ctor, args] of [[Headers, []], [Response, []], [Request, ['https://example.test']]]) {
  try { ctor(...args); throw Error('constructor called without new'); }
  catch (error) { if (!(error instanceof TypeError)) throw error; }
}
class ExtendedSocket extends WebSocket {}
const socket = new ExtendedSocket('ws://example.test');
if (!(socket instanceof ExtendedSocket) || !(socket instanceof EventTarget)) throw Error('WebSocket prototype');
socket.close();
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
    let mut runtime = portable();
    eval(
        &mut runtime,
        r#"
const buffer = new ArrayBuffer(2, { maxByteLength: 8 });
const view = new Uint8Array(buffer);
buffer.resize(4);
view.set([4, 3, 2, 1]);
__host.http('POST', 'https://example.test/raw', '', view);
"#,
    );
    let jobs = runtime.take_jobs();
    let [IoJob::Http(request)] = &jobs[..] else {
        panic!("expected request")
    };
    assert_eq!(request.body, [4, 3, 2, 1]);
}
