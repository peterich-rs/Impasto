#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#include "quickjs.h"

/*
 * Handles are raw NodeIds. Generation lives in the high bits, so a live id is
 * never 0. Callers pass 0 for "no anchor" / "no node". A detached subtree's
 * onClick stays in `taps`: the region cannot be hit, so the closure is never
 * called. `remove` queues the subtree; the session frees those taps when it retires
 * ids that were not inserted again. `detach` leaves the taps and the ids.
 * `discard` retires immediately and frees taps. `qjs_close`
 * frees whatever is still registered.
 */
/* Wire record for core._mutate. Layout must match `linked/mutation.rs`. */
typedef struct MutRec {
    uint32_t op;
    uint32_t field;
    uint64_t id;
    uint64_t parent;
    uint64_t before;
    double number;
    double number2;
    double number3;
    double number4;
    uint8_t color[4];
    uint32_t _pad;
    const char *text;
    size_t text_len;
} MutRec;

/* Numbers and an optional session-owned string. Layout must match `linked/call.rs`. */
typedef struct CallOut {
    int status;
    int nnums;
    uint64_t handle;
    double nums[128];
    const char *text;
    size_t text_len;
} CallOut;

typedef struct QjsFns {
    void *session;
    void (*log)(void *session, const char *utf8, size_t len, int level);
    int (*http)(void *session, uint64_t token, const char *method, size_t method_len,
                const char *url, size_t url_len, const char *headers, size_t headers_len,
                const uint8_t *body, size_t body_len);
    int (*http_stream)(void *session, uint64_t token, const char *method, size_t method_len,
                       const char *url, size_t url_len, const char *headers, size_t headers_len,
                       const uint8_t *body, size_t body_len);
    void (*abort_token)(void *session, uint64_t token);
    int (*module_has)(void *session, const char *name, size_t len);
    /* 0 synchronous, 1 promise, -1 not linked. */
    int (*module_prepare)(void *session, const char *name, size_t name_len, const char *method,
                          size_t method_len);
    void (*socket_open)(void *session, uint64_t id, const char *url, size_t url_len,
                        const char *protocols, size_t protocols_len);
    void (*socket_send)(void *session, uint64_t id, const uint8_t *data, size_t len, int is_text);
    void (*socket_close)(void *session, uint64_t id);
    int (*push_mutation)(void *session, uint32_t op, uint64_t id, uint64_t parent, uint64_t before,
                         uint32_t field, double number, double number2, const uint8_t color[4],
                         const char *text, size_t text_len);
    /* Frees one zero-copy payload the session kept alive for QuickJS. */
    void (*release_bytes)(void *session, uint64_t handle);
    void (*module_push_null)(void *session);
    void (*module_push_bool)(void *session, int value);
    void (*module_push_f64)(void *session, double value);
    void (*module_push_str)(void *session, const char *text, size_t len);
    void (*module_push_bytes)(void *session, const uint8_t *bytes, size_t len);
    void (*module_begin_list)(void *session);
    void (*module_end_list)(void *session);
    void (*module_begin_map)(void *session);
    void (*module_map_key)(void *session, const char *text, size_t len);
    void (*module_end_map)(void *session);
    void (*module_set_token)(void *session, uint64_t token);
    int (*module_finish)(void *session);
    const char *(*module_error)(void *session, size_t *len);
    int (*module_count)(void *session, const char *name, size_t len);
    const char *(*module_method_at)(void *session, const char *name, size_t name_len, int index,
                                    size_t *out_len);
    const char *(*module_name_of)(void *session, const char *specifier, size_t len, size_t *out_len);
    int (*list_create)(void *session, int overscan, float estimate, uint32_t *index,
                       uint32_t *generation);
    int (*list_release)(void *session, uint32_t index, uint32_t generation);
    int (*list_bind)(void *session, uint32_t index, uint32_t generation, const uint64_t *nodes,
                     int count);
    int (*list_configure)(void *session, uint32_t index, uint32_t generation, float estimate,
                          int overscan, int blocked);
    int (*list_begin)(void *session, uint32_t index, uint32_t generation);
    int (*list_item)(void *session, uint32_t index, uint32_t generation, const char *key,
                     size_t key_len, const char *type, size_t type_len, int sticky);
    int (*list_end)(void *session, uint32_t index, uint32_t generation);
    int (*list_note_row)(void *session, uint32_t index, uint32_t generation, const char *key,
                         size_t key_len, uint64_t node);
    int (*scroll_create)(void *session, uint32_t *index, uint32_t *generation);
    int (*scroll_release)(void *session, uint32_t index, uint32_t generation);
    int (*scroll_bind)(void *session, uint32_t index, uint32_t generation, uint64_t viewport,
                       uint64_t content);
    int (*scroll_configure)(void *session, uint32_t index, uint32_t generation, float content_min,
                            int pull_mode, float threshold, int rubber, float factor,
                            uint32_t parent_index, uint32_t parent_generation, int has_parent);
    int (*scroll_drag_by)(void *session, uint32_t index, uint32_t generation, float dx, float dy,
                          float *overflow_x, float *overflow_y, float *over_x, float *over_y);
    int (*scroll_finish)(void *session, uint32_t index, uint32_t generation, int taken);
    int (*scroll_drag_end)(void *session, uint32_t index, uint32_t generation);
    int (*scroll_wheel)(void *session, uint32_t index, uint32_t generation, float dx, float dy);
    int (*scroll_query)(void *session, uint32_t index, uint32_t generation, int cmd, float *x,
                        float *y);
    int (*scroll_consume)(void *session, uint32_t index, uint32_t generation, float ox, float oy);
    int (*debug_json)(void *session, int what, uint64_t node, float x, float y);
    const char *(*debug_json_bytes)(void *session);
    void (*debug_set_scan)(void *session, int on);
    void (*debug_set_pointer_trace)(void *session, int on, int suppress);
    void (*debug_set_owned)(void *session, uint64_t node, int on);
    int (*mutate)(void *session, const MutRec *rec, uint64_t *out_id);
    int (*query)(void *session, int kind, uint64_t node, double a, double b, const char *text,
                 size_t text_len, const uint8_t color[4], CallOut *out);
    int (*effect)(void *session, int kind, uint64_t node, const double *nums, int nnums,
                  const uint8_t color[4], const char *text, size_t text_len, const uint8_t *bytes,
                  size_t byte_len, CallOut *out);
    void (*fault)(void *session, const char *source, const char *message, size_t message_len,
                  const char *stack, size_t stack_len);
} QjsFns;

typedef struct BackHandler {
    uint64_t id;
    JSValue fn;
} BackHandler;

typedef struct Shortcut {
    char *spec;
    JSValue fn;
} Shortcut;

typedef struct Tap {
    uint64_t id;
    JSValue click;
    JSValue press;
    JSValue release;
    JSValue drag_begin;
    JSValue drag_by;
    JSValue drag_end;
    JSValue focus;
    JSValue blur;
    JSValue key;
    JSValue text;
    JSValue composing;
    JSValue hover;
    JSValue leave;
    JSValue wheel;
    JSValue secondary_click;
    JSValue scroll;
    JSValue text_replaced;
    JSValue link;
    JSValue zoom;
    JSValue long_press;
    int has_click;
    int has_press;
    int has_release;
    int has_drag_begin;
    int has_drag_by;
    int has_drag_end;
    int has_focus;
    int has_blur;
    int has_key;
    int has_text;
    int has_composing;
    int has_hover;
    int has_leave;
    int has_wheel;
    int has_secondary_click;
    int has_scroll;
    int has_text_replaced;
    int has_link;
    int has_zoom;
    int has_long_press;
} Tap;

typedef struct Timer {
    uint64_t id;
    int64_t due_ms;
    int64_t interval_ms;
    uint64_t min_frame;
    JSValue fn;
    JSValue extra;
    int kind;
    int active;
} Timer;

typedef struct PromiseSlot {
    uint64_t token;
    JSValue resolve;
    JSValue reject;
    int live;
} PromiseSlot;

typedef struct SettingsHandler {
    uint64_t id;
    JSValue fn;
} SettingsHandler;

typedef struct ModuleListener {
    char *module;
    char *event;
    JSValue fn;
} ModuleListener;

typedef struct MotionHandler {
    uint64_t id;
    uint64_t node;
    char *prop;
    JSValue fn;
} MotionHandler;

typedef struct Host {
    JSRuntime *rt;
    JSContext *ctx;
    QjsFns fns;
    Tap *taps;
    size_t tap_len;
    size_t tap_cap;
    JSValue frame_cb;
    int has_frame;
    JSValue layout_cb;
    int has_layout;
    JSValue scroll_state_cb;
    int has_scroll_state;
    JSValue driver_cb;
    int has_driver;
    JSClassID list_class;
    JSClassID scroll_class;
    int wants_frame;
    int64_t deadline_ms;
    char *exc_message;
    char *exc_stack;
    const char *entry_kind;
    uint64_t entry_id;
    BackHandler *backs;
    size_t back_len;
    size_t back_cap;
    uint64_t back_next;
    Shortcut *shortcuts;
    size_t shortcut_len;
    size_t shortcut_cap;
    int listener_freeze;
    Timer *timers;
    size_t timer_len;
    size_t timer_cap;
    uint64_t timer_next;
    PromiseSlot *promises;
    size_t promise_len;
    size_t promise_cap;
    uint64_t token_next;
    uint64_t socket_next;
    int64_t mono_ms;
    int64_t origin_ms;
    int origin_set;
    uint64_t frame_seq;
    int paused;
    char *locale;
    size_t locale_len;
    char *app_dir;
    double settings_text_scale;
    int settings_dark;
    int settings_reduce;
    int settings_bold;
    int settings_contrast;
    SettingsHandler *settings_handlers;
    size_t settings_len;
    size_t settings_cap;
    uint64_t settings_next;
    JSValue *values;
    size_t values_len;
    size_t values_cap;
    ModuleListener *mod_listeners;
    size_t mod_listener_len;
    size_t mod_listener_cap;
    MotionHandler *motions;
    size_t motion_len;
    size_t motion_cap;
    uint64_t motion_next;
    /* Promises already reported by the rejection checker this job round. */
    JSValue reject_seen[64];
    size_t reject_seen_len;
} Host;

#define QJS_MEMORY_LIMIT (16u << 20)
/* Debug quickjs-ng takes about 44 ms to eval dist/counter.js. Mounting the
   gallery list does the same kind of host calls and also exceeds one frame.
   Crossing the limit throws InternalError: interrupted and discards the turn,
   so this is a runaway cutoff, not a frame budget. */
#define QJS_TURN_MS 128
/* Evaluating the quotes bundle plus mounting its first screen is legitimate
   one-shot work in debug builds, not a runaway turn. Slice interrupts keep
   the 128 ms cutoff; evaluation gets a wider but still finite budget. */
#define QJS_EVAL_MS 1000

static int64_t now_ms(void) {
    struct timespec ts;
    if (clock_gettime(CLOCK_MONOTONIC, &ts) != 0) {
        return 0;
    }
    return (int64_t)ts.tv_sec * 1000 + (int64_t)ts.tv_nsec / 1000000;
}

static void disarm_entry_ptr(Host **slot) {
    Host *host = slot ? *slot : NULL;
    if (!host) {
        return;
    }
    host->entry_kind = NULL;
    host->entry_id = 0;
}

static void arm_slice(Host *host, const char *kind, uint64_t id) {
    host->entry_kind = kind;
    host->entry_id = id;
    host->deadline_ms = now_ms() + QJS_TURN_MS;
}

static void arm_eval(Host *host) {
    host->entry_kind = "eval";
    host->entry_id = 0;
    host->deadline_ms = now_ms() + QJS_EVAL_MS;
}

static const char *dispatch_kind_name(int kind) {
    switch (kind) {
    case 0:
        return "click";
    case 1:
        return "press";
    case 2:
        return "release";
    case 3:
        return "dragBegin";
    case 4:
        return "dragBy";
    case 5:
        return "dragEnd";
    case 6:
        return "focus";
    case 7:
        return "blur";
    case 8:
        return "key";
    case 9:
        return "text";
    case 10:
        return "composing";
    case 11:
        return "hover";
    case 12:
        return "leave";
    case 13:
        return "wheel";
    case 14:
        return "secondaryClick";
    case 15:
        return "scroll";
    case 16:
        return "textReplaced";
    case 17:
        return "link";
    case 18:
        return "longPress";
    case 19:
        return "doubleTap";
    case 20:
        return "zoom";
    case 21:
        return "scrollPhase";
    default:
        return "tap";
    }
}

static char *dup_cstr(const char *text) {
    size_t len;
    char *out;
    if (!text) {
        return NULL;
    }
    len = strlen(text);
    out = malloc(len + 1);
    if (!out) {
        return NULL;
    }
    memcpy(out, text, len + 1);
    return out;
}

static int should_annotate(const char *message) {
    return message &&
           (strstr(message, "interrupted") != NULL || strcmp(message, "quickjs job round limit") == 0);
}

static char *annotate_message(Host *host, const char *message) {
    const char *body = message ? message : "quickjs error";
    size_t len;
    char *out;
    if (!host->entry_kind || !should_annotate(body)) {
        return dup_cstr(body);
    }
    len = strlen(body) + strlen(host->entry_kind) + 64;
    out = malloc(len);
    if (!out) {
        return dup_cstr(body);
    }
    if (host->entry_id != 0) {
        snprintf(out, len, "%s (%s node %llu)", body, host->entry_kind,
                 (unsigned long long)host->entry_id);
    } else {
        snprintf(out, len, "%s (%s)", body, host->entry_kind);
    }
    return out;
}

static void release_stash(Host *host) {
    free(host->exc_message);
    free(host->exc_stack);
    host->exc_message = NULL;
    host->exc_stack = NULL;
}

static void stash_plain(Host *host, const char *message) {
    release_stash(host);
    host->exc_message = annotate_message(host, message);
    host->exc_stack = NULL;
}

static int interrupt_handler(JSRuntime *rt, void *opaque) {
    (void)rt;
    Host *host = opaque;
    return now_ms() > host->deadline_ms;
}

static JSValue throw_msg(JSContext *ctx, const char *msg) {
    return JS_ThrowTypeError(ctx, "%s", msg);
}

static Host *host_of(JSContext *ctx) {
    return JS_GetContextOpaque(ctx);
}

static void stash_exception(Host *host, JSContext *ctx, char *err, size_t err_len) {
    JSValue exception = JS_GetException(ctx);
    const char *text = JS_ToCString(ctx, exception);
    JSValue stack_val = JS_GetPropertyStr(ctx, exception, "stack");
    const char *stack = NULL;
    if (!JS_IsException(stack_val) && !JS_IsUndefined(stack_val)) {
        stack = JS_ToCString(ctx, stack_val);
    }
    release_stash(host);
    host->exc_message = annotate_message(host, text ? text : "quickjs error");
    host->exc_stack = stack ? dup_cstr(stack) : NULL;
    if (err && err_len > 0) {
        snprintf(err, err_len, "%s", host->exc_message ? host->exc_message : "quickjs error");
    }
    if (stack) {
        JS_FreeCString(ctx, stack);
    }
    JS_FreeValue(ctx, stack_val);
    if (text) {
        JS_FreeCString(ctx, text);
    }
    JS_FreeValue(ctx, exception);
}

static void publish_fault(Host *host, const char *source) {
    if (!host->fns.fault || !host->exc_message) {
        return;
    }
    host->fns.fault(host->fns.session, source, host->exc_message, strlen(host->exc_message),
                    host->exc_stack, host->exc_stack ? strlen(host->exc_stack) : 0);
}

