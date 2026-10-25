/* Timers, language globals, and promise completion. Included from glue.c. */

#define TIMER_TIMEOUT 0
#define TIMER_INTERVAL 1
#define TIMER_RAF 2
#define TIMER_FIRE_CAP 64

static int grow_timers(Host *host) {
    size_t cap = host->timer_cap ? host->timer_cap * 2 : 8;
    Timer *next = realloc(host->timers, cap * sizeof(Timer));
    if (!next) {
        return -1;
    }
    host->timers = next;
    host->timer_cap = cap;
    return 0;
}

static int grow_promises(Host *host) {
    size_t cap = host->promise_cap ? host->promise_cap * 2 : 8;
    PromiseSlot *next = realloc(host->promises, cap * sizeof(PromiseSlot));
    if (!next) {
        return -1;
    }
    host->promises = next;
    host->promise_cap = cap;
    return 0;
}

static void free_timers(Host *host) {
    if (!host->ctx) {
        free(host->timers);
        host->timers = NULL;
        host->timer_len = 0;
        return;
    }
    for (size_t index = 0; index < host->timer_len; index++) {
        JS_FreeValue(host->ctx, host->timers[index].fn);
        JS_FreeValue(host->ctx, host->timers[index].extra);
    }
    free(host->timers);
    host->timers = NULL;
    host->timer_len = 0;
}

static void free_promises(Host *host) {
    if (!host->ctx) {
        free(host->promises);
        host->promises = NULL;
        host->promise_len = 0;
        return;
    }
    for (size_t index = 0; index < host->promise_len; index++) {
        if (!host->promises[index].live) {
            continue;
        }
        JS_FreeValue(host->ctx, host->promises[index].resolve);
        JS_FreeValue(host->ctx, host->promises[index].reject);
        host->promises[index].live = 0;
    }
    free(host->promises);
    host->promises = NULL;
    host->promise_len = 0;
}

static Timer *timer_by_id(Host *host, uint64_t id) {
    for (size_t index = 0; index < host->timer_len; index++) {
        if (host->timers[index].active && host->timers[index].id == id) {
            return &host->timers[index];
        }
    }
    return NULL;
}

static void compact_timers(Host *host) {
    size_t write = 0;
    for (size_t index = 0; index < host->timer_len; index++) {
        if (host->timers[index].active) {
            if (write != index) {
                host->timers[write] = host->timers[index];
            }
            write++;
            continue;
        }
        JS_FreeValue(host->ctx, host->timers[index].fn);
        JS_FreeValue(host->ctx, host->timers[index].extra);
        host->timers[index].fn = JS_UNDEFINED;
        host->timers[index].extra = JS_UNDEFINED;
    }
    host->timer_len = write;
}

static uint64_t arm_timer(Host *host, JSValue fn, JSValue extra, int kind, int64_t delay_ms) {
    if (host->timer_len == host->timer_cap && grow_timers(host)) {
        JS_FreeValue(host->ctx, fn);
        JS_FreeValue(host->ctx, extra);
        return 0;
    }
    if (delay_ms < 0) {
        delay_ms = 0;
    }
    host->timer_next++;
    Timer *timer = &host->timers[host->timer_len++];
    timer->id = host->timer_next;
    timer->due_ms = kind == TIMER_RAF ? host->mono_ms : host->mono_ms + delay_ms;
    timer->interval_ms = kind == TIMER_INTERVAL ? delay_ms : 0;
    timer->min_frame = host->frame_seq + 1;
    timer->fn = fn;
    timer->extra = extra;
    timer->kind = kind;
    timer->active = 1;
    if (kind == TIMER_RAF || delay_ms <= 0) {
        host->wants_frame = 1;
    }
    return timer->id;
}

static JSValue copy_extra(JSContext *ctx, int argc, JSValueConst *argv) {
    JSValue extra = JS_NewArray(ctx);
    if (JS_IsException(extra)) {
        return JS_UNDEFINED;
    }
    uint32_t count = 0;
    for (int index = 2; index < argc; index++) {
        JS_SetPropertyUint32(ctx, extra, count, JS_DupValue(ctx, argv[index]));
        count++;
    }
    return extra;
}

