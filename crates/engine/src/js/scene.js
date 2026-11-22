(function (global) {
  "use strict";
  var core = global.__scene;
  var typedPrototype = Object.getPrototypeOf(Uint8Array.prototype);
  var typedBuffer = Object.getOwnPropertyDescriptor(typedPrototype, "buffer").get;
  var typedOffset = Object.getOwnPropertyDescriptor(typedPrototype, "byteOffset").get;
  var typedLength = Object.getOwnPropertyDescriptor(typedPrototype, "byteLength").get;
  var applyIntrinsic = Reflect.apply;
  var ByteView = Uint8Array;
  core._bytesView = function (value) {
    var buffer;
    try { buffer = applyIntrinsic(typedBuffer, value, []); }
    catch (_) { return null; }
    return new ByteView(buffer, applyIntrinsic(typedOffset, value, []), applyIntrinsic(typedLength, value, []));
  };

  // An insertion range owns only its current ids. Geometry stays in Tree.
  function normalize(value, out, texts, previous, previousTexts, depth) {
    if (depth > 256) throw new RangeError("recursive JSX children");
    if (typeof value === "function") return normalize(value(), out, texts, previous, previousTexts, depth + 1);
    if (Array.isArray(value)) {
      for (var i = 0; i < value.length; i++) normalize(value[i], out, texts, previous, previousTexts, depth + 1);
    } else if (value !== undefined && value !== null && typeof value !== "boolean") {
      var native = core._isNode(value);
      var node;
      if (native) node = value;
      else if (previousTexts[out.length] && core._isNode(previous[out.length])) {
        node = previous[out.length];
        core.setText(node, String(value));
      } else node = core._createTextNode(String(value));
      texts.push(!native);
      out.push(node);
    }
  }
  core._insertRange = function (parent, children, marker) {
    if (arguments.length > 2 && core._isNode(children)) return core._insertNode(parent, children, marker);
    var state = { current: [], texts: [] };
    return core._reactiveCall("createRenderEffect", function () {
      var out = [], texts = [];
      normalize(children, out, texts, state.current, state.texts, 0);
      state.texts = texts;
      return out;
    }, function (nodes) {
      for (var i = 0; i < state.current.length; i++) {
        var node = state.current[i];
        if (nodes.indexOf(node) < 0 && core.parent(node) === parent) core.remove(parent, node);
      }
      var before = marker || 0;
      for (var i = nodes.length - 1; i >= 0; i--) {
        var node = nodes[i];
        if (core.parent(node) !== parent || core.nextSibling(node) !== before) core._insertNode(parent, node, before);
        before = node;
      }
      state.current = nodes;
    });
  };
  core._append = function (parent, child) {
    if (core._isNode(child)) return core._insertNode(parent, child, 0);
    return core._insertRange(parent, child, 0);
  };

  delete global.__scene;
})(globalThis);