static void log_stashed(Host *host, int level) {
    size_t mlen;
    size_t slen;
    size_t total;
    char *line;
    if (!host->fns.log || !host->exc_message) {
        return;
    }
    mlen = strlen(host->exc_message);
    slen = host->exc_stack ? strlen(host->exc_stack) : 0;
    total = mlen + (slen ? 1 + slen : 0);
    line = malloc(total + 1);
    if (!line) {
        host->fns.log(host->fns.session, host->exc_message, mlen, level);
        return;
    }
    memcpy(line, host->exc_message, mlen);
    if (slen) {
        line[mlen] = '\n';
        memcpy(line + mlen + 1, host->exc_stack, slen);
    }
    line[total] = 0;
    host->fns.log(host->fns.session, line, total, level);
    free(line);
}

/* `report` logs and keeps dispatching. `freeze` fails the call so the frame holds. */
static void listener_failed(Host *host, char *err, size_t err_len, int *failed) {
    if (!host->listener_freeze) {
        stash_exception(host, host->ctx, NULL, 0);
        publish_fault(host, "listener");
        log_stashed(host, 3);
        release_stash(host);
        return;
    }
    if (!*failed) {
        stash_exception(host, host->ctx, err, err_len);
    } else {
        JSValue exception = JS_GetException(host->ctx);
        JS_FreeValue(host->ctx, exception);
    }
    *failed = 1;
}

/* Text of a rejection reason with its stack, prefixed like a console line.
   Returns a malloc'd string the caller frees with js_free. */
static char *format_rejection(JSContext *ctx, JSValueConst reason) {
    const char *text = JS_ToCString(ctx, reason);
    JSValue stack_val = JS_GetPropertyStr(ctx, reason, "stack");
    const char *stack = NULL;
    if (!JS_IsException(stack_val) && !JS_IsUndefined(stack_val)) {
        stack = JS_ToCString(ctx, stack_val);
    }
    size_t len = 32 + (text ? strlen(text) : 0) + (stack ? strlen(stack) : 0);
    char *out = js_malloc(ctx, len + 1);
    if (out) {
        snprintf(out, len + 1, "unhandled rejection: %s%s%s",
                 text ? text : "quickjs error", stack ? "\n" : "", stack ? stack : "");
    }
    if (stack) {
        JS_FreeCString(ctx, stack);
    }
    JS_FreeValue(ctx, stack_val);
    if (text) {
        JS_FreeCString(ctx, text);
    }
    return out;
}

static void rejection_fault(Host *host, JSValueConst reason) {
    JSContext *ctx = host->ctx;
    const char *text = JS_ToCString(ctx, reason);
    JSValue stack_val = JS_GetPropertyStr(ctx, reason, "stack");
    const char *stack = NULL;
    char *message;
    size_t len;
    if (!JS_IsException(stack_val) && !JS_IsUndefined(stack_val)) {
        stack = JS_ToCString(ctx, stack_val);
    }
    len = 32 + (text ? strlen(text) : 0);
    message = malloc(len + 1);
    if (message && host->fns.fault) {
        snprintf(message, len + 1, "unhandled rejection: %s", text ? text : "quickjs error");
        host->fns.fault(host->fns.session, "rejection", message, strlen(message), stack,
                        stack ? strlen(stack) : 0);
    }
    free(message);
    if (stack) {
        JS_FreeCString(ctx, stack);
    }
    JS_FreeValue(ctx, stack_val);
    if (text) {
        JS_FreeCString(ctx, text);
    }
}

static void report_rejection(Host *host, JSValueConst reason) {
    JSContext *ctx = host->ctx;
    char *text = format_rejection(ctx, reason);
    if (!text) {
        return;
    }
    rejection_fault(host, reason);
    /* The language packs install `globalThis.__reportUnhandled` when the host
       has no DOM events; it formats through the same reportError path. */
    JSValue global = JS_GetGlobalObject(ctx);
    JSValue hook = JS_GetPropertyStr(ctx, global, "__reportUnhandled");
    if (JS_IsFunction(ctx, hook)) {
        JSValue arg = JS_NewStringLen(ctx, text, strlen(text));
        JSValue result = JS_Call(ctx, hook, JS_UNDEFINED, 1, &arg);
        JS_FreeValue(ctx, arg);
        JS_FreeValue(ctx, result);
    }
    JS_FreeValue(ctx, hook);
    JS_FreeValue(ctx, global);
    if (host->fns.log) {
        host->fns.log(host->fns.session, text, strlen(text), 3);
    }
    js_free(ctx, text);
}

/* The runtime calls the checker the moment a promise rejects with no
   reactions, and again once a handler attaches. A late `.catch` in the same
   tick must not report, so candidates collect here and only promises still
   unhandled when the job queue drains are reported. */
static void promise_rejection_checker(JSContext *ctx, JSValueConst promise,
                                      JSValueConst reason, bool is_handled, void *opaque) {
    (void)reason;
    Host *host = opaque;
    if (!host) {
        return;
    }
    if (is_handled) {
        for (size_t i = 0; i < host->reject_seen_len; i++) {
            if (JS_VALUE_GET_PTR(host->reject_seen[i]) == JS_VALUE_GET_PTR(promise)) {
                JS_FreeValue(ctx, host->reject_seen[i]);
                host->reject_seen[i] = host->reject_seen[--host->reject_seen_len];
                return;
            }
        }
        return;
    }
    for (size_t i = 0; i < host->reject_seen_len; i++) {
        if (JS_VALUE_GET_PTR(host->reject_seen[i]) == JS_VALUE_GET_PTR(promise)) {
            return;
        }
    }
    if (host->reject_seen_len < sizeof(host->reject_seen) / sizeof(host->reject_seen[0])) {
        host->reject_seen[host->reject_seen_len++] = JS_DupValue(ctx, promise);
    }
}

static void flush_rejections(Host *host) {
    while (host->reject_seen_len > 0) {
        JSValue promise = host->reject_seen[--host->reject_seen_len];
        JSValue reason = JS_PromiseResult(host->ctx, promise);
        if (!JS_IsException(reason) && !JS_IsUndefined(reason)) {
            report_rejection(host, reason);
        }
        JS_FreeValue(host->ctx, reason);
        JS_FreeValue(host->ctx, promise);
    }
}

/* raw < 2^53, so the double is an exact integer. 0 is a valid "none". */
static int read_handle(JSContext *ctx, JSValueConst value, uint64_t *out) {
    double number = 0;
    if (JS_ToFloat64(ctx, &number, value)) {
        return -1;
    }
    if (!(number >= 0.0 && number < 9007199254740992.0) ||
        number != (double)(uint64_t)number) {
        JS_ThrowTypeError(ctx, "bad handle");
        return -1;
    }
    *out = (uint64_t)number;
    return 0;
}

static int read_optional_handle(JSContext *ctx, int argc, JSValueConst *argv, int index,
                                uint64_t *out) {
    *out = 0;
    if (argc <= index || JS_IsUndefined(argv[index]) || JS_IsNull(argv[index])) {
        return 0;
    }
    return read_handle(ctx, argv[index], out);
}

static JSValue handle_value(JSContext *ctx, uint64_t id) {
    if (id == 0) {
        return JS_NewInt32(ctx, 0);
    }
    return JS_NewFloat64(ctx, (double)id);
}

static JSValue created(JSContext *ctx, int status, uint64_t id, const char *what) {
    if (status) {
        return throw_msg(ctx, what);
    }
    return handle_value(ctx, id);
}

static JSValue done(JSContext *ctx, int status, const char *what) {
    if (status) {
        return throw_msg(ctx, what);
    }
    return JS_UNDEFINED;
}

static int read_color(JSContext *ctx, JSValueConst value, uint8_t out[4]) {
    if (!JS_IsArray(value)) {
        return -1;
    }
    for (uint32_t i = 0; i < 4; i++) {
        JSValue item = JS_GetPropertyUint32(ctx, value, i);
        if (JS_IsException(item)) {
            return -1;
        }
        int32_t channel = 0;
        int failed = JS_ToInt32(ctx, &channel, item);
        JS_FreeValue(ctx, item);
        if (failed || channel < 0 || channel > 255) {
            return -1;
        }
        out[i] = (uint8_t)channel;
    }
    return 0;
}



















static void free_tap(Host *host, Tap *tap);

static void forget_taps(Host *host, const uint64_t *ids, size_t len) {
    for (size_t id_index = 0; id_index < len; id_index++) {
        size_t index = 0;
        while (index < host->tap_len) {
            if (host->taps[index].id != ids[id_index]) {
                index++;
                continue;
            }
            free_tap(host, &host->taps[index]);
            host->tap_len--;
            host->taps[index] = host->taps[host->tap_len];
        }
    }
}







void qjs_forget_ids(Host *host, const uint64_t *ids, size_t len) {
    if (!host || !ids) {
        return;
    }
    forget_taps(host, ids, len);
}

























static void free_tap(Host *host, Tap *tap) {
    if (tap->has_click) {
        JS_FreeValue(host->ctx, tap->click);
    }
    if (tap->has_press) {
        JS_FreeValue(host->ctx, tap->press);
    }
    if (tap->has_release) {
        JS_FreeValue(host->ctx, tap->release);
    }
    if (tap->has_drag_begin) {
        JS_FreeValue(host->ctx, tap->drag_begin);
    }
    if (tap->has_drag_by) {
        JS_FreeValue(host->ctx, tap->drag_by);
    }
    if (tap->has_drag_end) {
        JS_FreeValue(host->ctx, tap->drag_end);
    }
    if (tap->has_focus) {
        JS_FreeValue(host->ctx, tap->focus);
    }
    if (tap->has_blur) {
        JS_FreeValue(host->ctx, tap->blur);
    }
    if (tap->has_key) {
        JS_FreeValue(host->ctx, tap->key);
    }
    if (tap->has_text) {
        JS_FreeValue(host->ctx, tap->text);
    }
    if (tap->has_composing) {
        JS_FreeValue(host->ctx, tap->composing);
    }
    if (tap->has_hover) {
        JS_FreeValue(host->ctx, tap->hover);
    }
    if (tap->has_leave) {
        JS_FreeValue(host->ctx, tap->leave);
    }
    if (tap->has_wheel) {
        JS_FreeValue(host->ctx, tap->wheel);
    }
    if (tap->has_secondary_click) {
        JS_FreeValue(host->ctx, tap->secondary_click);
    }
    if (tap->has_scroll) {
        JS_FreeValue(host->ctx, tap->scroll);
    }
    if (tap->has_text_replaced) {
        JS_FreeValue(host->ctx, tap->text_replaced);
    }
    if (tap->has_link) {
        JS_FreeValue(host->ctx, tap->link);
    }
    if (tap->has_zoom) {
        JS_FreeValue(host->ctx, tap->zoom);
    }
    if (tap->has_long_press) {
        JS_FreeValue(host->ctx, tap->long_press);
    }
}

static int tap_slot(Tap *tap, int kind, JSValue **slot, int **has) {
    switch (kind) {
    case 0:
        *slot = &tap->click;
        *has = &tap->has_click;
        return 0;
    case 1:
        *slot = &tap->press;
        *has = &tap->has_press;
        return 0;
    case 2:
        *slot = &tap->release;
        *has = &tap->has_release;
        return 0;
    case 3:
        *slot = &tap->drag_begin;
        *has = &tap->has_drag_begin;
        return 0;
    case 4:
        *slot = &tap->drag_by;
        *has = &tap->has_drag_by;
        return 0;
    case 5:
        *slot = &tap->drag_end;
        *has = &tap->has_drag_end;
        return 0;
    case 6:
        *slot = &tap->focus;
        *has = &tap->has_focus;
        return 0;
    case 7:
        *slot = &tap->blur;
        *has = &tap->has_blur;
        return 0;
    case 8:
        *slot = &tap->key;
        *has = &tap->has_key;
        return 0;
    case 9:
        *slot = &tap->text;
        *has = &tap->has_text;
        return 0;
    case 10:
        *slot = &tap->composing;
        *has = &tap->has_composing;
        return 0;
    case 11:
        *slot = &tap->hover;
        *has = &tap->has_hover;
        return 0;
    case 12:
        *slot = &tap->leave;
        *has = &tap->has_leave;
        return 0;
    case 13:
        *slot = &tap->wheel;
        *has = &tap->has_wheel;
        return 0;
    case 14:
        *slot = &tap->secondary_click;
        *has = &tap->has_secondary_click;
        return 0;
    case 15:
        *slot = &tap->scroll;
        *has = &tap->has_scroll;
        return 0;
    case 16:
        *slot = &tap->text_replaced;
        *has = &tap->has_text_replaced;
        return 0;
    case 17:
        *slot = &tap->link;
        *has = &tap->has_link;
        return 0;
    case 18:
        *slot = &tap->long_press;
        *has = &tap->has_long_press;
        return 0;
    case 20:
        *slot = &tap->zoom;
        *has = &tap->has_zoom;
        return 0;
    default:
        return -1;
    }
}

static Tap *tap_for(Host *host, uint64_t id) {
    for (size_t index = 0; index < host->tap_len; index++) {
        if (host->taps[index].id == id) {
            return &host->taps[index];
        }
    }
    if (host->tap_len == host->tap_cap) {
        size_t cap = host->tap_cap == 0 ? 4 : host->tap_cap * 2;
        Tap *grown = realloc(host->taps, cap * sizeof(Tap));
        if (!grown) {
            return NULL;
        }
        host->taps = grown;
        host->tap_cap = cap;
    }
    Tap *tap = &host->taps[host->tap_len++];
    memset(tap, 0, sizeof(Tap));
    tap->id = id;
    return tap;
}

static int store_callback(Host *host, uint64_t id, JSValue func, int kind) {
    Tap *tap = tap_for(host, id);
    JSValue *slot;
    int *has;
    if (!tap || tap_slot(tap, kind, &slot, &has)) {
        return -1;
    }
    if (*has) {
        JS_FreeValue(host->ctx, *slot);
    }
    *slot = func;
    *has = 1;
    return 0;
}

static JSValue bind_gesture(JSContext *ctx, int argc, JSValueConst *argv, int kind, const char *name) {
    Host *host = host_of(ctx);
    if (argc < 2) {
        return throw_msg(ctx, name);
    }
    uint64_t id = 0;
    if (read_handle(ctx, argv[0], &id)) {
        return JS_EXCEPTION;
    }
    if (!JS_IsFunction(ctx, argv[1])) {
        return throw_msg(ctx, name);
    }
    JSValue func = JS_DupValue(ctx, argv[1]);
    if (store_callback(host, id, func, kind)) {
        JS_FreeValue(ctx, func);
        return throw_msg(ctx, name);
    }
    return JS_UNDEFINED;
}

static int image_mode_tag(JSContext *ctx, JSValue value, int *mode);



static int image_mode_tag(JSContext *ctx, JSValue value, int *mode) {
    if (JS_IsNumber(value)) {
        int tag = 0;
        if (JS_ToInt32(ctx, &tag, value)) {
            return 1;
        }
        if (tag < 0 || tag > 2) {
            return -1;
        }
        *mode = tag;
        return 0;
    }
    const char *text = JS_ToCString(ctx, value);
    if (!text) {
        return -1;
    }
    int tag = -1;
    if (strcmp(text, "fill") == 0) {
        tag = 0;
    } else if (strcmp(text, "fit") == 0) {
        tag = 1;
    } else if (strcmp(text, "crop") == 0) {
        tag = 2;
    }
    JS_FreeCString(ctx, text);
    if (tag < 0) {
        return -1;
    }
    *mode = tag;
    return 0;
}