static JSValue js_set_timer(JSContext *ctx, int argc, JSValueConst *argv, int kind) {
    Host *host = host_of(ctx);
    if (!host || argc < 1 || !JS_IsFunction(ctx, argv[0])) {
        return throw_msg(ctx, "timer function");
    }
    int64_t delay = 0;
    if (argc >= 2 && JS_ToInt64(ctx, &delay, argv[1])) {
        return JS_EXCEPTION;
    }
    JSValue fn = JS_DupValue(ctx, argv[0]);
    JSValue extra = copy_extra(ctx, argc, argv);
    uint64_t id = arm_timer(host, fn, extra, kind, delay);
    if (id == 0) {
        return throw_msg(ctx, "timer");
    }
    return JS_NewFloat64(ctx, (double)id);
}

static JSValue js_set_timeout(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    return js_set_timer(ctx, argc, argv, TIMER_TIMEOUT);
}

static JSValue js_set_interval(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    return js_set_timer(ctx, argc, argv, TIMER_INTERVAL);
}

static void clear_timer(Host *host, uint64_t id, int kind) {
    Timer *timer = timer_by_id(host, id);
    if (!timer || timer->kind != kind) {
        return;
    }
    timer->active = 0;
}

static JSValue js_clear_timer(JSContext *ctx, int argc, JSValueConst *argv, int kind) {
    Host *host = host_of(ctx);
    if (!host || argc < 1) {
        return JS_UNDEFINED;
    }
    int64_t id = 0;
    if (JS_ToInt64(ctx, &id, argv[0])) {
        JS_FreeValue(ctx, JS_GetException(ctx));
        return JS_UNDEFINED;
    }
    if (id > 0) {
        clear_timer(host, (uint64_t)id, kind);
    }
    return JS_UNDEFINED;
}

static JSValue js_clear_timeout(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    return js_clear_timer(ctx, argc, argv, TIMER_TIMEOUT);
}

static JSValue js_clear_interval(JSContext *ctx, JSValueConst this_val, int argc,
                                 JSValueConst *argv) {
    (void)this_val;
    return js_clear_timer(ctx, argc, argv, TIMER_INTERVAL);
}

static JSValue js_request_animation_frame(JSContext *ctx, JSValueConst this_val, int argc,
                                          JSValueConst *argv) {
    (void)this_val;
    return js_set_timer(ctx, argc, argv, TIMER_RAF);
}

static JSValue js_cancel_animation_frame(JSContext *ctx, JSValueConst this_val, int argc,
                                         JSValueConst *argv) {
    (void)this_val;
    return js_clear_timer(ctx, argc, argv, TIMER_RAF);
}

static JSValue js_performance_now(JSContext *ctx, JSValueConst this_val, int argc,
                                  JSValueConst *argv) {
    (void)this_val;
    (void)argc;
    (void)argv;
    Host *host = host_of(ctx);
    if (!host) {
        return JS_NewFloat64(ctx, 0);
    }
    return JS_NewFloat64(ctx, (double)(host->mono_ms - host->origin_ms));
}

static uint64_t alloc_token(Host *host) {
    host->token_next++;
    if (host->token_next == 0) {
        host->token_next = 1;
    }
    return host->token_next;
}

static int store_promise(Host *host, uint64_t token, JSValue resolve, JSValue reject) {
    for (size_t index = 0; index < host->promise_len; index++) {
        if (!host->promises[index].live) {
            host->promises[index].token = token;
            host->promises[index].resolve = resolve;
            host->promises[index].reject = reject;
            host->promises[index].live = 1;
            return 0;
        }
    }
    if (host->promise_len == host->promise_cap && grow_promises(host)) {
        return -1;
    }
    PromiseSlot *slot = &host->promises[host->promise_len++];
    slot->token = token;
    slot->resolve = resolve;
    slot->reject = reject;
    slot->live = 1;
    return 0;
}

static PromiseSlot *promise_by_token(Host *host, uint64_t token) {
    for (size_t index = 0; index < host->promise_len; index++) {
        if (host->promises[index].live && host->promises[index].token == token) {
            return &host->promises[index];
        }
    }
    return NULL;
}

