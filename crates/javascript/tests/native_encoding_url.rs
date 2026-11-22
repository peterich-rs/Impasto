#![cfg(feature = "runtime")]

use javascript::runtime::Runtime;
use javascript::standard::StandardProfile;

#[test]
fn native_web_constructors_keep_subclass_and_live_iterator_semantics() {
    let result = Runtime::builder()
        .standard(StandardProfile)
        .build()
        .and_then(|mut runtime| {
            runtime.eval_script(
                r#"
for (const ctor of [URL, URLSearchParams, TextEncoder, TextDecoder]) {
  try {
    ctor.call(ctor, 'https://example.test');
    throw new Error('called without new: ' + ctor.name);
  } catch (error) {
    if (!(error instanceof TypeError)) throw error;
  }
}
class AppURL extends URL {}
class AppEncoder extends TextEncoder {}
const url = new AppURL('https://example.test/?first=1');
if (!(url instanceof AppURL) || !(url instanceof URL)) throw new Error('URL new.target');
if (new AppEncoder().encode('A')[0] !== 65) throw new Error('encoder new.target');
const params = url.searchParams;
const entries = params.entries();
if (params[Symbol.iterator] !== params.entries || entries[Symbol.iterator]() !== entries) {
  throw new Error('iteration protocol');
}
if (String(entries.next().value) !== 'first,1') throw new Error('first entry');
params.append('next', '2');
if (String(entries.next().value) !== 'next,2' || !entries.next().done) {
  throw new Error('iterator did not see mutation');
}
url.search = '?last=3';
if (!entries.next().done || url.searchParams !== params || params.get('last') !== '3') {
  throw new Error('iterator completion or linked params identity');
}
const seen = [];
params.forEach((value, key, owner) => {
  seen.push(key + value);
  if (key === 'last') owner.append('extra', 4);
});
if (seen.join(',') !== 'last3,extra4') throw new Error('reentrant forEach');
let converted = false;
const pair = function* () {
  yield { toString() { converted = true; return 'name\ud800'; } };
  yield converted ? 'first' : 'early';
};
const input = function* () {
  yield pair();
  yield ['next', converted ? 'second' : 'early'];
};
if (String(new URLSearchParams(input())) !== 'name%EF%BF%BD=first&next=second') {
  throw new Error('iterable coercion order');
}
let closed = false;
const broken = function* () {
  try { yield ['too few']; } finally { closed = true; }
};
try { new URLSearchParams(broken()); } catch (error) {
  if (!(error instanceof TypeError)) throw error;
}
if (!closed) throw new Error('iterator not closed after invalid tuple');
"#,
                "native-constructors.js",
            )
        });
    assert!(result.is_ok(), "{result:?}");
}

#[test]
fn utf8_buffers_use_native_ranges_after_prototype_mutation() {
    let result = Runtime::builder()
        .standard(StandardProfile)
        .build()
        .and_then(|mut runtime| {
            runtime.eval_script(
                r#"
const bytes = new Uint8Array([255, 0x41, 0x42, 255]);
const view = new DataView(bytes.buffer, 1, 2);
const poison = { get() { throw new Error('read from user property'); } };
Object.defineProperty(view, 'buffer', poison);
Object.defineProperty(view, 'byteOffset', poison);
Object.defineProperty(view, 'byteLength', poison);
const decoder = new TextDecoder();
if (decoder.decode(view) !== 'AB') throw new Error('wrong byte range');
const dynamic = new ArrayBuffer(2, {maxByteLength: 4});
const tracking = new Uint8Array(dynamic);
tracking.set([65, 66]);
dynamic.resize(4);
tracking.set([67, 68], 2);
if (decoder.decode(tracking) !== 'ABCD') throw new Error('stale grown range');
dynamic.resize(1);
if (decoder.decode(tracking) !== 'A') throw new Error('stale shrunken range');
for (const input of ['\ud800', '\udc00', '\ud800\ud800']) {
  if (decoder.decode(new TextEncoder().encode(input)) !== input.replace(/./g, '\ufffd')) {
    throw new Error('unpaired surrogate conversion');
  }
}
"#,
                "native-encoding.js",
            )
        });
    assert!(result.is_ok(), "{result:?}");
}