static JSValue js_on_click(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    return bind_gesture(ctx, argc, argv, 0, "onClick(region, function)");
}

static JSValue js_on_press(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    return bind_gesture(ctx, argc, argv, 1, "onPress(region, function)");
}

static JSValue js_on_release(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    return bind_gesture(ctx, argc, argv, 2, "onRelease(region, function)");
}

static JSValue js_on_drag_begin(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    return bind_gesture(ctx, argc, argv, 3, "onDragBegin(node, function)");
}

static JSValue js_on_drag_by(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    return bind_gesture(ctx, argc, argv, 4, "onDragBy(node, function)");
}

static JSValue js_on_drag_end(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    return bind_gesture(ctx, argc, argv, 5, "onDragEnd(node, function)");
}

static JSValue js_on_focus(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    return bind_gesture(ctx, argc, argv, 6, "onFocus(node, function)");
}

static JSValue js_on_blur(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    return bind_gesture(ctx, argc, argv, 7, "onBlur(node, function)");
}

static int focus_direction(const char *name) {
    if (strcmp(name, "next") == 0) {
        return 0;
    }
    if (strcmp(name, "previous") == 0) {
        return 1;
    }
    if (strcmp(name, "left") == 0) {
        return 2;
    }
    if (strcmp(name, "right") == 0) {
        return 3;
    }
    if (strcmp(name, "up") == 0) {
        return 4;
    }
    if (strcmp(name, "down") == 0) {
        return 5;
    }
    return -1;
}















static JSValue settings_object(JSContext *ctx, Host *host) {
    JSValue object = JS_NewObject(ctx);
    JS_SetPropertyStr(ctx, object, "locale",
                      JS_NewStringLen(ctx, host->locale ? host->locale : "", host->locale_len));
    JS_SetPropertyStr(ctx, object, "textScale", JS_NewFloat64(ctx, host->settings_text_scale));
    JS_SetPropertyStr(ctx, object, "brightness",
                      JS_NewString(ctx, host->settings_dark ? "dark" : "light"));
    JS_SetPropertyStr(ctx, object, "reduceMotion", JS_NewBool(ctx, host->settings_reduce));
    JS_SetPropertyStr(ctx, object, "boldText", JS_NewBool(ctx, host->settings_bold));
    JS_SetPropertyStr(ctx, object, "highContrast", JS_NewBool(ctx, host->settings_contrast));
    return object;
}

static JSValue js_settings(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    (void)argc;
    (void)argv;
    return settings_object(ctx, host_of(ctx));
}

static JSValue js_on_settings(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 1 || !JS_IsFunction(ctx, argv[0])) {
        return throw_msg(ctx, "onSettings(function)");
    }
    if (host->settings_len == host->settings_cap) {
        size_t cap = host->settings_cap ? host->settings_cap * 2 : 4;
        SettingsHandler *next = realloc(host->settings_handlers, cap * sizeof(SettingsHandler));
        if (!next) {
            return throw_msg(ctx, "onSettings failed");
        }
        host->settings_handlers = next;
        host->settings_cap = cap;
    }
    host->settings_next += 1;
    host->settings_handlers[host->settings_len].id = host->settings_next;
    host->settings_handlers[host->settings_len].fn = JS_DupValue(ctx, argv[0]);
    host->settings_len += 1;
    return JS_NewUint32(ctx, (uint32_t)host->settings_next);
}

static JSValue js_off_settings(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 1) {
        return throw_msg(ctx, "offSettings(id)");
    }
    uint32_t id = 0;
    if (JS_ToUint32(ctx, &id, argv[0])) {
        return JS_EXCEPTION;
    }
    for (size_t index = 0; index < host->settings_len; index++) {
        if (host->settings_handlers[index].id != id) {
            continue;
        }
        JS_FreeValue(ctx, host->settings_handlers[index].fn);
        memmove(&host->settings_handlers[index], &host->settings_handlers[index + 1],
                (host->settings_len - index - 1) * sizeof(SettingsHandler));
        host->settings_len -= 1;
        break;
    }
    return JS_UNDEFINED;
}













static int read_axis(JSContext *ctx, JSValueConst value, int *tag, float *number) {
    if (JS_IsString(value)) {
        const char *text = JS_ToCString(ctx, value);
        if (!text) {
            return -1;
        }
        if (strcmp(text, "auto") == 0) {
            *tag = 0;
            *number = 0;
            JS_FreeCString(ctx, text);
            return 0;
        }
        char *end = NULL;
        double parsed = strtod(text, &end);
        if (end != text && end[0] == '%' && end[1] == '\0' && isfinite(parsed) && parsed >= 0.0) {
            *tag = 2;
            *number = (float)(parsed / 100.0);
            JS_FreeCString(ctx, text);
            return 0;
        }
        JS_FreeCString(ctx, text);
        JS_ThrowTypeError(ctx, "bad axis");
        return -1;
    }
    if (!JS_IsNumber(value)) {
        JS_ThrowTypeError(ctx, "bad axis");
        return -1;
    }
    double parsed = 0;
    if (JS_ToFloat64(ctx, &parsed, value)) {
        return -1;
    }
    if (!isfinite(parsed) || parsed < 0.0) {
        JS_ThrowTypeError(ctx, "bad axis");
        return -1;
    }
    *tag = 1;
    *number = (float)parsed;
    return 0;
}

static int read_inset_edge(JSContext *ctx, JSValueConst value, int *is_auto, float *number) {
    if (JS_IsString(value)) {
        const char *text = JS_ToCString(ctx, value);
        if (!text) {
            return -1;
        }
        int ok = strcmp(text, "auto") == 0;
        JS_FreeCString(ctx, text);
        if (!ok) {
            JS_ThrowTypeError(ctx, "bad inset");
            return -1;
        }
        *is_auto = 1;
        *number = 0;
        return 0;
    }
    if (!JS_IsNumber(value)) {
        JS_ThrowTypeError(ctx, "bad inset");
        return -1;
    }
    double parsed = 0;
    if (JS_ToFloat64(ctx, &parsed, value)) {
        return -1;
    }
    if (!isfinite(parsed)) {
        JS_ThrowTypeError(ctx, "bad inset");
        return -1;
    }
    *is_auto = 0;
    *number = (float)parsed;
    return 0;
}

static int read_grid_line(JSContext *ctx, JSValueConst value, int *tag, int *number) {
    if (JS_IsString(value)) {
        const char *text = JS_ToCString(ctx, value);
        if (!text) {
            return -1;
        }
        int ok = strcmp(text, "auto") == 0;
        JS_FreeCString(ctx, text);
        if (!ok) {
            JS_ThrowTypeError(ctx, "bad grid line");
            return -1;
        }
        *tag = 0;
        *number = 0;
        return 0;
    }
    if (!JS_IsObject(value) || JS_IsNull(value) || JS_IsArray(value)) {
        JS_ThrowTypeError(ctx, "bad grid line");
        return -1;
    }
    JSValue line = JS_GetPropertyStr(ctx, value, "line");
    JSValue span = JS_GetPropertyStr(ctx, value, "span");
    int has_line = !JS_IsUndefined(line);
    int has_span = !JS_IsUndefined(span);
    if (has_line == has_span) {
        JS_FreeValue(ctx, line);
        JS_FreeValue(ctx, span);
        JS_ThrowTypeError(ctx, "bad grid line");
        return -1;
    }
    JSValue chosen = has_line ? line : span;
    double parsed = 0;
    if (JS_ToFloat64(ctx, &parsed, chosen)) {
        JS_FreeValue(ctx, line);
        JS_FreeValue(ctx, span);
        return -1;
    }
    JS_FreeValue(ctx, line);
    JS_FreeValue(ctx, span);
    if (!isfinite(parsed) || parsed != trunc(parsed)) {
        JS_ThrowTypeError(ctx, "bad grid line");
        return -1;
    }
    if (has_line) {
        if (parsed == 0.0 || parsed < -32768.0 || parsed > 32767.0) {
            JS_ThrowTypeError(ctx, "bad grid line");
            return -1;
        }
        *tag = 1;
    } else if (parsed < 1.0 || parsed > 65535.0) {
        JS_ThrowTypeError(ctx, "bad grid line");
        return -1;
    } else {
        *tag = 2;
    }
    *number = (int)parsed;
    return 0;
}









// measurePanel(node, mode, width, height) -> [w, h] or null.
// mode 0 max-content, 1 capped-max (width is the cap), 2 definite (width, and
// height unless NaN).


















static JSValue js_on_back(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 1 || !JS_IsFunction(ctx, argv[0])) {
        return throw_msg(ctx, "onBack(function)");
    }
    if (host->back_len == host->back_cap) {
        size_t cap = host->back_cap ? host->back_cap * 2 : 4;
        BackHandler *next = realloc(host->backs, cap * sizeof(BackHandler));
        if (!next) {
            return throw_msg(ctx, "onBack failed");
        }
        host->backs = next;
        host->back_cap = cap;
    }
    host->back_next += 1;
    host->backs[host->back_len].id = host->back_next;
    host->backs[host->back_len].fn = JS_DupValue(ctx, argv[0]);
    host->back_len += 1;
    return JS_NewUint32(ctx, (uint32_t)host->back_next);
}

static JSValue js_on_motion_end(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 3 || !JS_IsString(argv[1]) || !JS_IsFunction(ctx, argv[2])) {
        return throw_msg(ctx, "onMotionEnd(id, prop, function)");
    }
    uint64_t node = 0;
    if (read_handle(ctx, argv[0], &node)) {
        return JS_EXCEPTION;
    }
    size_t prop_len = 0;
    const char *prop = JS_ToCStringLen(ctx, &prop_len, argv[1]);
    if (!prop) {
        return JS_EXCEPTION;
    }
    if (host->motion_len == host->motion_cap) {
        size_t cap = host->motion_cap ? host->motion_cap * 2 : 4;
        MotionHandler *next = realloc(host->motions, cap * sizeof(MotionHandler));
        if (!next) {
            JS_FreeCString(ctx, prop);
            return throw_msg(ctx, "onMotionEnd failed");
        }
        host->motions = next;
        host->motion_cap = cap;
    }
    host->motion_next += 1;
    MotionHandler *handler = &host->motions[host->motion_len];
    handler->id = host->motion_next;
    handler->node = node;
    handler->prop = malloc(prop_len + 1);
    if (handler->prop) {
        memcpy(handler->prop, prop, prop_len);
        handler->prop[prop_len] = 0;
    }
    handler->fn = JS_DupValue(ctx, argv[2]);
    JS_FreeCString(ctx, prop);
    if (!handler->prop) {
        JS_FreeValue(ctx, handler->fn);
        return throw_msg(ctx, "onMotionEnd failed");
    }
    host->motion_len += 1;
    return JS_NewFloat64(ctx, (double)handler->id);
}

static JSValue js_off_motion_end(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 1) {
        return JS_UNDEFINED;
    }
    int64_t id = 0;
    if (JS_ToInt64(ctx, &id, argv[0]) || id <= 0) {
        return JS_UNDEFINED;
    }
    for (size_t index = 0; index < host->motion_len; index++) {
        if ((int64_t)host->motions[index].id != id) {
            continue;
        }
        free(host->motions[index].prop);
        JS_FreeValue(ctx, host->motions[index].fn);
        memmove(&host->motions[index], &host->motions[index + 1],
                (host->motion_len - index - 1) * sizeof(MotionHandler));
        host->motion_len--;
        break;
    }
    return JS_UNDEFINED;
}

static JSValue js_off_back(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 1) {
        return throw_msg(ctx, "offBack(id)");
    }
    uint32_t id = 0;
    if (JS_ToUint32(ctx, &id, argv[0])) {
        return JS_EXCEPTION;
    }
    for (size_t index = 0; index < host->back_len; index++) {
        if (host->backs[index].id != id) {
            continue;
        }
        JS_FreeValue(ctx, host->backs[index].fn);
        memmove(&host->backs[index], &host->backs[index + 1],
                (host->back_len - index - 1) * sizeof(BackHandler));
        host->back_len -= 1;
        break;
    }
    return JS_UNDEFINED;
}

static char *shortcut_spec(JSContext *ctx, JSValueConst value) {
    const char *text = JS_ToCString(ctx, value);
    if (!text) {
        return NULL;
    }
    size_t len = strlen(text);
    char *copy = malloc(len + 1);
    if (!copy) {
        JS_FreeCString(ctx, text);
        return NULL;
    }
    for (size_t index = 0; index < len; index++) {
        char ch = text[index];
        if (ch >= 'A' && ch <= 'Z') {
            ch = (char)(ch - 'A' + 'a');
        }
        copy[index] = ch;
    }
    copy[len] = 0;
    JS_FreeCString(ctx, text);
    return copy;
}

static JSValue js_on_shortcut(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 2 || !JS_IsFunction(ctx, argv[1])) {
        return throw_msg(ctx, "onShortcut(spec, function)");
    }
    char *spec = shortcut_spec(ctx, argv[0]);
    if (!spec || spec[0] == 0) {
        free(spec);
        return throw_msg(ctx, "onShortcut(spec, function)");
    }
    for (size_t index = 0; index < host->shortcut_len; index++) {
        if (strcmp(host->shortcuts[index].spec, spec) != 0) {
            continue;
        }
        JS_FreeValue(ctx, host->shortcuts[index].fn);
        host->shortcuts[index].fn = JS_DupValue(ctx, argv[1]);
        free(spec);
        return JS_UNDEFINED;
    }
    if (host->shortcut_len == host->shortcut_cap) {
        size_t cap = host->shortcut_cap ? host->shortcut_cap * 2 : 4;
        Shortcut *next = realloc(host->shortcuts, cap * sizeof(Shortcut));
        if (!next) {
            free(spec);
            return throw_msg(ctx, "onShortcut failed");
        }
        host->shortcuts = next;
        host->shortcut_cap = cap;
    }
    host->shortcuts[host->shortcut_len].spec = spec;
    host->shortcuts[host->shortcut_len].fn = JS_DupValue(ctx, argv[1]);
    host->shortcut_len += 1;
    return JS_UNDEFINED;
}

static JSValue js_set_listener_error_policy(JSContext *ctx, JSValueConst this_val, int argc,
                                            JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 1) {
        return throw_msg(ctx, "setListenerErrorPolicy(policy)");
    }
    const char *text = JS_ToCString(ctx, argv[0]);
    if (!text) {
        return JS_EXCEPTION;
    }
    if (strcmp(text, "freeze") == 0) {
        host->listener_freeze = 1;
    } else if (strcmp(text, "report") == 0) {
        host->listener_freeze = 0;
    } else {
        JS_FreeCString(ctx, text);
        return throw_msg(ctx, "setListenerErrorPolicy(policy)");
    }
    JS_FreeCString(ctx, text);
    return JS_UNDEFINED;
}













static JSValue js_on_key(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    return bind_gesture(ctx, argc, argv, 8, "onKey(node, function)");
}

static JSValue js_on_text(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    return bind_gesture(ctx, argc, argv, 9, "onText(node, function)");
}

static JSValue js_on_text_replaced(JSContext *ctx, JSValueConst this_val, int argc,
                                   JSValueConst *argv) {
    (void)this_val;
    return bind_gesture(ctx, argc, argv, 16, "onTextReplaced(node, function)");
}

static JSValue js_on_link(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    return bind_gesture(ctx, argc, argv, 17, "onLink(node, function)");
}

static JSValue js_on_composing(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    return bind_gesture(ctx, argc, argv, 10, "onComposing(node, function)");
}