static int settle(Host *host, uint64_t token, int ok, JSValue value) {
    PromiseSlot *slot = promise_by_token(host, token);
    if (!slot) {
        JS_FreeValue(host->ctx, value);
        return 0;
    }
    JSValue fn = ok ? slot->resolve : slot->reject;
    JSValue other = ok ? slot->reject : slot->resolve;
    slot->live = 0;
    JSValue result = JS_Call(host->ctx, fn, JS_UNDEFINED, 1, &value);
    JS_FreeValue(host->ctx, value);
    JS_FreeValue(host->ctx, fn);
    JS_FreeValue(host->ctx, other);
    if (JS_IsException(result)) {
        JSValue exception = JS_GetException(host->ctx);
        JS_FreeValue(host->ctx, exception);
        return -1;
    }
    JS_FreeValue(host->ctx, result);
    return 0;
}

static JSValue read_body(JSContext *ctx, JSValueConst value, const uint8_t **out, size_t *len) {
    *out = NULL;
    *len = 0;
    if (JS_IsUndefined(value) || JS_IsNull(value)) {
        return JS_UNDEFINED;
    }
    if (JS_IsString(value)) {
        size_t size = 0;
        const char *text = JS_ToCStringLen(ctx, &size, value);
        if (!text) {
            return JS_EXCEPTION;
        }
        *out = (const uint8_t *)text;
        *len = size;
        return JS_UNDEFINED;
    }
    size_t size = 0;
    uint8_t *bytes = JS_GetArrayBuffer(ctx, &size, value);
    if (bytes) {
        *out = bytes;
        *len = size;
        return JS_UNDEFINED;
    }
    JS_FreeValue(ctx, JS_GetException(ctx));
    return throw_msg(ctx, "body");
}

static void release_body(JSContext *ctx, JSValueConst value, const uint8_t *bytes) {
    if (bytes && JS_IsString(value)) {
        JS_FreeCString(ctx, (const char *)bytes);
    }
}

static JSValue make_promise(Host *host, uint64_t *token_out) {
    JSValue funcs[2];
    JSValue promise = JS_NewPromiseCapability(host->ctx, funcs);
    if (JS_IsException(promise)) {
        return promise;
    }
    uint64_t token = alloc_token(host);
    if (store_promise(host, token, funcs[0], funcs[1])) {
        JS_FreeValue(host->ctx, funcs[0]);
        JS_FreeValue(host->ctx, funcs[1]);
        JS_FreeValue(host->ctx, promise);
        return throw_msg(host->ctx, "promise");
    }
    JS_DefinePropertyValueStr(host->ctx, promise, "token", JS_NewFloat64(host->ctx, (double)token),
                              JS_PROP_C_W_E);
    *token_out = token;
    return promise;
}

static JSValue js_http(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (!host || argc < 2) {
        return throw_msg(ctx, "http(method, url)");
    }
    uint64_t token = 0;
    JSValue promise = make_promise(host, &token);
    if (JS_IsException(promise)) {
        return promise;
    }
    size_t method_len = 0;
    size_t url_len = 0;
    size_t headers_len = 0;
    const char *method = JS_ToCStringLen(ctx, &method_len, argv[0]);
    const char *url = JS_ToCStringLen(ctx, &url_len, argv[1]);
    const char *headers = "";
    if (argc >= 3 && !JS_IsUndefined(argv[2]) && !JS_IsNull(argv[2])) {
        headers = JS_ToCStringLen(ctx, &headers_len, argv[2]);
    }
    const uint8_t *body = NULL;
    size_t body_len = 0;
    JSValue body_error = JS_UNDEFINED;
    if (argc >= 4) {
        body_error = read_body(ctx, argv[3], &body, &body_len);
    }
    if (!method || !url || !headers || JS_IsException(body_error)) {
        if (method) {
            JS_FreeCString(ctx, method);
        }
        if (url) {
            JS_FreeCString(ctx, url);
        }
        if (headers && headers_len > 0) {
            JS_FreeCString(ctx, headers);
        }
        JS_FreeValue(ctx, body_error);
        settle(host, token, 0, JS_NewString(ctx, "http"));
        return promise;
    }
    if (host->fns.http) {
        host->fns.http(host->fns.session, token, method, method_len, url, url_len, headers,
                       headers_len, body, body_len);
    } else {
        settle(host, token, 0, JS_NewString(ctx, "http is not linked"));
    }
    if (argc >= 4) {
        release_body(ctx, argv[3], body);
    }
    JS_FreeCString(ctx, method);
    JS_FreeCString(ctx, url);
    if (headers_len > 0) {
        JS_FreeCString(ctx, headers);
    }
    return promise;
}

