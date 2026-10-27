(function (global) {
  "use strict";

  var bridge = global.__uifw;
  var sockets = new Map();
  var modules = new Map();

  function AbortSignal() {
    this.aborted = false;
    this._fns = [];
  }
  AbortSignal.prototype.addEventListener = function (type, fn) {
    if (type === "abort") {
      this._fns.push(fn);
    }
  };
  function AbortController() {
    this.signal = new AbortSignal();
  }
  AbortController.prototype.abort = function () {
    var signal = this.signal;
    if (signal.aborted) {
      return;
    }
    signal.aborted = true;
    signal._fns.slice().forEach(function (fn) {
      fn();
    });
  };

  function encodeUtf8(text) {
    var bytes = [];
    var i = 0;
    while (i < text.length) {
      var code = text.charCodeAt(i);
      if (code >= 0xd800 && code <= 0xdbff && i + 1 < text.length) {
        var next = text.charCodeAt(i + 1);
        if (next >= 0xdc00 && next <= 0xdfff) {
          code = 0x10000 + ((code - 0xd800) << 10) + (next - 0xdc00);
          i++;
        }
      }
      if (code < 0x80) {
        bytes.push(code);
      } else if (code < 0x800) {
        bytes.push(0xc0 | (code >> 6), 0x80 | (code & 0x3f));
      } else if (code < 0x10000) {
        bytes.push(0xe0 | (code >> 12), 0x80 | ((code >> 6) & 0x3f), 0x80 | (code & 0x3f));
      } else {
        bytes.push(
          0xf0 | (code >> 18),
          0x80 | ((code >> 12) & 0x3f),
          0x80 | ((code >> 6) & 0x3f),
          0x80 | (code & 0x3f),
        );
      }
      i++;
    }
    return new Uint8Array(bytes);
  }

  function decodeUtf8(bytes) {
    var out = "";
    var i = 0;
    while (i < bytes.length) {
      var b0 = bytes[i];
      if (b0 < 0x80) {
        out += String.fromCharCode(b0);
        i += 1;
      } else if ((b0 & 0xe0) === 0xc0 && i + 1 < bytes.length) {
        out += String.fromCharCode(((b0 & 0x1f) << 6) | (bytes[i + 1] & 0x3f));
        i += 2;
      } else if ((b0 & 0xf0) === 0xe0 && i + 2 < bytes.length) {
        out += String.fromCharCode(
          ((b0 & 0x0f) << 12) | ((bytes[i + 1] & 0x3f) << 6) | (bytes[i + 2] & 0x3f),
        );
        i += 3;
      } else if ((b0 & 0xf8) === 0xf0 && i + 3 < bytes.length) {
        var point =
          ((b0 & 0x07) << 18) |
          ((bytes[i + 1] & 0x3f) << 12) |
          ((bytes[i + 2] & 0x3f) << 6) |
          (bytes[i + 3] & 0x3f);
        point -= 0x10000;
        out += String.fromCharCode(0xd800 + (point >> 10), 0xdc00 + (point & 0x3ff));
        i += 4;
      } else {
        out += "\ufffd";
        i += 1;
      }
    }
    return out;
  }

  function TextEncoder() {}
  TextEncoder.prototype.encode = function (input) {
    return encodeUtf8(input === undefined ? "" : String(input));
  };
  function TextDecoder() {}
  TextDecoder.prototype.decode = function (input) {
    if (input === undefined || input === null) {
      return "";
    }
    var bytes = input;
    if (input.buffer) {
      bytes = new Uint8Array(input.buffer, input.byteOffset || 0, input.byteLength);
    }
    return decodeUtf8(bytes);
  };

  function URLSearchParams(init) {
    this._pairs = [];
    if (typeof init === "string") {
      var text = init.charAt(0) === "?" ? init.slice(1) : init;
      if (text.length === 0) {
        return;
      }
      var parts = text.split("&");
      for (var index = 0; index < parts.length; index++) {
        var piece = parts[index];
        var eq = piece.indexOf("=");
        var key = eq < 0 ? piece : piece.slice(0, eq);
        var value = eq < 0 ? "" : piece.slice(eq + 1);
        this._pairs.push([decodeURIComponent(key.replace(/\+/g, " ")), decodeURIComponent(value.replace(/\+/g, " "))]);
      }
    }
  }
  URLSearchParams.prototype.get = function (name) {
    for (var index = 0; index < this._pairs.length; index++) {
      if (this._pairs[index][0] === name) {
        return this._pairs[index][1];
      }
    }
    return null;
  };
  URLSearchParams.prototype.set = function (name, value) {
    var kept = [];
    var replaced = false;
    for (var index = 0; index < this._pairs.length; index++) {
      if (this._pairs[index][0] === name) {
        if (!replaced) {
          kept.push([name, String(value)]);
          replaced = true;
        }
      } else {
        kept.push(this._pairs[index]);
      }
    }
    if (!replaced) {
      kept.push([name, String(value)]);
    }
    this._pairs = kept;
  };
  URLSearchParams.prototype.append = function (name, value) {
    this._pairs.push([String(name), String(value)]);
  };
  URLSearchParams.prototype.toString = function () {
    var out = [];
    for (var index = 0; index < this._pairs.length; index++) {
      out.push(encodeURIComponent(this._pairs[index][0]) + "=" + encodeURIComponent(this._pairs[index][1]));
    }
    return out.join("&");
  };

  function parseAbsolute(text) {
    var match = /^([A-Za-z][A-Za-z0-9+.-]*):\/\/([^/?#]*)([^?#]*)(\?[^#]*)?(#.*)?$/.exec(text);
    if (!match) {
      return null;
    }
    return {
      protocol: match[1].toLowerCase() + ":",
      host: match[2],
      pathname: match[3].length === 0 ? "/" : match[3],
      search: match[4] || "",
      hash: match[5] || "",
    };
  }

  function URL(input, base) {
    var text = String(input);
    if (base !== undefined && !/^[A-Za-z][A-Za-z0-9+.-]*:/.test(text)) {
      var root = String(base);
      if (text.charAt(0) === "/") {
        var origin = root.match(/^([A-Za-z][A-Za-z0-9+.-]*:\/\/[^/?#]*)/);
        text = (origin ? origin[1] : "") + text;
      } else {
        var slash = root.lastIndexOf("/");
        text = root.slice(0, slash + 1) + text;
      }
    }
    var parts = parseAbsolute(text);
    if (!parts || (parts.protocol !== "http:" && parts.protocol !== "https:" && parts.protocol !== "ws:" && parts.protocol !== "wss:")) {
      throw new TypeError("Invalid URL");
    }
    this.protocol = parts.protocol;
    this.host = parts.host;
    this.pathname = parts.pathname;
    this.search = parts.search;
    this.hash = parts.hash;
    this.searchParams = new URLSearchParams(parts.search);
    this.href = parts.protocol + "//" + parts.host + parts.pathname + parts.search + parts.hash;
  }
  URL.prototype.toString = function () {
    return this.href;
  };

  function Headers(init) {
    this._map = new Map();
    if (typeof init === "string" && init.length > 0) {
      var lines = init.split("\n");
      for (var index = 0; index < lines.length; index++) {
        var line = lines[index];
        var colon = line.indexOf(":");
        if (colon > 0) {
          this.append(line.slice(0, colon), line.slice(colon + 1).trim());
        }
      }
    }
  }
  Headers.prototype.append = function (name, value) {
    var key = String(name).toLowerCase();
    var previous = this._map.get(key);
    this._map.set(key, previous ? previous + ", " + String(value) : String(value));
  };
  Headers.prototype.set = function (name, value) {
    this._map.set(String(name).toLowerCase(), String(value));
  };
  Headers.prototype.get = function (name) {
    var value = this._map.get(String(name).toLowerCase());
    return value === undefined ? null : value;
  };
  Headers.prototype.has = function (name) {
    return this._map.has(String(name).toLowerCase());
  };
  Headers.prototype.forEach = function (fn) {
    this._map.forEach(function (value, key) {
      fn(value, key);
    });
  };

  function headerBlock(headers) {
    var lines = [];
    if (!headers) {
      return "";
    }
    headers.forEach(function (value, key) {
      lines.push(key + ": " + value);
    });
    return lines.join("\n");
  }

  function Request(input, init) {
    init = init || {};
    if (typeof input === "string") {
      this.url = new URL(input).href;
      this.method = "GET";
      this.headers = new Headers();
      this._body = null;
    } else {
      this.url = input.url;
      this.method = input.method;
      this.headers = new Headers();
      input.headers.forEach(function (value, key) {
        this.append(key, value);
      }, this.headers);
      this._body = input._body;
    }
    if (init.method) {
      this.method = String(init.method).toUpperCase();
    }
    if (init.headers) {
      var extra = new Headers();
      if (typeof init.headers === "string") {
        extra = new Headers(init.headers);
      } else if (Array.isArray(init.headers)) {
        for (var index = 0; index < init.headers.length; index++) {
          extra.append(init.headers[index][0], init.headers[index][1]);
        }
      } else {
        for (var key in init.headers) {
          if (Object.prototype.hasOwnProperty.call(init.headers, key)) {
            extra.set(key, init.headers[key]);
          }
        }
      }
      extra.forEach(function (value, key) {
        this.set(key, value);
      }, this.headers);
    }
    if (init.body !== undefined && init.body !== null) {
      this._body = typeof init.body === "string" ? init.body : init.body;
    }
    this.signal = init.signal || null;
  }

  function Response(body, init) {
    init = init || {};
    this.status = init.status || 200;
    this.ok = this.status >= 200 && this.status < 300;
    this.headers = new Headers(init.headers || "");
    this._bytes = body || new Uint8Array(0);
    this.bodyUsed = false;
  }
  Response.prototype._take = function () {
    if (this.bodyUsed) {
      return Promise.reject(new TypeError("body used"));
    }
    this.bodyUsed = true;
    return Promise.resolve(this._bytes);
  };
  Response.prototype.text = function () {
    var self = this;
    return this._take().then(function (bytes) {
      return new TextDecoder().decode(bytes);
    });
  };
  Response.prototype.json = function () {
    return this.text().then(function (text) {
      return JSON.parse(text);
    });
  };
  Response.prototype.arrayBuffer = function () {
    return this._take().then(function (bytes) {
      var copy = bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
      return copy;
    });
  };

  function fetch(input, init) {
    var request = new Request(input, init);
    if (request.signal && request.signal.aborted) {
      return Promise.reject(new Error("AbortError"));
    }
    var body = request._body;
    if (body instanceof Uint8Array) {
      body = body.buffer.slice(body.byteOffset, body.byteOffset + body.byteLength);
    }
    var promise = bridge.http(request.method, request.url, headerBlock(request.headers), body);
    if (request.signal) {
      request.signal.addEventListener("abort", function () {
        bridge.abort(promise.token);
      });
    }
    return promise.then(function (raw) {
      var bytes = new Uint8Array(raw.body);
      return new Response(bytes, { status: raw.status, headers: raw.headers });
    });
  }

  function WebSocket(url) {
    var parsed = new URL(url);
    if (parsed.protocol !== "ws:" && parsed.protocol !== "wss:") {
      throw new TypeError("Invalid URL");
    }
    this.url = parsed.href;
    this.readyState = 0;
    this.onopen = null;
    this.onmessage = null;
    this.onerror = null;
    this.onclose = null;
    this._id = bridge.socketOpen(parsed.href);
    sockets.set(this._id, this);
  }
  WebSocket.prototype.send = function (data) {
    if (typeof data === "string") {
      bridge.socketSend(this._id, data);
      return;
    }
    bridge.socketSend(this._id, data);
  };
  WebSocket.prototype.close = function () {
    this.readyState = 2;
    bridge.socketClose(this._id);
  };
  WebSocket.prototype.addEventListener = function (type, fn) {
    this["on" + type] = fn;
  };

  bridge._socket = function (id, event, data) {
    var socket = sockets.get(id);
    if (!socket) {
      return;
    }
    if (event === "open") {
      socket.readyState = 1;
      if (socket.onopen) {
        socket.onopen({});
      }
    } else if (event === "message") {
      if (socket.onmessage) {
        socket.onmessage({ data: data });
      }
    } else if (event === "error") {
      socket.readyState = 3;
      if (socket.onerror) {
        socket.onerror({});
      }
    } else if (event === "close") {
      socket.readyState = 3;
      if (socket.onclose) {
        socket.onclose({});
      }
    }
  };

  function getModule(name) {
    if (modules.has(name)) {
      return modules.get(name);
    }
    var created = bridge.module(name);
    if (!created) {
      return null;
    }
    modules.set(name, created);
    return created;
  }

  global.AbortController = AbortController;
  global.AbortSignal = AbortSignal;
  global.TextEncoder = TextEncoder;
  global.TextDecoder = TextDecoder;
  global.URL = URL;
  global.URLSearchParams = URLSearchParams;
  global.Headers = Headers;
  global.Request = Request;
  global.Response = Response;
  global.fetch = fetch;
  global.WebSocket = WebSocket;
  global.TurboModuleRegistry = {
    get: getModule,
    getEnforcing: function (name) {
      var mod = getModule(name);
      if (!mod) {
        throw new Error("module \"" + name + "\" is not linked");
      }
      return mod;
    },
  };
})(globalThis);