static JSValue js_on_frame(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 1) {
        return throw_msg(ctx, "onFrame(function)");
    }
    if (!JS_IsFunction(ctx, argv[0])) {
        return throw_msg(ctx, "onFrame function");
    }
    if (host->has_frame) {
        JS_FreeValue(ctx, host->frame_cb);
    }
    host->frame_cb = JS_DupValue(ctx, argv[0]);
    host->has_frame = 1;
    return JS_UNDEFINED;
}

static JSValue js_on_layout(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 1) {
        return throw_msg(ctx, "onLayout(function)");
    }
    if (!JS_IsFunction(ctx, argv[0])) {
        return throw_msg(ctx, "onLayout function");
    }
    if (host->has_layout) {
        JS_FreeValue(ctx, host->layout_cb);
    }
    host->layout_cb = JS_DupValue(ctx, argv[0]);
    host->has_layout = 1;
    return JS_UNDEFINED;
}





static int named_align(const char *name, uint32_t *out, const char *a, uint32_t a_value,
                       const char *b, uint32_t b_value, const char *c, uint32_t c_value) {
    if (strcmp(name, a) == 0) {
        *out = a_value;
        return 0;
    }
    if (strcmp(name, b) == 0) {
        *out = b_value;
        return 0;
    }
    if (strcmp(name, c) == 0) {
        *out = c_value;
        return 0;
    }
    return -1;
}





static int named_cross(const char *name, uint32_t *out) {
    if (strcmp(name, "start") == 0) {
        *out = 0;
        return 0;
    }
    if (strcmp(name, "center") == 0) {
        *out = 1;
        return 0;
    }
    if (strcmp(name, "end") == 0) {
        *out = 2;
        return 0;
    }
    if (strcmp(name, "stretch") == 0) {
        *out = 3;
        return 0;
    }
    if (strcmp(name, "baseline") == 0) {
        *out = 4;
        return 0;
    }
    return -1;
}



























static JSValue point_object(JSContext *ctx, float x, float y, float w, float h, int with_size) {
    JSValue obj = JS_NewObject(ctx);
    if (JS_IsException(obj)) {
        return obj;
    }
    if (JS_SetPropertyStr(ctx, obj, "x", JS_NewFloat64(ctx, x)) < 0 ||
        JS_SetPropertyStr(ctx, obj, "y", JS_NewFloat64(ctx, y)) < 0) {
        JS_FreeValue(ctx, obj);
        return JS_EXCEPTION;
    }
    if (with_size &&
        (JS_SetPropertyStr(ctx, obj, "w", JS_NewFloat64(ctx, w)) < 0 ||
         JS_SetPropertyStr(ctx, obj, "h", JS_NewFloat64(ctx, h)) < 0)) {
        JS_FreeValue(ctx, obj);
        return JS_EXCEPTION;
    }
    return obj;
}

















static JSValue js_on_scroll_state(JSContext *ctx, JSValueConst this_val, int argc,
                                  JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 1) {
        return throw_msg(ctx, "onScrollState(function)");
    }
    if (!JS_IsFunction(ctx, argv[0])) {
        return throw_msg(ctx, "onScrollState function");
    }
    if (host->has_scroll_state) {
        JS_FreeValue(ctx, host->scroll_state_cb);
    }
    host->scroll_state_cb = JS_DupValue(ctx, argv[0]);
    host->has_scroll_state = 1;
    return JS_UNDEFINED;
}



static JSValue js_apply_mutation(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 8) {
        return throw_msg(ctx, "applyMutation(op, id, parent, before, field, number, number2, color, text)");
    }
    int32_t op = 0;
    int32_t field = 0;
    uint64_t id = 0;
    uint64_t parent = 0;
    uint64_t before = 0;
    double number = 0;
    double number2 = 0;
    uint8_t color[4] = {0, 0, 0, 0};
    const char *text = "";
    size_t text_len = 0;
    if (JS_ToInt32(ctx, &op, argv[0]) || read_handle(ctx, argv[1], &id) ||
        read_handle(ctx, argv[2], &parent) || read_handle(ctx, argv[3], &before) ||
        JS_ToInt32(ctx, &field, argv[4]) || JS_ToFloat64(ctx, &number, argv[5]) ||
        JS_ToFloat64(ctx, &number2, argv[6])) {
        return JS_EXCEPTION;
    }
    if (JS_IsArray(argv[7])) {
        for (int index = 0; index < 4; index++) {
            JSValue item = JS_GetPropertyUint32(ctx, argv[7], (uint32_t)index);
            int32_t channel = 0;
            if (!JS_IsUndefined(item) && JS_ToInt32(ctx, &channel, item)) {
                JS_FreeValue(ctx, item);
                return JS_EXCEPTION;
            }
            color[index] = (uint8_t)channel;
            JS_FreeValue(ctx, item);
        }
    }
    if (argc > 8 && JS_IsString(argv[8])) {
        text = JS_ToCStringLen(ctx, &text_len, argv[8]);
        if (text == NULL) {
            return JS_EXCEPTION;
        }
    }
    int failed = host->fns.push_mutation(host->fns.session, (uint32_t)op, id, parent, before,
                                         (uint32_t)field, number, number2, color, text, text_len);
    if (text_len > 0) {
        JS_FreeCString(ctx, text);
    }
    return done(ctx, failed, "applyMutation failed");
}





static int justify_value(const char *name, uint32_t *out) {
    if (strcmp(name, "start") == 0) {
        *out = 0;
        return 0;
    }
    if (strcmp(name, "center") == 0) {
        *out = 1;
        return 0;
    }
    if (strcmp(name, "end") == 0) {
        *out = 2;
        return 0;
    }
    if (strcmp(name, "space-between") == 0) {
        *out = 3;
        return 0;
    }
    if (strcmp(name, "space-around") == 0) {
        *out = 4;
        return 0;
    }
    if (strcmp(name, "space-evenly") == 0) {
        *out = 5;
        return 0;
    }
    return -1;
}







static int join_array(JSContext *ctx, JSValueConst value, char **out, size_t *out_len) {
    if (!JS_IsArray(value)) {
        return 1;
    }
    JSValue length_value = JS_GetPropertyStr(ctx, value, "length");
    int32_t length = 0;
    if (JS_ToInt32(ctx, &length, length_value) || length < 0) {
        JS_FreeValue(ctx, length_value);
        return -1;
    }
    JS_FreeValue(ctx, length_value);
    size_t cap = 64;
    char *buf = malloc(cap);
    if (!buf) {
        return -1;
    }
    buf[0] = 0;
    size_t len = 0;
    for (int32_t index = 0; index < length; index++) {
        JSValue item = JS_GetPropertyUint32(ctx, value, (uint32_t)index);
        size_t part_len = 0;
        const char *part = JS_ToCStringLen(ctx, &part_len, item);
        JS_FreeValue(ctx, item);
        if (!part) {
            free(buf);
            return -1;
        }
        size_t extra = part_len + (index > 0 ? 1 : 0);
        if (len + extra + 1 > cap) {
            size_t next = cap * 2;
            while (next < len + extra + 1) {
                next *= 2;
            }
            char *grown = realloc(buf, next);
            if (!grown) {
                JS_FreeCString(ctx, part);
                free(buf);
                return -1;
            }
            buf = grown;
            cap = next;
        }
        if (index > 0) {
            buf[len++] = ',';
        }
        memcpy(buf + len, part, part_len);
        len += part_len;
        buf[len] = 0;
        JS_FreeCString(ctx, part);
    }
    *out = buf;
    *out_len = len;
    return 0;
}



static int read_edges(JSContext *ctx, int argc, JSValueConst *argv, uint64_t *node, float *top,
                      float *right, float *bottom, float *left) {
    if (argc < 5 || read_handle(ctx, argv[0], node)) {
        return -1;
    }
    double values[4];
    for (int index = 0; index < 4; index++) {
        if (JS_ToFloat64(ctx, &values[index], argv[index + 1])) {
            return -1;
        }
    }
    *top = (float)values[0];
    *right = (float)values[1];
    *bottom = (float)values[2];
    *left = (float)values[3];
    return 0;
}















static JSValue js_on_hover(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    return bind_gesture(ctx, argc, argv, 11, "onHover(node, fn)");
}

static JSValue js_on_leave(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    return bind_gesture(ctx, argc, argv, 12, "onLeave(node, fn)");
}

static JSValue js_on_wheel(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    return bind_gesture(ctx, argc, argv, 13, "onWheel(node, fn)");
}

static JSValue js_on_secondary_click(JSContext *ctx, JSValueConst this_val, int argc,
                                     JSValueConst *argv) {
    (void)this_val;
    return bind_gesture(ctx, argc, argv, 14, "onSecondaryClick(node, fn)");
}

static JSValue js_on_scroll(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    return bind_gesture(ctx, argc, argv, 15, "onScroll(node, fn)");
}

static JSValue js_on_zoom(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    return bind_gesture(ctx, argc, argv, 20, "onZoom(node, fn)");
}

static JSValue js_on_long_press(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    return bind_gesture(ctx, argc, argv, 18, "onLongPress(node, fn)");
}

































static JSValue js_request_frame(JSContext *ctx, JSValueConst this_val, int argc,
                                JSValueConst *argv) {
    (void)this_val;
    (void)argc;
    (void)argv;
    host_of(ctx)->wants_frame = 1;
    return JS_UNDEFINED;
}

static JSValue console_call(JSContext *ctx, int argc, JSValueConst *argv, int level) {
    Host *host = host_of(ctx);
    char *buf = NULL;
    size_t len = 0;
    size_t cap = 0;
    for (int index = 0; index < argc; index++) {
        size_t part_len = 0;
        const char *part = JS_ToCStringLen(ctx, &part_len, argv[index]);
        if (!part) {
            free(buf);
            return JS_EXCEPTION;
        }
        size_t extra = part_len + (index > 0 ? 1 : 0);
        if (len + extra + 1 > cap) {
            size_t next = cap == 0 ? 64 : cap * 2;
            while (next < len + extra + 1) {
                next *= 2;
            }
            char *grown = realloc(buf, next);
            if (!grown) {
                JS_FreeCString(ctx, part);
                free(buf);
                return throw_msg(ctx, "console");
            }
            buf = grown;
            cap = next;
        }
        if (index > 0) {
            buf[len++] = ' ';
        }
        memcpy(buf + len, part, part_len);
        len += part_len;
        buf[len] = 0;
        JS_FreeCString(ctx, part);
    }
    if (!buf) {
        buf = calloc(1, 1);
        if (!buf) {
            return throw_msg(ctx, "console");
        }
    }
    if (host->fns.log) {
        host->fns.log(host->fns.session, buf, len, level);
    }
    free(buf);
    return JS_UNDEFINED;
}

static JSValue js_console_log(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    return console_call(ctx, argc, argv, 0);
}

static JSValue js_console_info(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    return console_call(ctx, argc, argv, 1);
}

static JSValue js_console_warn(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    return console_call(ctx, argc, argv, 2);
}

static JSValue js_console_error(JSContext *ctx, JSValueConst this_val, int argc,
                                JSValueConst *argv) {
    (void)this_val;
    return console_call(ctx, argc, argv, 3);
}

static JSValue js_console_debug(JSContext *ctx, JSValueConst this_val, int argc,
                                JSValueConst *argv) {
    (void)this_val;
    return console_call(ctx, argc, argv, 4);
}

/* `core.log(level, message)` mirrors console levels by name so error
   reporting in the language packs has one entry point. */
static JSValue js_core_log(JSContext *ctx, JSValueConst this_val, int argc,
                           JSValueConst *argv);
static JSValue js_debug_layout_issues(JSContext *ctx, JSValueConst this_val, int argc,
                                      JSValueConst *argv);
static JSValue js_debug_snapshot(JSContext *ctx, JSValueConst this_val, int argc,
                                 JSValueConst *argv);
static JSValue js_debug_probe(JSContext *ctx, JSValueConst this_val, int argc,
                              JSValueConst *argv);
static JSValue js_debug_last_pointer(JSContext *ctx, JSValueConst this_val, int argc,
                                     JSValueConst *argv);
static JSValue js_debug_set_scan(JSContext *ctx, JSValueConst this_val, int argc,
                                 JSValueConst *argv);
static JSValue js_debug_set_pointer_trace(JSContext *ctx, JSValueConst this_val, int argc,
                                          JSValueConst *argv);
static JSValue js_debug_set_owned(JSContext *ctx, JSValueConst this_val, int argc,
                                  JSValueConst *argv);
static JSValue js_debug_memory(JSContext *ctx, JSValueConst this_val, int argc,
                               JSValueConst *argv);

static JSValue js_core_log(JSContext *ctx, JSValueConst this_val, int argc,
                           JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 2 || !host->fns.log) {
        return JS_UNDEFINED;
    }
    const char *name = JS_ToCString(ctx, argv[0]);
    const char *text = JS_ToCString(ctx, argv[1]);
    if (!name || !text) {
        if (name) {
            JS_FreeCString(ctx, name);
        }
        if (text) {
            JS_FreeCString(ctx, text);
        }
        return JS_EXCEPTION;
    }
    int level = 1;
    if (strcmp(name, "error") == 0) {
        level = 3;
    } else if (strcmp(name, "warn") == 0) {
        level = 2;
    } else if (strcmp(name, "debug") == 0) {
        level = 4;
    }
    host->fns.log(host->fns.session, text, strlen(text), level);
    JS_FreeCString(ctx, name);
    JS_FreeCString(ctx, text);
    return JS_UNDEFINED;
}

static int bind_fn(JSContext *ctx, JSValue core, const char *name, JSCFunction *fn, int length);

static int bind_console(Host *host) {
    JSContext *ctx = host->ctx;
    JSValue global = JS_GetGlobalObject(ctx);
    JSValue console = JS_NewObject(ctx);
    int failed = bind_fn(ctx, console, "log", js_console_log, 0);
    failed |= bind_fn(ctx, console, "info", js_console_info, 0);
    failed |= bind_fn(ctx, console, "warn", js_console_warn, 0);
    failed |= bind_fn(ctx, console, "error", js_console_error, 0);
    failed |= bind_fn(ctx, console, "debug", js_console_debug, 0);
    failed |= JS_SetPropertyStr(ctx, global, "console", console) < 0;
    JS_FreeValue(ctx, global);
    return failed ? -1 : 0;
}

static int bind_fn(JSContext *ctx, JSValue core, const char *name, JSCFunction *fn, int length) {
    JSValue function = JS_NewCFunction(ctx, fn, name, length);
    if (JS_IsException(function)) {
        return -1;
    }
    /* JS_SetPropertyStr takes ownership of `function`. Success is 1, not 0. */
    return JS_SetPropertyStr(ctx, core, name, function) < 0 ? -1 : 0;
}

typedef struct DriverBox {
    uint32_t index;
    uint32_t generation;
} DriverBox;

static DriverBox *driver_box(Host *host, JSValueConst value, JSClassID class_id) {
    if (!host) {
        return NULL;
    }
    return JS_GetOpaque(value, class_id);
}

static void release_box(JSRuntime *rt, JSValueConst val, JSClassID class_id, int list) {
    Host *host = JS_GetRuntimeOpaque(rt);
    if (!host) {
        return;
    }
    DriverBox *box = JS_GetOpaque(val, class_id);
    if (!box) {
        return;
    }
    if (list) {
        host->fns.list_release(host->fns.session, box->index, box->generation);
    } else {
        host->fns.scroll_release(host->fns.session, box->index, box->generation);
    }
    free(box);
}

static void list_finalizer(JSRuntime *rt, JSValueConst val) {
    Host *host = JS_GetRuntimeOpaque(rt);
    if (!host) {
        return;
    }
    release_box(rt, val, host->list_class, 1);
}