static JSValue js_abort(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (!host || argc < 1) {
        return JS_UNDEFINED;
    }
    int64_t token = 0;
    if (JS_ToInt64(ctx, &token, argv[0])) {
        JS_FreeValue(ctx, JS_GetException(ctx));
        return JS_UNDEFINED;
    }
    if (token > 0 && host->fns.abort_token) {
        host->fns.abort_token(host->fns.session, (uint64_t)token);
    }
    return JS_UNDEFINED;
}

static JSValue js_has_module(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (!host || argc < 1 || !host->fns.module_has) {
        return JS_FALSE;
    }
    size_t len = 0;
    const char *name = JS_ToCStringLen(ctx, &len, argv[0]);
    if (!name) {
        return JS_EXCEPTION;
    }
    int present = host->fns.module_has(host->fns.session, name, len);
    JS_FreeCString(ctx, name);
    return JS_NewBool(ctx, present);
}

static JSValue js_module_call(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (!host || argc < 3) {
        return throw_msg(ctx, "call(name, method, json)");
    }
    uint64_t token = 0;
    JSValue promise = make_promise(host, &token);
    if (JS_IsException(promise)) {
        return promise;
    }
    size_t name_len = 0;
    size_t method_len = 0;
    size_t json_len = 0;
    const char *name = JS_ToCStringLen(ctx, &name_len, argv[0]);
    const char *method = JS_ToCStringLen(ctx, &method_len, argv[1]);
    const char *json = JS_ToCStringLen(ctx, &json_len, argv[2]);
    if (!name || !method || !json) {
        if (name) {
            JS_FreeCString(ctx, name);
        }
        if (method) {
            JS_FreeCString(ctx, method);
        }
        if (json) {
            JS_FreeCString(ctx, json);
        }
        settle(host, token, 0, JS_NewString(ctx, "module"));
        return promise;
    }
    if (host->fns.module_call) {
        host->fns.module_call(host->fns.session, token, name, name_len, method, method_len, json,
                              json_len);
    } else {
        settle(host, token, 0, JS_NewString(ctx, "module is not linked"));
    }
    JS_FreeCString(ctx, name);
    JS_FreeCString(ctx, method);
    JS_FreeCString(ctx, json);
    return promise;
}

static JSValue js_socket_open(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (!host || argc < 1) {
        return throw_msg(ctx, "socket url");
    }
    size_t len = 0;
    const char *url = JS_ToCStringLen(ctx, &len, argv[0]);
    if (!url) {
        return JS_EXCEPTION;
    }
    host->socket_next++;
    if (host->socket_next == 0) {
        host->socket_next = 1;
    }
    uint64_t id = host->socket_next;
    if (host->fns.socket_open) {
        host->fns.socket_open(host->fns.session, id, url, len);
    }
    JS_FreeCString(ctx, url);
    return JS_NewFloat64(ctx, (double)id);
}

static JSValue js_socket_send(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (!host || argc < 2 || !host->fns.socket_send) {
        return JS_UNDEFINED;
    }
    int64_t id = 0;
    if (JS_ToInt64(ctx, &id, argv[0]) || id <= 0) {
        return JS_UNDEFINED;
    }
    if (JS_IsString(argv[1])) {
        size_t len = 0;
        const char *text = JS_ToCStringLen(ctx, &len, argv[1]);
        if (!text) {
            return JS_EXCEPTION;
        }
        host->fns.socket_send(host->fns.session, (uint64_t)id, (const uint8_t *)text, len, 1);
        JS_FreeCString(ctx, text);
        return JS_UNDEFINED;
    }
    size_t len = 0;
    uint8_t *bytes = JS_GetArrayBuffer(ctx, &len, argv[1]);
    if (!bytes) {
        JS_FreeValue(ctx, JS_GetException(ctx));
        return JS_UNDEFINED;
    }
    host->fns.socket_send(host->fns.session, (uint64_t)id, bytes, len, 0);
    return JS_UNDEFINED;
}

static JSValue js_socket_close(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (!host || argc < 1 || !host->fns.socket_close) {
        return JS_UNDEFINED;
    }
    int64_t id = 0;
    if (JS_ToInt64(ctx, &id, argv[0]) || id <= 0) {
        return JS_UNDEFINED;
    }
    host->fns.socket_close(host->fns.session, (uint64_t)id);
    return JS_UNDEFINED;
}

static int bind_global_fn(JSContext *ctx, JSValue global, const char *name, JSCFunction *fn,
                          int length) {
    return bind_fn(ctx, global, name, fn, length);
}

static int bind_host_globals(Host *host) {
    JSContext *ctx = host->ctx;
    JSValue global = JS_GetGlobalObject(ctx);
    int failed = bind_global_fn(ctx, global, "setTimeout", js_set_timeout, 2);
    failed |= bind_global_fn(ctx, global, "clearTimeout", js_clear_timeout, 1);
    failed |= bind_global_fn(ctx, global, "setInterval", js_set_interval, 2);
    failed |= bind_global_fn(ctx, global, "clearInterval", js_clear_interval, 1);
    failed |= bind_global_fn(ctx, global, "requestAnimationFrame", js_request_animation_frame, 1);
    failed |= bind_global_fn(ctx, global, "cancelAnimationFrame", js_cancel_animation_frame, 1);
    JSValue performance = JS_NewObject(ctx);
    failed |= bind_fn(ctx, performance, "now", js_performance_now, 0);
    failed |= JS_SetPropertyStr(ctx, global, "performance", performance) < 0;
    JSValue bridge = JS_NewObject(ctx);
    failed |= bind_fn(ctx, bridge, "http", js_http, 4);
    failed |= bind_fn(ctx, bridge, "abort", js_abort, 1);
    failed |= bind_fn(ctx, bridge, "hasModule", js_has_module, 1);
    failed |= bind_fn(ctx, bridge, "call", js_module_call, 3);
    failed |= bind_fn(ctx, bridge, "socketOpen", js_socket_open, 1);
    failed |= bind_fn(ctx, bridge, "socketSend", js_socket_send, 2);
    failed |= bind_fn(ctx, bridge, "socketClose", js_socket_close, 1);
    failed |= JS_SetPropertyStr(ctx, global, "__uifw", bridge) < 0;
    JS_FreeValue(ctx, global);
    return failed ? -1 : 0;
}

static int call_timer(Host *host, Timer *timer, char *err, size_t err_len) {
    JSValue fn = JS_DupValue(host->ctx, timer->fn);
    JSValue extra = JS_DupValue(host->ctx, timer->extra);
    if (timer->kind == TIMER_TIMEOUT || timer->kind == TIMER_RAF) {
        timer->active = 0;
    } else {
        int64_t interval = timer->interval_ms > 0 ? timer->interval_ms : 0;
        timer->due_ms = host->mono_ms + interval;
        timer->min_frame = host->frame_seq + 1;
    }
    int argc = 0;
    JSValue args[8];
    if (!JS_IsUndefined(extra)) {
        JSValue length = JS_GetPropertyStr(host->ctx, extra, "length");
        int64_t count = 0;
        if (!JS_IsException(length)) {
            JS_ToInt64(host->ctx, &count, length);
        }
        JS_FreeValue(host->ctx, length);
        if (count > 8) {
            count = 8;
        }
        for (int64_t index = 0; index < count; index++) {
            args[argc++] = JS_GetPropertyUint32(host->ctx, extra, (uint32_t)index);
        }
    }
    JSValue result = JS_Call(host->ctx, fn, JS_UNDEFINED, argc, argc ? args : NULL);
    for (int index = 0; index < argc; index++) {
        JS_FreeValue(host->ctx, args[index]);
    }
    JS_FreeValue(host->ctx, fn);
    JS_FreeValue(host->ctx, extra);
    if (!JS_IsException(result)) {
        JS_FreeValue(host->ctx, result);
        return 0;
    }
    if (err && err_len > 0 && err[0] == 0) {
        write_exception(host->ctx, err, err_len);
    } else {
        JSValue exception = JS_GetException(host->ctx);
        JS_FreeValue(host->ctx, exception);
    }
    return -1;
}