static void scroll_finalizer(JSRuntime *rt, JSValueConst val) {
    Host *host = JS_GetRuntimeOpaque(rt);
    if (!host) {
        return;
    }
    release_box(rt, val, host->scroll_class, 0);
}

static JSValue make_driver(JSContext *ctx, JSClassID class_id, uint32_t index, uint32_t generation) {
    JSValue obj = JS_NewObjectClass(ctx, class_id);
    if (JS_IsException(obj)) {
        return obj;
    }
    DriverBox *box = calloc(1, sizeof(DriverBox));
    if (!box) {
        JS_FreeValue(ctx, obj);
        return throw_msg(ctx, "driver");
    }
    box->index = index;
    box->generation = generation;
    JS_SetOpaque(obj, box);
    return obj;
}

static JSValue driver_id_value(JSContext *ctx, DriverBox *box) {
    JSValue obj = JS_NewObject(ctx);
    JS_SetPropertyStr(ctx, obj, "index", JS_NewUint32(ctx, box->index));
    JS_SetPropertyStr(ctx, obj, "generation", JS_NewUint32(ctx, box->generation));
    return obj;
}

static JSValue js_list_driver(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    int32_t overscan = 4;
    double estimate = 1;
    if (argc >= 1 && JS_ToInt32(ctx, &overscan, argv[0])) {
        return JS_EXCEPTION;
    }
    if (argc >= 2 && JS_ToFloat64(ctx, &estimate, argv[1])) {
        return JS_EXCEPTION;
    }
    uint32_t index = 0;
    uint32_t generation = 0;
    if (host->fns.list_create(host->fns.session, overscan, (float)estimate, &index, &generation)) {
        return throw_msg(ctx, "listDriver");
    }
    return make_driver(ctx, host->list_class, index, generation);
}

static JSValue js_scroll_controller(JSContext *ctx, JSValueConst this_val, int argc,
                                    JSValueConst *argv) {
    (void)this_val;
    (void)argc;
    (void)argv;
    Host *host = host_of(ctx);
    uint32_t index = 0;
    uint32_t generation = 0;
    if (host->fns.scroll_create(host->fns.session, &index, &generation)) {
        return throw_msg(ctx, "scrollController");
    }
    return make_driver(ctx, host->scroll_class, index, generation);
}

static JSValue js_list_id(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)argc;
    (void)argv;
    DriverBox *box = driver_box(host_of(ctx), this_val, host_of(ctx)->list_class);
    if (!box) {
        return throw_msg(ctx, "listDriver");
    }
    return driver_id_value(ctx, box);
}

static JSValue js_scroll_id(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)argc;
    (void)argv;
    DriverBox *box = driver_box(host_of(ctx), this_val, host_of(ctx)->scroll_class);
    if (!box) {
        return throw_msg(ctx, "scrollController");
    }
    return driver_id_value(ctx, box);
}

static JSValue js_list_release(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)argc;
    (void)argv;
    Host *host = host_of(ctx);
    DriverBox *box = driver_box(host, this_val, host->list_class);
    if (!box) {
        return JS_UNDEFINED;
    }
    JS_SetOpaque(this_val, NULL);
    host->fns.list_release(host->fns.session, box->index, box->generation);
    free(box);
    return JS_UNDEFINED;
}

static JSValue js_scroll_release(JSContext *ctx, JSValueConst this_val, int argc,
                                 JSValueConst *argv) {
    (void)argc;
    (void)argv;
    Host *host = host_of(ctx);
    DriverBox *box = driver_box(host, this_val, host->scroll_class);
    if (!box) {
        return JS_UNDEFINED;
    }
    JS_SetOpaque(this_val, NULL);
    host->fns.scroll_release(host->fns.session, box->index, box->generation);
    free(box);
    return JS_UNDEFINED;
}

static JSValue js_list_bind(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    Host *host = host_of(ctx);
    DriverBox *box = driver_box(host, this_val, host->list_class);
    if (!box) {
        return throw_msg(ctx, "listDriver");
    }
    uint64_t nodes[10] = {0, 0, 0, 0, 0, 0, 0, 0, 0, 0};
    int count = argc < 10 ? argc : 10;
    for (int index = 0; index < count; index++) {
        if (JS_IsUndefined(argv[index]) || JS_IsNull(argv[index])) {
            continue;
        }
        if (read_handle(ctx, argv[index], &nodes[index])) {
            return JS_EXCEPTION;
        }
    }
    if (host->fns.list_bind(host->fns.session, box->index, box->generation, nodes, 10)) {
        return throw_msg(ctx, "listDriver.bind");
    }
    return JS_UNDEFINED;
}

static JSValue js_list_configure(JSContext *ctx, JSValueConst this_val, int argc,
                                 JSValueConst *argv) {
    Host *host = host_of(ctx);
    DriverBox *box = driver_box(host, this_val, host->list_class);
    if (!box || argc < 3) {
        return throw_msg(ctx, "listDriver.configure");
    }
    double estimate = 0;
    int32_t overscan = 0;
    if (JS_ToFloat64(ctx, &estimate, argv[0]) || JS_ToInt32(ctx, &overscan, argv[1])) {
        return JS_EXCEPTION;
    }
    int blocked = JS_ToBool(ctx, argv[2]);
    if (blocked < 0) {
        return JS_EXCEPTION;
    }
    if (host->fns.list_configure(host->fns.session, box->index, box->generation, (float)estimate,
                                 overscan, blocked)) {
        return throw_msg(ctx, "listDriver.configure");
    }
    return JS_UNDEFINED;
}

static JSValue js_list_set_items(JSContext *ctx, JSValueConst this_val, int argc,
                                 JSValueConst *argv) {
    Host *host = host_of(ctx);
    DriverBox *box = driver_box(host, this_val, host->list_class);
    if (!box || argc < 1 || !JS_IsArray(argv[0])) {
        return throw_msg(ctx, "listDriver.setItems");
    }
    int64_t length = 0;
    if (JS_GetLength(ctx, argv[0], &length)) {
        return JS_EXCEPTION;
    }
    if (host->fns.list_begin(host->fns.session, box->index, box->generation)) {
        return throw_msg(ctx, "listDriver.setItems");
    }
    for (int64_t index = 0; index < length; index++) {
        JSValue item = JS_GetPropertyInt64(ctx, argv[0], index);
        JSValue key_v = JS_GetPropertyStr(ctx, item, "key");
        JSValue type_v = JS_GetPropertyStr(ctx, item, "type");
        JSValue sticky_v = JS_GetPropertyStr(ctx, item, "sticky");
        size_t key_len = 0;
        size_t type_len = 0;
        const char *key = JS_ToCStringLen(ctx, &key_len, key_v);
        const char *type = JS_ToCStringLen(ctx, &type_len, type_v);
        int sticky = JS_ToBool(ctx, sticky_v);
        int failed = !key || !type || sticky < 0;
        if (!failed) {
            failed = host->fns.list_item(host->fns.session, box->index, box->generation, key, key_len,
                                         type ? type : "", type_len, sticky) != 0;
        }
        if (key) {
            JS_FreeCString(ctx, key);
        }
        if (type) {
            JS_FreeCString(ctx, type);
        }
        JS_FreeValue(ctx, sticky_v);
        JS_FreeValue(ctx, type_v);
        JS_FreeValue(ctx, key_v);
        JS_FreeValue(ctx, item);
        if (failed) {
            return throw_msg(ctx, "listDriver.setItems");
        }
    }
    if (host->fns.list_end(host->fns.session, box->index, box->generation)) {
        return throw_msg(ctx, "listDriver.setItems");
    }
    return JS_UNDEFINED;
}

static JSValue js_list_note_row(JSContext *ctx, JSValueConst this_val, int argc,
                                JSValueConst *argv) {
    Host *host = host_of(ctx);
    DriverBox *box = driver_box(host, this_val, host->list_class);
    if (!box || argc < 2) {
        return throw_msg(ctx, "listDriver.noteRow");
    }
    size_t key_len = 0;
    const char *key = JS_ToCStringLen(ctx, &key_len, argv[0]);
    uint64_t node = 0;
    if (!key || read_handle(ctx, argv[1], &node)) {
        if (key) {
            JS_FreeCString(ctx, key);
        }
        return key ? JS_EXCEPTION : throw_msg(ctx, "listDriver.noteRow");
    }
    int failed = host->fns.list_note_row(host->fns.session, box->index, box->generation, key, key_len,
                                         node);
    JS_FreeCString(ctx, key);
    if (failed) {
        return throw_msg(ctx, "listDriver.noteRow");
    }
    return JS_UNDEFINED;
}

static JSValue js_scroll_bind(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    Host *host = host_of(ctx);
    DriverBox *box = driver_box(host, this_val, host->scroll_class);
    if (!box || argc < 2) {
        return throw_msg(ctx, "scrollController.bind");
    }
    uint64_t viewport = 0;
    uint64_t content = 0;
    if (read_handle(ctx, argv[0], &viewport) || read_handle(ctx, argv[1], &content)) {
        return JS_EXCEPTION;
    }
    if (host->fns.scroll_bind(host->fns.session, box->index, box->generation, viewport, content)) {
        return throw_msg(ctx, "scrollController.bind");
    }
    return JS_UNDEFINED;
}

static JSValue js_scroll_configure(JSContext *ctx, JSValueConst this_val, int argc,
                                   JSValueConst *argv) {
    Host *host = host_of(ctx);
    DriverBox *box = driver_box(host, this_val, host->scroll_class);
    if (!box || argc < 5) {
        return throw_msg(ctx, "scrollController.configure");
    }
    double content_min = 0;
    int32_t pull_mode = 0;
    double threshold = 0;
    double factor = 0;
    if (JS_ToFloat64(ctx, &content_min, argv[0]) || JS_ToInt32(ctx, &pull_mode, argv[1]) ||
        JS_ToFloat64(ctx, &threshold, argv[2]) || JS_ToFloat64(ctx, &factor, argv[4])) {
        return JS_EXCEPTION;
    }
    int rubber = JS_ToBool(ctx, argv[3]);
    if (rubber < 0) {
        return JS_EXCEPTION;
    }
    uint32_t parent_index = 0;
    uint32_t parent_generation = 0;
    int has_parent = 0;
    if (argc >= 6 && !JS_IsUndefined(argv[5]) && !JS_IsNull(argv[5])) {
        DriverBox *parent = driver_box(host, argv[5], host->scroll_class);
        if (!parent) {
            return throw_msg(ctx, "scrollController.configure parent");
        }
        has_parent = 1;
        parent_index = parent->index;
        parent_generation = parent->generation;
    }
    if (host->fns.scroll_configure(host->fns.session, box->index, box->generation, (float)content_min,
                                   pull_mode, (float)threshold, rubber, (float)factor, parent_index,
                                   parent_generation, has_parent)) {
        return throw_msg(ctx, "scrollController.configure");
    }
    return JS_UNDEFINED;
}

static JSValue drag_object(JSContext *ctx, float ox, float oy, float over_x, float over_y,
                           int pending) {
    JSValue obj = JS_NewObject(ctx);
    JS_SetPropertyStr(ctx, obj, "ox", JS_NewFloat64(ctx, ox));
    JS_SetPropertyStr(ctx, obj, "oy", JS_NewFloat64(ctx, oy));
    JS_SetPropertyStr(ctx, obj, "overX", JS_NewFloat64(ctx, over_x));
    JS_SetPropertyStr(ctx, obj, "overY", JS_NewFloat64(ctx, over_y));
    JS_SetPropertyStr(ctx, obj, "pending", JS_NewBool(ctx, pending));
    return obj;
}

static JSValue js_scroll_drag_by(JSContext *ctx, JSValueConst this_val, int argc,
                                 JSValueConst *argv) {
    Host *host = host_of(ctx);
    DriverBox *box = driver_box(host, this_val, host->scroll_class);
    if (!box || argc < 2) {
        return throw_msg(ctx, "scrollController.dragBy");
    }
    double dx = 0;
    double dy = 0;
    if (JS_ToFloat64(ctx, &dx, argv[0]) || JS_ToFloat64(ctx, &dy, argv[1])) {
        return JS_EXCEPTION;
    }
    float ox = 0;
    float oy = 0;
    float over_x = 0;
    float over_y = 0;
    int status = host->fns.scroll_drag_by(host->fns.session, box->index, box->generation, (float)dx,
                                          (float)dy, &ox, &oy, &over_x, &over_y);
    if (status < 0) {
        return throw_msg(ctx, "scrollController.dragBy");
    }
    return drag_object(ctx, ox, oy, over_x, over_y, status > 0);
}

static JSValue js_scroll_finish(JSContext *ctx, JSValueConst this_val, int argc,
                                JSValueConst *argv) {
    Host *host = host_of(ctx);
    DriverBox *box = driver_box(host, this_val, host->scroll_class);
    if (!box || argc < 1) {
        return throw_msg(ctx, "scrollController.finishDrag");
    }
    int taken = JS_ToBool(ctx, argv[0]);
    if (taken < 0) {
        return JS_EXCEPTION;
    }
    if (host->fns.scroll_finish(host->fns.session, box->index, box->generation, taken)) {
        return throw_msg(ctx, "scrollController.finishDrag");
    }
    return JS_UNDEFINED;
}

static JSValue js_scroll_drag_end(JSContext *ctx, JSValueConst this_val, int argc,
                                  JSValueConst *argv) {
    (void)argc;
    (void)argv;
    Host *host = host_of(ctx);
    DriverBox *box = driver_box(host, this_val, host->scroll_class);
    if (!box) {
        return throw_msg(ctx, "scrollController.dragEnd");
    }
    if (host->fns.scroll_drag_end(host->fns.session, box->index, box->generation)) {
        return throw_msg(ctx, "scrollController.dragEnd");
    }
    return JS_UNDEFINED;
}

static JSValue js_scroll_wheel(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    Host *host = host_of(ctx);
    DriverBox *box = driver_box(host, this_val, host->scroll_class);
    if (!box || argc < 2) {
        return throw_msg(ctx, "scrollController.wheel");
    }
    double dx = 0;
    double dy = 0;
    if (JS_ToFloat64(ctx, &dx, argv[0]) || JS_ToFloat64(ctx, &dy, argv[1])) {
        return JS_EXCEPTION;
    }
    if (host->fns.scroll_wheel(host->fns.session, box->index, box->generation, (float)dx, (float)dy)) {
        return throw_msg(ctx, "scrollController.wheel");
    }
    return JS_UNDEFINED;
}

static JSValue js_scroll_query(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    Host *host = host_of(ctx);
    DriverBox *box = driver_box(host, this_val, host->scroll_class);
    if (!box || argc < 1) {
        return throw_msg(ctx, "scrollController.query");
    }
    int32_t cmd = 0;
    if (JS_ToInt32(ctx, &cmd, argv[0])) {
        return JS_EXCEPTION;
    }
    float x = 0;
    float y = 0;
    if (host->fns.scroll_query(host->fns.session, box->index, box->generation, cmd, &x, &y)) {
        return throw_msg(ctx, "scrollController.query");
    }
    return point_object(ctx, x, y, 0, 0, 0);
}

static JSValue js_scroll_consume(JSContext *ctx, JSValueConst this_val, int argc,
                                 JSValueConst *argv) {
    Host *host = host_of(ctx);
    DriverBox *box = driver_box(host, this_val, host->scroll_class);
    if (!box || argc < 2) {
        return throw_msg(ctx, "scrollController.consume");
    }
    double ox = 0;
    double oy = 0;
    if (JS_ToFloat64(ctx, &ox, argv[0]) || JS_ToFloat64(ctx, &oy, argv[1])) {
        return JS_EXCEPTION;
    }
    int status = host->fns.scroll_consume(host->fns.session, box->index, box->generation, (float)ox,
                                          (float)oy);
    if (status < 0) {
        return throw_msg(ctx, "scrollController.consume");
    }
    return JS_NewBool(ctx, status == 1);
}

static JSValue js_on_driver(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 1 || !JS_IsFunction(ctx, argv[0])) {
        return throw_msg(ctx, "onDriver(function)");
    }
    if (host->has_driver) {
        JS_FreeValue(ctx, host->driver_cb);
    }
    host->driver_cb = JS_DupValue(ctx, argv[0]);
    host->has_driver = 1;
    return JS_UNDEFINED;
}

static const JSCFunctionListEntry list_methods[] = {
    JS_CFUNC_DEF("bind", 10, js_list_bind),
    JS_CFUNC_DEF("configure", 3, js_list_configure),
    JS_CFUNC_DEF("setItems", 1, js_list_set_items),
    JS_CFUNC_DEF("noteRow", 2, js_list_note_row),
    JS_CFUNC_DEF("release", 0, js_list_release),
    JS_CFUNC_DEF("id", 0, js_list_id),
};

static const JSCFunctionListEntry scroll_methods[] = {
    JS_CFUNC_DEF("bind", 2, js_scroll_bind),
    JS_CFUNC_DEF("configure", 6, js_scroll_configure),
    JS_CFUNC_DEF("dragBy", 2, js_scroll_drag_by),
    JS_CFUNC_DEF("finishDrag", 1, js_scroll_finish),
    JS_CFUNC_DEF("dragEnd", 0, js_scroll_drag_end),
    JS_CFUNC_DEF("wheel", 2, js_scroll_wheel),
    JS_CFUNC_DEF("query", 1, js_scroll_query),
    JS_CFUNC_DEF("consume", 2, js_scroll_consume),
    JS_CFUNC_DEF("release", 0, js_scroll_release),
    JS_CFUNC_DEF("id", 0, js_scroll_id),
};

static JSValue call_object(JSContext *ctx, const CallOut *out) {
    JSValue object = JS_NewObject(ctx);
    JS_SetPropertyStr(ctx, object, "ok", JS_NewBool(ctx, out->status == 0));
    JS_SetPropertyStr(ctx, object, "id", JS_NewFloat64(ctx, (double)out->handle));
    JSValue nums = JS_NewArray(ctx);
    int count = out->nnums;
    if (count < 0) {
        count = 0;
    }
    if (count > 128) {
        count = 128;
    }
    for (int index = 0; index < count; index++) {
        JS_SetPropertyUint32(ctx, nums, (uint32_t)index, JS_NewFloat64(ctx, out->nums[index]));
    }
    JS_SetPropertyStr(ctx, object, "n", nums);
    if (out->text != NULL && out->text_len > 0) {
        JS_SetPropertyStr(ctx, object, "t", JS_NewStringLen(ctx, out->text, out->text_len));
    } else {
        JS_SetPropertyStr(ctx, object, "t", JS_NewString(ctx, ""));
    }
    return object;
}

static int read_number_list(JSContext *ctx, JSValueConst value, double *nums, int cap, int *count) {
    *count = 0;
    if (JS_IsUndefined(value) || JS_IsNull(value)) {
        return 0;
    }
    if (!JS_IsArray(value)) {
        return -1;
    }
    JSValue length_value = JS_GetPropertyStr(ctx, value, "length");
    int32_t length = 0;
    if (JS_ToInt32(ctx, &length, length_value) || length < 0) {
        JS_FreeValue(ctx, length_value);
        return -1;
    }
    JS_FreeValue(ctx, length_value);
    if (length > cap) {
        length = cap;
    }
    for (int32_t index = 0; index < length; index++) {
        JSValue item = JS_GetPropertyUint32(ctx, value, (uint32_t)index);
        double number = 0;
        int failed = JS_ToFloat64(ctx, &number, item);
        JS_FreeValue(ctx, item);
        if (failed) {
            return -1;
        }
        nums[index] = number;
    }
    *count = length;
    return 0;
}

static JSValue js_mutate(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 11) {
        return throw_msg(ctx, "mutate");
    }
    MutRec rec;
    memset(&rec, 0, sizeof(rec));
    int32_t op = 0;
    int32_t field = 0;
    double id = 0;
    double parent = 0;
    double before = 0;
    if (JS_ToInt32(ctx, &op, argv[0]) || JS_ToFloat64(ctx, &id, argv[1]) ||
        JS_ToFloat64(ctx, &parent, argv[2]) || JS_ToFloat64(ctx, &before, argv[3]) ||
        JS_ToInt32(ctx, &field, argv[4]) || JS_ToFloat64(ctx, &rec.number, argv[5]) ||
        JS_ToFloat64(ctx, &rec.number2, argv[6]) || JS_ToFloat64(ctx, &rec.number3, argv[7]) ||
        JS_ToFloat64(ctx, &rec.number4, argv[8])) {
        return JS_EXCEPTION;
    }
    rec.op = (uint32_t)op;
    rec.field = (uint32_t)field;
    rec.id = (uint64_t)id;
    rec.parent = (uint64_t)parent;
    rec.before = (uint64_t)before;
    if (JS_IsArray(argv[9]) && read_color(ctx, argv[9], rec.color)) {
        return JS_EXCEPTION;
    }
    if (JS_IsString(argv[10])) {
        rec.text = JS_ToCStringLen(ctx, &rec.text_len, argv[10]);
        if (rec.text == NULL) {
            return JS_EXCEPTION;
        }
    }
    uint64_t created_id = 0;
    int failed = host->fns.mutate(host->fns.session, &rec, &created_id);
    if (rec.text != NULL) {
        JS_FreeCString(ctx, rec.text);
    }
    CallOut out;
    memset(&out, 0, sizeof(out));
    out.status = failed;
    out.handle = created_id;
    return call_object(ctx, &out);
}

static JSValue js_query(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 1) {
        return throw_msg(ctx, "query");
    }
    int32_t kind = 0;
    double node = 0;
    double a = 0;
    double b = 0;
    if (JS_ToInt32(ctx, &kind, argv[0])) {
        return JS_EXCEPTION;
    }
    if (argc > 1 && !JS_IsUndefined(argv[1]) && JS_ToFloat64(ctx, &node, argv[1])) {
        return JS_EXCEPTION;
    }
    if (argc > 2 && !JS_IsUndefined(argv[2]) && JS_ToFloat64(ctx, &a, argv[2])) {
        return JS_EXCEPTION;
    }
    if (argc > 3 && !JS_IsUndefined(argv[3]) && JS_ToFloat64(ctx, &b, argv[3])) {
        return JS_EXCEPTION;
    }
    const char *text = NULL;
    size_t text_len = 0;
    if (argc > 4 && JS_IsString(argv[4])) {
        text = JS_ToCStringLen(ctx, &text_len, argv[4]);
        if (text == NULL) {
            return JS_EXCEPTION;
        }
    }
    uint8_t color[4] = {0, 0, 0, 0};
    if (argc > 5 && JS_IsArray(argv[5]) && read_color(ctx, argv[5], color)) {
        if (text != NULL) {
            JS_FreeCString(ctx, text);
        }
        return JS_EXCEPTION;
    }
    CallOut out;
    memset(&out, 0, sizeof(out));
    host->fns.query(host->fns.session, kind, (uint64_t)node, a, b, text, text_len, color, &out);
    if (text != NULL) {
        JS_FreeCString(ctx, text);
    }
    return call_object(ctx, &out);
}

static JSValue js_effect(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 1) {
        return throw_msg(ctx, "effect");
    }
    int32_t kind = 0;
    double node = 0;
    if (JS_ToInt32(ctx, &kind, argv[0])) {
        return JS_EXCEPTION;
    }
    if (argc > 1 && !JS_IsUndefined(argv[1]) && JS_ToFloat64(ctx, &node, argv[1])) {
        return JS_EXCEPTION;
    }
    double nums[16];
    int nnums = 0;
    if (argc > 2 && read_number_list(ctx, argv[2], nums, 16, &nnums)) {
        return JS_EXCEPTION;
    }
    const char *text = NULL;
    size_t text_len = 0;
    if (argc > 3 && JS_IsString(argv[3])) {
        text = JS_ToCStringLen(ctx, &text_len, argv[3]);
        if (text == NULL) {
            return JS_EXCEPTION;
        }
    }
    uint8_t color[4] = {0, 0, 0, 0};
    const uint8_t *color_ptr = NULL;
    if (argc > 4 && JS_IsArray(argv[4])) {
        if (read_color(ctx, argv[4], color)) {
            if (text != NULL) {
                JS_FreeCString(ctx, text);
            }
            return JS_EXCEPTION;
        }
        color_ptr = color;
    }
    const uint8_t *bytes = NULL;
    size_t byte_len = 0;
    size_t offset = 0;
    JSValue buffer = JS_UNDEFINED;
    if (argc > 5 && !JS_IsUndefined(argv[5]) && !JS_IsNull(argv[5])) {
        buffer = JS_GetTypedArrayBuffer(ctx, argv[5], &offset, &byte_len, NULL);
        if (JS_IsException(buffer)) {
            if (text != NULL) {
                JS_FreeCString(ctx, text);
            }
            return JS_EXCEPTION;
        }
        size_t buffer_len = 0;
        uint8_t *data = JS_GetArrayBuffer(ctx, &buffer_len, buffer);
        if (data == NULL || offset > buffer_len || byte_len > buffer_len - offset) {
            JS_FreeValue(ctx, buffer);
            if (text != NULL) {
                JS_FreeCString(ctx, text);
            }
            return throw_msg(ctx, "effect bytes");
        }
        bytes = data + offset;
    }
    CallOut out;
    memset(&out, 0, sizeof(out));
    host->fns.effect(host->fns.session, kind, (uint64_t)node, nums, nnums, color_ptr, text, text_len,
                     bytes, byte_len, &out);
    if (!JS_IsUndefined(buffer)) {
        JS_FreeValue(ctx, buffer);
    }
    if (text != NULL) {
        JS_FreeCString(ctx, text);
    }
    return call_object(ctx, &out);
}

static int bind_driver_classes(Host *host) {
    JS_NewClassID(host->rt, &host->list_class);
    JS_NewClassID(host->rt, &host->scroll_class);
    JSClassDef list_def;
    JSClassDef scroll_def;
    memset(&list_def, 0, sizeof(list_def));
    memset(&scroll_def, 0, sizeof(scroll_def));
    list_def.class_name = "ListDriver";
    list_def.finalizer = list_finalizer;
    scroll_def.class_name = "ScrollController";
    scroll_def.finalizer = scroll_finalizer;
    if (JS_NewClass(host->rt, host->list_class, &list_def) ||
        JS_NewClass(host->rt, host->scroll_class, &scroll_def)) {
        return -1;
    }
    JSValue list_proto = JS_NewObject(host->ctx);
    JSValue scroll_proto = JS_NewObject(host->ctx);
    JS_SetPropertyFunctionList(host->ctx, list_proto, list_methods,
                               (int)(sizeof(list_methods) / sizeof(list_methods[0])));
    JS_SetPropertyFunctionList(host->ctx, scroll_proto, scroll_methods,
                               (int)(sizeof(scroll_methods) / sizeof(scroll_methods[0])));
    JS_SetClassProto(host->ctx, host->list_class, list_proto);
    JS_SetClassProto(host->ctx, host->scroll_class, scroll_proto);
    return 0;
}

static int bind_core(Host *host) {
    if (bind_driver_classes(host)) {
        return -1;
    }
    JSContext *ctx = host->ctx;
    JSValue global = JS_GetGlobalObject(ctx);
    JSValue core = JS_NewObject(ctx);
    int failed = bind_fn(ctx, core, "_mutate", js_mutate, 11);
    failed |= bind_fn(ctx, core, "_query", js_query, 1);
    failed |= bind_fn(ctx, core, "_effect", js_effect, 1);
    failed |= bind_fn(ctx, core, "onScrollState", js_on_scroll_state, 1);
    failed |= bind_fn(ctx, core, "applyMutation", js_apply_mutation, 8);
    failed |= bind_fn(ctx, core, "onHover", js_on_hover, 2);
    failed |= bind_fn(ctx, core, "onLeave", js_on_leave, 2);
    failed |= bind_fn(ctx, core, "onWheel", js_on_wheel, 2);
    failed |= bind_fn(ctx, core, "onSecondaryClick", js_on_secondary_click, 2);
    failed |= bind_fn(ctx, core, "onScroll", js_on_scroll, 2);
    failed |= bind_fn(ctx, core, "onZoom", js_on_zoom, 2);
    failed |= bind_fn(ctx, core, "onLongPress", js_on_long_press, 2);
    failed |= bind_fn(ctx, core, "requestFrame", js_request_frame, 0);
    failed |= bind_fn(ctx, core, "onClick", js_on_click, 2);
    failed |= bind_fn(ctx, core, "onPress", js_on_press, 2);
    failed |= bind_fn(ctx, core, "onRelease", js_on_release, 2);
    failed |= bind_fn(ctx, core, "onDragBegin", js_on_drag_begin, 2);
    failed |= bind_fn(ctx, core, "onDragBy", js_on_drag_by, 2);
    failed |= bind_fn(ctx, core, "onDragEnd", js_on_drag_end, 2);
    failed |= bind_fn(ctx, core, "onFocus", js_on_focus, 2);
    failed |= bind_fn(ctx, core, "onBlur", js_on_blur, 2);
    failed |= bind_fn(ctx, core, "onKey", js_on_key, 2);
    failed |= bind_fn(ctx, core, "onText", js_on_text, 2);
    failed |= bind_fn(ctx, core, "onTextReplaced", js_on_text_replaced, 2);
    failed |= bind_fn(ctx, core, "onLink", js_on_link, 2);
    failed |= bind_fn(ctx, core, "onComposing", js_on_composing, 2);
    failed |= bind_fn(ctx, core, "onFrame", js_on_frame, 1);
    failed |= bind_fn(ctx, core, "onLayout", js_on_layout, 1);
    failed |= bind_fn(ctx, core, "onBack", js_on_back, 1);
    failed |= bind_fn(ctx, core, "onMotionEnd", js_on_motion_end, 3);
    failed |= bind_fn(ctx, core, "offMotionEnd", js_off_motion_end, 1);
    failed |= bind_fn(ctx, core, "offBack", js_off_back, 1);
    failed |= bind_fn(ctx, core, "onShortcut", js_on_shortcut, 2);
    failed |= bind_fn(ctx, core, "setListenerErrorPolicy", js_set_listener_error_policy, 1);
    failed |= bind_fn(ctx, core, "log", js_core_log, 2);
    failed |= bind_fn(ctx, core, "debugLayoutIssues", js_debug_layout_issues, 0);
    failed |= bind_fn(ctx, core, "debugSnapshot", js_debug_snapshot, 0);
    failed |= bind_fn(ctx, core, "debugProbe", js_debug_probe, 2);
    failed |= bind_fn(ctx, core, "debugLastPointer", js_debug_last_pointer, 0);
    failed |= bind_fn(ctx, core, "debugSetScan", js_debug_set_scan, 1);
    failed |= bind_fn(ctx, core, "debugSetPointerTrace", js_debug_set_pointer_trace, 2);
    failed |= bind_fn(ctx, core, "debugSetOwned", js_debug_set_owned, 2);
    failed |= bind_fn(ctx, core, "debugMemory", js_debug_memory, 0);
    failed |= bind_fn(ctx, core, "settings", js_settings, 0);
    failed |= bind_fn(ctx, core, "onSettings", js_on_settings, 1);
    failed |= bind_fn(ctx, core, "offSettings", js_off_settings, 1);
    failed |= bind_fn(ctx, core, "listDriver", js_list_driver, 2);
    failed |= bind_fn(ctx, core, "scrollController", js_scroll_controller, 0);
    failed |= bind_fn(ctx, core, "onDriver", js_on_driver, 1);
    /* JS_SetPropertyStr takes ownership of `core`. Success is 1, not 0. */
    failed |= JS_SetPropertyStr(ctx, global, "core", core) < 0;
    JS_FreeValue(ctx, global);
    return failed ? -1 : 0;
}