void qjs_set_clock(Host *host, int64_t mono_ms, int paused) {
    if (!host) {
        return;
    }
    host->mono_ms = mono_ms;
    host->paused = paused ? 1 : 0;
    host->frame_seq++;
    if (!host->origin_set) {
        host->origin_ms = mono_ms;
        host->origin_set = 1;
    }
}

int qjs_fire_timers(Host *host, char *err, size_t err_len) {
    if (!host || host->paused) {
        return 0;
    }
    if (err && err_len > 0) {
        err[0] = 0;
    }
    int failed = 0;
    int fired = 0;
    while (fired < TIMER_FIRE_CAP) {
        Timer *found = NULL;
        for (size_t index = 0; index < host->timer_len; index++) {
            Timer *timer = &host->timers[index];
            if (!timer->active || timer->kind == TIMER_RAF) {
                continue;
            }
            if (timer->min_frame > host->frame_seq || timer->due_ms > host->mono_ms) {
                continue;
            }
            found = timer;
            break;
        }
        if (!found) {
            break;
        }
        if (call_timer(host, found, err, err_len)) {
            failed = 1;
        }
        fired++;
    }
    compact_timers(host);
    for (size_t index = 0; index < host->timer_len; index++) {
        Timer *timer = &host->timers[index];
        if (timer->active && timer->kind != TIMER_RAF && timer->min_frame <= host->frame_seq &&
            timer->due_ms <= host->mono_ms) {
            host->wants_frame = 1;
            break;
        }
    }
    return failed ? -1 : 0;
}

int qjs_fire_raf(Host *host, char *err, size_t err_len) {
    if (!host || host->paused) {
        return 0;
    }
    if (err && err_len > 0) {
        err[0] = 0;
    }
    int failed = 0;
    for (size_t index = 0; index < host->timer_len; index++) {
        Timer *timer = &host->timers[index];
        if (!timer->active || timer->kind != TIMER_RAF || timer->min_frame > host->frame_seq) {
            continue;
        }
        if (call_timer(host, timer, err, err_len)) {
            failed = 1;
        }
    }
    compact_timers(host);
    return failed ? -1 : 0;
}

int qjs_needs_frame(Host *host) {
    if (!host || host->paused) {
        return 0;
    }
    for (size_t index = 0; index < host->timer_len; index++) {
        Timer *timer = &host->timers[index];
        if (!timer->active) {
            continue;
        }
        if (timer->kind == TIMER_RAF) {
            return 1;
        }
        if (timer->min_frame <= host->frame_seq + 1 && timer->due_ms <= host->mono_ms) {
            return 1;
        }
    }
    return 0;
}

int64_t qjs_next_deadline(Host *host) {
    if (!host || host->paused) {
        return -1;
    }
    int64_t best = -1;
    for (size_t index = 0; index < host->timer_len; index++) {
        Timer *timer = &host->timers[index];
        if (!timer->active || timer->kind == TIMER_RAF || timer->due_ms <= host->mono_ms) {
            continue;
        }
        if (best < 0 || timer->due_ms < best) {
            best = timer->due_ms;
        }
    }
    return best;
}

int qjs_settle_json(Host *host, uint64_t token, int ok, const char *text, size_t len) {
    if (!host || !host->ctx) {
        return -1;
    }
    JSValue value;
    if (ok) {
        value = JS_ParseJSON(host->ctx, text ? text : "null", text ? len : 4, "<plugin>");
        if (JS_IsException(value)) {
            JSValue exception = JS_GetException(host->ctx);
            JS_FreeValue(host->ctx, exception);
            value = JS_NULL;
        }
    } else {
        value = JS_NewStringLen(host->ctx, text ? text : "", text ? len : 0);
    }
    return settle(host, token, ok, value);
}