static int bind_host_globals(Host *host);
int qjs_drain_jobs(Host *host, char *err, size_t err_len);
static void free_timers(Host *host);
static void free_promises(Host *host);

static void module_reject(JSContext *ctx) {
    JS_ThrowReferenceError(ctx, "module path is outside the app directory");
}

static JSModuleDef *load_host_module(JSContext *ctx, Host *host, const char *name);

/* "@impasto/<pkg>/host" with one package segment. */
static const char HOST_SCOPE[] = "@impasto/";

static int is_host_specifier(const char *name) {
    static const char suffix[] = "/host";
    size_t len;
    size_t mid_len;
    const char *mid;
    size_t index;
    if (!name) {
        return 0;
    }
    len = strlen(name);
    if (len <= sizeof(HOST_SCOPE) - 1 + sizeof(suffix) - 1) {
        return 0;
    }
    if (strncmp(name, HOST_SCOPE, sizeof(HOST_SCOPE) - 1) != 0) {
        return 0;
    }
    if (strcmp(name + len - (sizeof(suffix) - 1), suffix) != 0) {
        return 0;
    }
    mid = name + sizeof(HOST_SCOPE) - 1;
    mid_len = len - (sizeof(HOST_SCOPE) - 1) - (sizeof(suffix) - 1);
    if (mid_len == 0) {
        return 0;
    }
    for (index = 0; index < mid_len; index++) {
        if (mid[index] == '/' || mid[index] == '\\') {
            return 0;
        }
    }
    return 1;
}

static int module_segment(const char *text, size_t len) {
    if (len == 0) {
        return 0;
    }
    for (size_t index = 0; index < len; index++) {
        char c = text[index];
        int ok = (c >= 'A' && c <= 'Z') || (c >= 'a' && c <= 'z') || (c >= '0' && c <= '9') || c == '.' ||
                 c == '_' || c == '-';
        if (!ok) {
            return 0;
        }
    }
    return 1;
}

/* `path` is absolute and already starts with `app_dir`. Collapses `.` and `..`. */
static int module_canonicalize(char *path, size_t app_len) {
    char *dst = path + app_len;
    const char *src = path + app_len;
    while (*src) {
        while (*src == '/') {
            src++;
        }
        if (!*src) {
            break;
        }
        const char *slash = strchr(src, '/');
        size_t seglen = slash ? (size_t)(slash - src) : strlen(src);
        if (seglen == 1 && src[0] == '.') {
            src += seglen;
            continue;
        }
        if (seglen == 2 && src[0] == '.' && src[1] == '.') {
            if (dst == path + app_len) {
                return 0;
            }
            if (dst > path + app_len && dst[-1] == '/') {
                dst--;
            }
            while (dst > path + app_len && dst[-1] != '/') {
                dst--;
            }
            src += seglen;
            continue;
        }
        if (!module_segment(src, seglen)) {
            return 0;
        }
        if ((size_t)(dst - path) + seglen + 2 >= 4096) {
            return 0;
        }
        *dst++ = '/';
        memcpy(dst, src, seglen);
        dst += seglen;
        src += seglen;
    }
    *dst = 0;
    if (path[app_len] != '/') {
        return 0;
    }
    const char *rel = path + app_len + 1;
    size_t rel_len = strlen(rel);
    return rel_len >= 3 && strcmp(rel + rel_len - 3, ".js") == 0;
}

static char *module_normalize(JSContext *ctx, const char *base_name, const char *name, void *opaque) {
    Host *host = opaque;
    if (name && is_host_specifier(name)) {
        return js_strdup(ctx, name);
    }
    if (name && strncmp(name, HOST_SCOPE, sizeof(HOST_SCOPE) - 1) == 0) {
        JS_ThrowReferenceError(ctx,
                               "bare @impasto/* must be bundled; runtime only loads @impasto/<pkg>/host");
        return NULL;
    }
    if (!host || !host->app_dir || !name || !name[0] || strchr(name, '\\') || strstr(name, "://")) {
        module_reject(ctx);
        return NULL;
    }
    const char *app = host->app_dir;
    size_t app_len = strlen(app);
    char raw[4096];
    if (name[0] == '/') {
        if (strlen(name) >= sizeof(raw)) {
            module_reject(ctx);
            return NULL;
        }
        memcpy(raw, name, strlen(name) + 1);
    } else {
        const char *dir = app;
        size_t dir_len = app_len;
        if (base_name && strchr(base_name, '/')) {
            const char *slash = strrchr(base_name, '/');
            dir = base_name;
            dir_len = (size_t)(slash - base_name);
        }
        if (dir_len + 1 + strlen(name) + 1 >= sizeof(raw)) {
            module_reject(ctx);
            return NULL;
        }
        memcpy(raw, dir, dir_len);
        raw[dir_len] = '/';
        memcpy(raw + dir_len + 1, name, strlen(name) + 1);
    }
    if (strncmp(raw, app, app_len) != 0 || (raw[app_len] != '/' && raw[app_len] != 0)) {
        module_reject(ctx);
        return NULL;
    }
    if (!module_canonicalize(raw, app_len)) {
        module_reject(ctx);
        return NULL;
    }
    return js_strdup(ctx, raw);
}

static JSModuleDef *module_loader(JSContext *ctx, const char *module_name, void *opaque) {
    Host *host = opaque;
    if (host && module_name && is_host_specifier(module_name)) {
        return load_host_module(ctx, host, module_name);
    }
    if (!host || !host->app_dir || !module_name) {
        module_reject(ctx);
        return NULL;
    }
    size_t app_len = strlen(host->app_dir);
    if (strncmp(module_name, host->app_dir, app_len) != 0 || module_name[app_len] != '/') {
        module_reject(ctx);
        return NULL;
    }
    FILE *file = fopen(module_name, "rb");
    if (!file) {
        JS_ThrowReferenceError(ctx, "could not load module '%s'", module_name);
        return NULL;
    }
    if (fseek(file, 0, SEEK_END) != 0) {
        fclose(file);
        module_reject(ctx);
        return NULL;
    }
    long length = ftell(file);
    if (length < 0 || length > 16L * 1024L * 1024L) {
        fclose(file);
        module_reject(ctx);
        return NULL;
    }
    if (fseek(file, 0, SEEK_SET) != 0) {
        fclose(file);
        module_reject(ctx);
        return NULL;
    }
    char *buf = malloc((size_t)length + 1);
    if (!buf) {
        fclose(file);
        JS_ThrowOutOfMemory(ctx);
        return NULL;
    }
    size_t got = fread(buf, 1, (size_t)length, file);
    fclose(file);
    if (got != (size_t)length) {
        free(buf);
        module_reject(ctx);
        return NULL;
    }
    buf[length] = 0;
    JSValue val = JS_Eval(ctx, buf, (size_t)length, module_name, JS_EVAL_TYPE_MODULE | JS_EVAL_FLAG_COMPILE_ONLY);
    free(buf);
    if (JS_IsException(val)) {
        return NULL;
    }
    JSModuleDef *mod = JS_VALUE_GET_PTR(val);
    JS_FreeValue(ctx, val);
    return mod;
}

void qjs_set_app_dir(Host *host, const char *dir) {
    if (!host) {
        return;
    }
    free(host->app_dir);
    host->app_dir = NULL;
    if (!dir || !dir[0]) {
        return;
    }
    size_t len = strlen(dir);
    while (len > 1 && dir[len - 1] == '/') {
        len--;
    }
    host->app_dir = malloc(len + 1);
    if (!host->app_dir) {
        return;
    }
    memcpy(host->app_dir, dir, len);
    host->app_dir[len] = 0;
}

Host *qjs_open(const QjsFns *fns) {
    Host *host = calloc(1, sizeof(Host));
    if (!host) {
        return NULL;
    }
    host->fns = *fns;
    host->settings_text_scale = 1.0;
    host->rt = JS_NewRuntime();
    host->ctx = host->rt ? JS_NewContext(host->rt) : NULL;
    if (!host->ctx) {
        if (host->rt) {
            JS_FreeRuntime(host->rt);
        }
        free(host);
        return NULL;
    }
    JS_SetContextOpaque(host->ctx, host);
    JS_SetRuntimeOpaque(host->rt, host);
    host->deadline_ms = INT64_MAX;
    JS_SetMemoryLimit(host->rt, QJS_MEMORY_LIMIT);
    /* Solid 2's render effects nest deeper than 1 MiB on the gallery page. */
    JS_SetMaxStackSize(host->rt, 8u << 20);
    JS_SetInterruptHandler(host->rt, interrupt_handler, host);
    JS_SetHostPromiseRejectionTracker(host->rt, promise_rejection_checker, host);
    JS_SetModuleLoaderFunc(host->rt, module_normalize, module_loader, host);
    if (bind_core(host) || bind_console(host) || bind_host_globals(host)) {
        JS_FreeContext(host->ctx);
        JS_FreeRuntime(host->rt);
        free(host->taps);
        free(host);
        return NULL;
    }
    return host;
}

int qjs_eval(Host *host, const char *src, size_t len, const char *name, char *err, size_t err_len) {
    arm_eval(host);
    Host *armed __attribute__((cleanup(disarm_entry_ptr))) = host;
    (void)armed;
    /* QuickJS reads input[input_len] and requires it to be 0. */
    char *owned = malloc(len + 1);
    if (!owned) {
        if (err && err_len > 0) {
            snprintf(err, err_len, "out of memory");
        }
        return -1;
    }
    memcpy(owned, src, len);
    owned[len] = 0;
    int flags = JS_DetectModule(owned, len) ? JS_EVAL_TYPE_MODULE : JS_EVAL_TYPE_GLOBAL;
    JSValue result = JS_Eval(host->ctx, owned, len, name, flags);
    free(owned);
    if (!JS_IsException(result)) {
        if (flags == JS_EVAL_TYPE_MODULE) {
            if (qjs_drain_jobs(host, err, err_len) != 0) {
                JS_FreeValue(host->ctx, result);
                return -1;
            }
            /* A module body throw rejects the evaluation promise. `JS_Eval`
               still returns that promise, so the rejection has to be read here. */
            if (JS_PromiseState(host->ctx, result) == JS_PROMISE_REJECTED) {
                JSValue reason = JS_PromiseResult(host->ctx, result);
                const char *text = JS_ToCString(host->ctx, reason);
                if (err && err_len > 0) {
                    snprintf(err, err_len, "%s", text ? text : "module evaluation failed");
                }
                if (text) {
                    JS_FreeCString(host->ctx, text);
                }
                JS_FreeValue(host->ctx, reason);
                JS_FreeValue(host->ctx, result);
                return -1;
            }
        }
        JS_FreeValue(host->ctx, result);
        return 0;
    }
    stash_exception(host, host->ctx, err, err_len);
    return -1;
}

int qjs_dispatch(Host *host, uint64_t id, int kind, double a, double b, char *err, size_t err_len) {
    arm_slice(host, dispatch_kind_name(kind), id);
    Host *armed __attribute__((cleanup(disarm_entry_ptr))) = host;
    (void)armed;
    int failed = 0;
    for (size_t index = 0; index < host->tap_len; index++) {
        JSValue *slot = NULL;
        int *has = NULL;
        if (host->taps[index].id != id) {
            continue;
        }
        if (tap_slot(&host->taps[index], kind, &slot, &has) || !has || !*has || !slot) {
            continue;
        }
        JSValue args[2];
        int argc = 0;
        if (kind == 4 || kind == 5 || kind == 13 || kind == 15) {
            args[0] = JS_NewFloat64(host->ctx, a);
            args[1] = JS_NewFloat64(host->ctx, b);
            argc = 2;
        } else if (kind == 17 || kind == 20) {
            args[0] = JS_NewFloat64(host->ctx, a);
            argc = 1;
        }
        JSValue result = JS_Call(host->ctx, *slot, JS_UNDEFINED, argc, argc ? args : NULL);
        for (int arg = 0; arg < argc; arg++) {
            JS_FreeValue(host->ctx, args[arg]);
        }
        if (!JS_IsException(result)) {
            JS_FreeValue(host->ctx, result);
            continue;
        }
        listener_failed(host, err, err_len, &failed);
    }
    return failed ? -1 : 0;
}

int qjs_dispatch_input(Host *host, uint64_t id, int kind, const char *utf8, size_t len,
                       int with_span, int sel_start, int sel_end, int compose_start,
                       int compose_end, char *err, size_t err_len) {
    arm_slice(host, dispatch_kind_name(kind), id);
    Host *armed __attribute__((cleanup(disarm_entry_ptr))) = host;
    (void)armed;
    int failed = 0;
    for (size_t index = 0; index < host->tap_len; index++) {
        JSValue *slot = NULL;
        int *has = NULL;
        if (host->taps[index].id != id) {
            continue;
        }
        if (tap_slot(&host->taps[index], kind, &slot, &has) || !has || !*has || !slot) {
            continue;
        }
        JSValue args[5];
        int argc = 1;
        if (utf8 == NULL) {
            args[0] = JS_NULL;
        } else {
            args[0] = JS_NewStringLen(host->ctx, utf8, len);
        }
        if (with_span) {
            args[1] = JS_NewInt32(host->ctx, sel_start);
            args[2] = JS_NewInt32(host->ctx, sel_end);
            args[3] = JS_NewInt32(host->ctx, compose_start);
            args[4] = JS_NewInt32(host->ctx, compose_end);
            argc = 5;
        }
        JSValue result = JS_Call(host->ctx, *slot, JS_UNDEFINED, argc, args);
        for (int arg = 0; arg < argc; arg++) {
            JS_FreeValue(host->ctx, args[arg]);
        }
        if (!JS_IsException(result)) {
            JS_FreeValue(host->ctx, result);
            continue;
        }
        listener_failed(host, err, err_len, &failed);
    }
    return failed ? -1 : 0;
}

int qjs_dispatch_text(Host *host, uint64_t id, const char *utf8, size_t len, char *err,
                      size_t err_len) {
    return qjs_dispatch_input(host, id, 16, utf8, len, 0, -1, -1, -1, -1, err, err_len);
}

#define QJS_JOB_ROUND_LIMIT 32

int qjs_drain_jobs(Host *host, char *err, size_t err_len) {
    arm_slice(host, "jobs", 0);
    Host *armed __attribute__((cleanup(disarm_entry_ptr))) = host;
    (void)armed;
    int rounds = 0;
    while (JS_IsJobPending(host->rt)) {
        if (rounds >= QJS_JOB_ROUND_LIMIT) {
            stash_plain(host, "quickjs job round limit");
            if (err && err_len > 0) {
                snprintf(err, err_len, "%s", host->exc_message ? host->exc_message : "quickjs job round limit");
            }
            return -1;
        }
        JSContext *ctx = NULL;
        int ran = JS_ExecutePendingJob(host->rt, &ctx);
        if (ran < 0) {
            stash_exception(host, ctx ? ctx : host->ctx, err, err_len);
            return -1;
        }
        if (ran == 0) {
            flush_rejections(host);
            return 0;
        }
        rounds++;
    }
    /* Only rejections that survived the whole queue without a handler are
       worth a log line. */
    flush_rejections(host);
    return 0;
}