int qjs_settle_http(Host *host, uint64_t token, int status, const char *headers, size_t headers_len,
                    const uint8_t *body, size_t body_len, const char *error) {
    if (!host || !host->ctx) {
        return -1;
    }
    if (error) {
        return settle(host, token, 0, JS_NewString(host->ctx, error));
    }
    if (!body) {
        body = (const uint8_t *)"";
    }
    JSValue object = JS_NewObject(host->ctx);
    JS_SetPropertyStr(host->ctx, object, "status", JS_NewInt32(host->ctx, status));
    JS_SetPropertyStr(host->ctx, object, "headers",
                      JS_NewStringLen(host->ctx, headers ? headers : "", headers_len));
    JS_SetPropertyStr(host->ctx, object, "body", JS_NewArrayBufferCopy(host->ctx, body, body_len));
    return settle(host, token, 1, object);
}

int qjs_emit(Host *host, const char *name, const char *event, const char *json, size_t len) {
    if (!host || !host->ctx) {
        return -1;
    }
    JSValue global = JS_GetGlobalObject(host->ctx);
    JSValue bridge = JS_GetPropertyStr(host->ctx, global, "__uifw");
    JSValue emit = JS_GetPropertyStr(host->ctx, bridge, "_emit");
    int failed = 0;
    if (JS_IsFunction(host->ctx, emit)) {
        JSValue args[3];
        args[0] = JS_NewString(host->ctx, name ? name : "");
        args[1] = JS_NewString(host->ctx, event ? event : "");
        args[2] = JS_ParseJSON(host->ctx, json ? json : "null", json ? len : 4, "<emit>");
        if (JS_IsException(args[2])) {
            JSValue exception = JS_GetException(host->ctx);
            JS_FreeValue(host->ctx, exception);
            args[2] = JS_NULL;
        }
        JSValue result = JS_Call(host->ctx, emit, JS_UNDEFINED, 3, args);
        JS_FreeValue(host->ctx, args[0]);
        JS_FreeValue(host->ctx, args[1]);
        JS_FreeValue(host->ctx, args[2]);
        if (JS_IsException(result)) {
            JSValue exception = JS_GetException(host->ctx);
            JS_FreeValue(host->ctx, exception);
            failed = -1;
        } else {
            JS_FreeValue(host->ctx, result);
        }
    }
    JS_FreeValue(host->ctx, emit);
    JS_FreeValue(host->ctx, bridge);
    JS_FreeValue(host->ctx, global);
    return failed;
}

int qjs_socket_event(Host *host, uint64_t id, const char *event, int is_text, const uint8_t *data,
                     size_t len) {
    if (!host || !host->ctx) {
        return -1;
    }
    JSValue global = JS_GetGlobalObject(host->ctx);
    JSValue bridge = JS_GetPropertyStr(host->ctx, global, "__uifw");
    JSValue hook = JS_GetPropertyStr(host->ctx, bridge, "_socket");
    int failed = 0;
    if (JS_IsFunction(host->ctx, hook)) {
        JSValue args[3];
        args[0] = JS_NewFloat64(host->ctx, (double)id);
        args[1] = JS_NewString(host->ctx, event ? event : "");
        if (is_text) {
            args[2] = JS_NewStringLen(host->ctx, data ? (const char *)data : "", len);
        } else if (len == 0) {
            args[2] = JS_NewArrayBufferCopy(host->ctx, (const uint8_t *)"", 0);
        } else {
            args[2] = JS_NewArrayBufferCopy(host->ctx, data, len);
        }
        JSValue result = JS_Call(host->ctx, hook, JS_UNDEFINED, 3, args);
        JS_FreeValue(host->ctx, args[0]);
        JS_FreeValue(host->ctx, args[1]);
        JS_FreeValue(host->ctx, args[2]);
        if (JS_IsException(result)) {
            JSValue exception = JS_GetException(host->ctx);
            JS_FreeValue(host->ctx, exception);
            failed = -1;
        } else {
            JS_FreeValue(host->ctx, result);
        }
    }
    JS_FreeValue(host->ctx, hook);
    JS_FreeValue(host->ctx, bridge);
    JS_FreeValue(host->ctx, global);
    return failed;
}