void qjs_reset_rejections(Host *host) {
    for (size_t i = 0; i < host->reject_seen_len; i++) {
        JS_FreeValue(host->ctx, host->reject_seen[i]);
    }
    host->reject_seen_len = 0;
}

int qjs_jobs_pending(Host *host) {
    return host && JS_IsJobPending(host->rt) ? 1 : 0;
}

int qjs_tap_count(Host *host) {
    return host ? (int)host->tap_len : 0;
}

int qjs_scroll_state(Host *host, uint64_t id, int phase, float vx, float vy, float ox, float oy,
                      char *err, size_t err_len) {
    if (!host || !host->has_scroll_state) {
        return 0;
    }
    arm_slice(host, "scroll", id);
    Host *armed __attribute__((cleanup(disarm_entry_ptr))) = host;
    (void)armed;
    JSValue args[6];
    args[0] = handle_value(host->ctx, id);
    args[1] = JS_NewInt32(host->ctx, phase);
    args[2] = JS_NewFloat64(host->ctx, vx);
    args[3] = JS_NewFloat64(host->ctx, vy);
    args[4] = JS_NewFloat64(host->ctx, ox);
    args[5] = JS_NewFloat64(host->ctx, oy);
    JSValue result = JS_Call(host->ctx, host->scroll_state_cb, JS_UNDEFINED, 6, args);
    for (int index = 0; index < 6; index++) {
        JS_FreeValue(host->ctx, args[index]);
    }
    if (!JS_IsException(result)) {
        JS_FreeValue(host->ctx, result);
        return 0;
    }
    stash_exception(host, host->ctx, err, err_len);
    return -1;
}

int qjs_driver(Host *host, int kind, uint32_t index, uint32_t generation, int tag, int i0, int i1,
               float f0, float f1, float f2, float f3, float f4, float f5, const char *s0, size_t n0,
               const char *s1, size_t n1, char *err, size_t err_len) {
    if (!host || !host->has_driver) {
        return 0;
    }
    arm_slice(host, "driver", 0);
    Host *armed __attribute__((cleanup(disarm_entry_ptr))) = host;
    (void)armed;
    JSValue args[14];
    args[0] = JS_NewInt32(host->ctx, kind);
    args[1] = JS_NewUint32(host->ctx, index);
    args[2] = JS_NewUint32(host->ctx, generation);
    args[3] = JS_NewInt32(host->ctx, tag);
    args[4] = JS_NewInt32(host->ctx, i0);
    args[5] = JS_NewInt32(host->ctx, i1);
    args[6] = JS_NewFloat64(host->ctx, f0);
    args[7] = JS_NewFloat64(host->ctx, f1);
    args[8] = JS_NewFloat64(host->ctx, f2);
    args[9] = JS_NewFloat64(host->ctx, f3);
    args[10] = JS_NewFloat64(host->ctx, f4);
    args[11] = JS_NewFloat64(host->ctx, f5);
    args[12] = JS_NewStringLen(host->ctx, s0 ? s0 : "", n0);
    args[13] = JS_NewStringLen(host->ctx, s1 ? s1 : "", n1);
    JSValue result = JS_Call(host->ctx, host->driver_cb, JS_UNDEFINED, 14, args);
    for (int slot = 0; slot < 14; slot++) {
        JS_FreeValue(host->ctx, args[slot]);
    }
    if (!JS_IsException(result)) {
        JS_FreeValue(host->ctx, result);
        return 0;
    }
    stash_exception(host, host->ctx, err, err_len);
    return -1;
}

int qjs_layout(Host *host, char *err, size_t err_len) {
    if (!host || !host->has_layout) {
        return 0;
    }
    arm_slice(host, "layout", 0);
    Host *armed __attribute__((cleanup(disarm_entry_ptr))) = host;
    (void)armed;
    JSValue result = JS_Call(host->ctx, host->layout_cb, JS_UNDEFINED, 0, NULL);
    if (!JS_IsException(result)) {
        JS_FreeValue(host->ctx, result);
        return 0;
    }
    stash_exception(host, host->ctx, err, err_len);
    return -1;
}

int qjs_frame(Host *host, double target_ms, double interval_ms, char *err, size_t err_len) {
    arm_slice(host, "frame", 0);
    Host *armed __attribute__((cleanup(disarm_entry_ptr))) = host;
    (void)armed;
    if (!host->has_frame) {
        return 0;
    }
    JSValue args[2];
    args[0] = JS_NewFloat64(host->ctx, target_ms);
    args[1] = JS_NewFloat64(host->ctx, interval_ms);
    JSValue result = JS_Call(host->ctx, host->frame_cb, JS_UNDEFINED, 2, args);
    JS_FreeValue(host->ctx, args[0]);
    JS_FreeValue(host->ctx, args[1]);
    if (!JS_IsException(result)) {
        JS_FreeValue(host->ctx, result);
        return 0;
    }
    stash_exception(host, host->ctx, err, err_len);
    return -1;
}

int qjs_take_wants_frame(Host *host) {
    if (!host) {
        return 0;
    }
    int value = host->wants_frame;
    host->wants_frame = 0;
    return value;
}

int qjs_apply_settings(Host *host, const char *locale, size_t locale_len, double text_scale, int dark,
                       int reduce_motion, int bold_text, int high_contrast, char *err,
                       size_t err_len) {
    if (!host) {
        return -1;
    }
    free(host->locale);
    host->locale = NULL;
    host->locale_len = 0;
    if (locale && locale_len > 0) {
        char *copy = malloc(locale_len + 1);
        if (!copy) {
            return -1;
        }
        memcpy(copy, locale, locale_len);
        copy[locale_len] = 0;
        host->locale = copy;
        host->locale_len = locale_len;
    }
    host->settings_text_scale = text_scale;
    host->settings_dark = dark;
    host->settings_reduce = reduce_motion;
    host->settings_bold = bold_text;
    host->settings_contrast = high_contrast;
    if (host->settings_len == 0) {
        return 0;
    }
    arm_slice(host, "settings", 0);
    Host *armed __attribute__((cleanup(disarm_entry_ptr))) = host;
    (void)armed;
    JSValue object = settings_object(host->ctx, host);
    for (size_t index = 0; index < host->settings_len; index++) {
        JSValue fn = host->settings_handlers[index].fn;
        JSValue result = JS_Call(host->ctx, fn, JS_UNDEFINED, 1, &object);
        if (JS_IsException(result)) {
            JS_FreeValue(host->ctx, object);
            stash_exception(host, host->ctx, err, err_len);
            return -1;
        }
        JS_FreeValue(host->ctx, result);
    }
    JS_FreeValue(host->ctx, object);
    return 0;
}

int qjs_dispatch_back(Host *host, char *err, size_t err_len) {
    if (!host || host->back_len == 0) {
        return 0;
    }
    arm_slice(host, "back", 0);
    Host *armed __attribute__((cleanup(disarm_entry_ptr))) = host;
    (void)armed;
    for (size_t index = host->back_len; index > 0; index--) {
        JSValue fn = host->backs[index - 1].fn;
        JSValue result = JS_Call(host->ctx, fn, JS_UNDEFINED, 0, NULL);
        if (JS_IsException(result)) {
            stash_exception(host, host->ctx, err, err_len);
            return -1;
        }
        int handled = JS_ToBool(host->ctx, result);
        JS_FreeValue(host->ctx, result);
        if (handled > 0) {
            return 1;
        }
    }
    return 0;
}

int qjs_try_shortcut(Host *host, const char *spec, char *err, size_t err_len) {
    if (!host || !spec) {
        return 0;
    }
    for (size_t index = 0; index < host->shortcut_len; index++) {
        if (strcmp(host->shortcuts[index].spec, spec) != 0) {
            continue;
        }
        arm_slice(host, "shortcut", 0);
        Host *armed __attribute__((cleanup(disarm_entry_ptr))) = host;
        (void)armed;
    (void)armed;
        JSValue result = JS_Call(host->ctx, host->shortcuts[index].fn, JS_UNDEFINED, 0, NULL);
        if (JS_IsException(result)) {
            int failed = 0;
            listener_failed(host, err, err_len, &failed);
            return host->listener_freeze ? -1 : 1;
        }
        JS_FreeValue(host->ctx, result);
        return 1;
    }
    return 0;
}

#include "runtime_host.inc"

void qjs_take_error(Host *host, char **message, char **stack) {
    if (!host) {
        if (message) {
            *message = NULL;
        }
        if (stack) {
            *stack = NULL;
        }
        return;
    }
    if (message) {
        *message = host->exc_message;
    } else {
        free(host->exc_message);
    }
    if (stack) {
        *stack = host->exc_stack;
    } else {
        free(host->exc_stack);
    }
    host->exc_message = NULL;
    host->exc_stack = NULL;
}

void qjs_free(void *ptr) {
    free(ptr);
}

void qjs_close(Host *host) {
    if (!host) {
        return;
    }
    free(host->exc_message);
    free(host->exc_stack);
    host->exc_message = NULL;
    host->exc_stack = NULL;
    for (size_t index = 0; index < host->tap_len; index++) {
        free_tap(host, &host->taps[index]);
    }
    free(host->taps);
    for (size_t index = 0; index < host->back_len; index++) {
        JS_FreeValue(host->ctx, host->backs[index].fn);
    }
    free(host->backs);
    for (size_t index = 0; index < host->shortcut_len; index++) {
        free(host->shortcuts[index].spec);
        JS_FreeValue(host->ctx, host->shortcuts[index].fn);
    }
    free(host->shortcuts);
    for (size_t index = 0; index < host->settings_len; index++) {
        JS_FreeValue(host->ctx, host->settings_handlers[index].fn);
    }
    free(host->settings_handlers);
    for (size_t index = 0; index < host->mod_listener_len; index++) {
        free(host->mod_listeners[index].module);
        free(host->mod_listeners[index].event);
        JS_FreeValue(host->ctx, host->mod_listeners[index].fn);
    }
    free(host->mod_listeners);
    for (size_t index = 0; index < host->motion_len; index++) {
        free(host->motions[index].prop);
        JS_FreeValue(host->ctx, host->motions[index].fn);
    }
    free(host->motions);
    for (size_t index = 0; index < host->values_len; index++) {
        JS_FreeValue(host->ctx, host->values[index]);
    }
    free(host->values);
    for (size_t index = 0; index < host->reject_seen_len; index++) {
        JS_FreeValue(host->ctx, host->reject_seen[index]);
    }
    host->reject_seen_len = 0;
    free(host->locale);
    free(host->app_dir);
    if (host->has_frame) {
        JS_FreeValue(host->ctx, host->frame_cb);
    }
    if (host->has_layout) {
        JS_FreeValue(host->ctx, host->layout_cb);
    }
    if (host->has_scroll_state) {
        JS_FreeValue(host->ctx, host->scroll_state_cb);
    }
    if (host->has_driver) {
        JS_FreeValue(host->ctx, host->driver_cb);
    }
    free_timers(host);
    free_promises(host);
    if (host->ctx) {
        JS_FreeContext(host->ctx);
    }
    if (host->rt) {
        JS_FreeRuntime(host->rt);
    }
    free(host);
}

/* ---- devtools debug surface ---- */

#define DEBUG_LAYOUT_ISSUES 0
#define DEBUG_SNAPSHOT 1
#define DEBUG_PROBE 2
#define DEBUG_LAST_POINTER 3

static JSValue js_debug_json(JSContext *ctx, JSValueConst this_val, int argc,
                             JSValueConst *argv, int what) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (!host->fns.debug_json) {
        return JS_UNDEFINED;
    }
    uint64_t node = 0;
    double x = 0;
    double y = 0;
    if (argc >= 1 && read_handle(ctx, argv[0], &node)) {
        return JS_EXCEPTION;
    }
    if (argc >= 2 && JS_ToFloat64(ctx, &x, argv[1])) {
        return JS_EXCEPTION;
    }
    if (argc >= 3 && JS_ToFloat64(ctx, &y, argv[2])) {
        return JS_EXCEPTION;
    }
    if (host->fns.debug_json(host->fns.session, what, node, (float)x, (float)y)) {
        return JS_NULL;
    }
    const char *text = host->fns.debug_json_bytes(host->fns.session);
    if (!text) {
        return JS_NULL;
    }
    return JS_NewString(ctx, text);
}

static JSValue js_debug_layout_issues(JSContext *ctx, JSValueConst this_val, int argc,
                                      JSValueConst *argv) {
    return js_debug_json(ctx, this_val, argc, argv, DEBUG_LAYOUT_ISSUES);
}

static JSValue js_debug_snapshot(JSContext *ctx, JSValueConst this_val, int argc,
                                 JSValueConst *argv) {
    return js_debug_json(ctx, this_val, argc, argv, DEBUG_SNAPSHOT);
}

static JSValue js_debug_probe(JSContext *ctx, JSValueConst this_val, int argc,
                              JSValueConst *argv) {
    return js_debug_json(ctx, this_val, argc, argv, DEBUG_PROBE);
}

static JSValue js_debug_last_pointer(JSContext *ctx, JSValueConst this_val, int argc,
                                     JSValueConst *argv) {
    return js_debug_json(ctx, this_val, argc, argv, DEBUG_LAST_POINTER);
}

static JSValue js_debug_set_scan(JSContext *ctx, JSValueConst this_val, int argc,
                                 JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 1 || !host->fns.debug_set_scan) {
        return JS_UNDEFINED;
    }
    int on = JS_ToBool(ctx, argv[0]);
    if (on < 0) {
        return JS_EXCEPTION;
    }
    host->fns.debug_set_scan(host->fns.session, on);
    return JS_UNDEFINED;
}

static JSValue js_debug_set_pointer_trace(JSContext *ctx, JSValueConst this_val, int argc,
                                          JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 2 || !host->fns.debug_set_pointer_trace) {
        return JS_UNDEFINED;
    }
    int on = JS_ToBool(ctx, argv[0]);
    int suppress = JS_ToBool(ctx, argv[1]);
    if (on < 0 || suppress < 0) {
        return JS_EXCEPTION;
    }
    host->fns.debug_set_pointer_trace(host->fns.session, on, suppress);
    return JS_UNDEFINED;
}

static JSValue js_debug_memory(JSContext *ctx, JSValueConst this_val, int argc,
                                JSValueConst *argv) {
    (void)this_val;
    (void)argc;
    (void)argv;
    Host *host = host_of(ctx);
    JSMemoryUsage usage;
    memset(&usage, 0, sizeof(usage));
    JS_ComputeMemoryUsage(host->rt, &usage);
    char buf[256];
    snprintf(buf, sizeof(buf),
             "{\"mallocBytes\":%lld,\"objects\":%lld,\"atoms\":%lld,\"strings\":%lld}",
             (long long)usage.malloc_size, (long long)usage.memory_used_count,
             (long long)usage.atom_count, (long long)usage.str_count);
    return JS_NewString(ctx, buf);
}

static JSValue js_debug_set_owned(JSContext *ctx, JSValueConst this_val, int argc,
                                  JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 2 || !host->fns.debug_set_owned) {
        return JS_UNDEFINED;
    }
    uint64_t node = 0;
    if (read_handle(ctx, argv[0], &node) || !JS_IsBool(argv[1])) {
        return JS_EXCEPTION;
    }
    host->fns.debug_set_owned(host->fns.session, node, JS_ToBool(ctx, argv[1]));
    return JS_UNDEFINED;
}
