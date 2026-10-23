#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "quickjs.h"

/*
 * Handles are raw NodeIds. Generation lives in the high bits, so a live id is
 * never 0. Callers pass 0 for "no anchor" / "no node". A detached subtree's
 * onClick stays in `taps`: the region cannot be hit, so the closure is never
 * called. `remove` discards the subtree and frees those taps. `qjs_close`
 * frees whatever is still registered.
 */
typedef struct QjsFns {
    void *session;
    int (*column)(void *session, float gap, uint64_t *out);
    int (*row)(void *session, float gap, uint64_t *out);
    int (*stack)(void *session, uint64_t *out);
    int (*padding)(void *session, float edges, uint64_t *out);
    int (*pointer_region)(void *session, uint64_t *out);
    int (*decorated)(void *session, const uint8_t color[4], float radius, uint64_t *out);
    int (*text)(void *session, const char *utf8, size_t len, float font_size,
                const uint8_t color[4], uint64_t *out);
    int (*append)(void *session, uint64_t parent, uint64_t child);
    int (*insert)(void *session, uint64_t parent, uint64_t child, uint64_t before);
    int (*remove)(void *session, uint64_t parent, uint64_t child);
    int (*discard)(void *session, uint64_t id, uint64_t **out_ids, size_t *out_len);
    void (*free_ids)(uint64_t *ids, size_t len);
    int (*parent)(void *session, uint64_t node, uint64_t *out);
    int (*first_child)(void *session, uint64_t node, uint64_t *out);
    int (*next_sibling)(void *session, uint64_t node, uint64_t *out);
    int (*is_text)(void *session, uint64_t node);
    int (*set_text)(void *session, uint64_t node, const char *utf8, size_t len);
    int (*set_font_size)(void *session, uint64_t node, float font_size);
    int (*set_color)(void *session, uint64_t node, const uint8_t color[4]);
    int (*set_gap)(void *session, uint64_t node, float gap);
    int (*set_padding)(void *session, uint64_t node, float edges);
    int (*clip)(void *session, float radius, uint64_t *out);
    int (*opacity)(void *session, uint8_t alpha, uint64_t *out);
    int (*set_grow)(void *session, uint64_t node, float grow);
    int (*image)(void *session, const char *utf8, size_t len, float width, float height,
                 uint64_t *out);
    int (*set_radius)(void *session, uint64_t node, float radius);
    int (*set_alpha)(void *session, uint64_t node, uint8_t alpha);
    int (*set_align)(void *session, uint64_t node, uint32_t align);
    int (*set_size)(void *session, uint64_t node, float width, float height);
    int (*set_image)(void *session, uint64_t node, const char *utf8, size_t len, float width,
                     float height);
    int (*set_max_lines)(void *session, uint64_t node, uint32_t lines);
    int (*set_ellipsis)(void *session, uint64_t node, const char *utf8, size_t len);
    int (*set_text_align)(void *session, uint64_t node, uint32_t align);
    int (*set_font_weight)(void *session, uint64_t node, uint32_t weight);
    int (*set_text_direction)(void *session, uint64_t node, int rtl);
    int (*set_line_height)(void *session, uint64_t node, float height);
    int (*set_family)(void *session, uint64_t node, const char *utf8, size_t len);
    int (*scroll)(void *session, uint64_t *out);
    int (*set_scroll_offset)(void *session, uint64_t node, float dx, float dy);
    int (*scroll_offset)(void *session, uint64_t node, float *dx, float *dy);
    int (*rect)(void *session, uint64_t node, float *x, float *y, float *w, float *h);
    int (*transform)(void *session, uint64_t *out);
    int (*set_transform)(void *session, uint64_t node, float tx, float ty, float rotate, float scale);
    int (*set_justify)(void *session, uint64_t node, uint32_t justify);
    int (*set_wrap)(void *session, uint64_t node, int wrap);
    int (*set_fill)(void *session, uint64_t node);
    void (*log)(void *session, const char *utf8, size_t len, int level);
    int (*sized)(void *session, float width, float height, uint64_t *out);
    int (*set_focusable)(void *session, uint64_t node, int enabled);
    int (*focus)(void *session, uint64_t node);
    int (*blur)(void *session);
    int (*set_ime_enabled)(void *session, int enabled);
    int (*set_ime_text)(void *session, const char *utf8, size_t len);
    int (*measure_text)(void *session, const char *utf8, size_t len, float font_size,
                        float max_width, const uint8_t color[4], float *w, float *h);
} QjsFns;

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
} Tap;

typedef struct Host {
    JSRuntime *rt;
    JSContext *ctx;
    QjsFns fns;
    Tap *taps;
    size_t tap_len;
    size_t tap_cap;
    JSValue frame_cb;
    int has_frame;
    int wants_frame;
} Host;

static JSValue throw_msg(JSContext *ctx, const char *msg) {
    return JS_ThrowTypeError(ctx, "%s", msg);
}

static Host *host_of(JSContext *ctx) {
    return JS_GetContextOpaque(ctx);
}

static void write_exception(JSContext *ctx, char *err, size_t err_len) {
    JSValue exception = JS_GetException(ctx);
    const char *text = JS_ToCString(ctx, exception);
    JSValue stack_val = JS_GetPropertyStr(ctx, exception, "stack");
    const char *stack = NULL;
    if (!JS_IsException(stack_val) && !JS_IsUndefined(stack_val)) {
        stack = JS_ToCString(ctx, stack_val);
    }
    if (err && err_len > 0) {
        snprintf(err, err_len, "%s%s%s", text ? text : "quickjs error", stack ? "\n" : "",
                 stack ? stack : "");
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

static JSValue js_column(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 1) {
        return throw_msg(ctx, "column(gap)");
    }
    double gap = 0;
    if (JS_ToFloat64(ctx, &gap, argv[0])) {
        return JS_EXCEPTION;
    }
    uint64_t id = 0;
    int status = host->fns.column(host->fns.session, (float)gap, &id);
    return created(ctx, status, id, "column failed");
}

static JSValue js_row(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 1) {
        return throw_msg(ctx, "row(gap)");
    }
    double gap = 0;
    if (JS_ToFloat64(ctx, &gap, argv[0])) {
        return JS_EXCEPTION;
    }
    uint64_t id = 0;
    int status = host->fns.row(host->fns.session, (float)gap, &id);
    return created(ctx, status, id, "row failed");
}

static JSValue js_stack(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    (void)argc;
    (void)argv;
    Host *host = host_of(ctx);
    uint64_t id = 0;
    int status = host->fns.stack(host->fns.session, &id);
    return created(ctx, status, id, "stack failed");
}

static JSValue js_padding(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 1) {
        return throw_msg(ctx, "padding(edges)");
    }
    double edges = 0;
    if (JS_ToFloat64(ctx, &edges, argv[0])) {
        return JS_EXCEPTION;
    }
    uint64_t id = 0;
    int status = host->fns.padding(host->fns.session, (float)edges, &id);
    return created(ctx, status, id, "padding failed");
}

static JSValue js_pointer_region(JSContext *ctx, JSValueConst this_val, int argc,
                                 JSValueConst *argv) {
    (void)this_val;
    (void)argc;
    (void)argv;
    Host *host = host_of(ctx);
    uint64_t id = 0;
    int status = host->fns.pointer_region(host->fns.session, &id);
    return created(ctx, status, id, "pointerRegion failed");
}

static JSValue js_decorated(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 1) {
        return throw_msg(ctx, "decorated(color)");
    }
    uint8_t color[4];
    if (read_color(ctx, argv[0], color)) {
        return throw_msg(ctx, "color must be [r,g,b,a]");
    }
    double radius = 0;
    if (argc >= 2 && JS_ToFloat64(ctx, &radius, argv[1])) {
        return JS_EXCEPTION;
    }
    uint64_t id = 0;
    int status = host->fns.decorated(host->fns.session, color, (float)radius, &id);
    return created(ctx, status, id, "decorated failed");
}

static JSValue js_text(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 3) {
        return throw_msg(ctx, "text(string, fontSize, color)");
    }
    size_t len = 0;
    const char *utf8 = JS_ToCStringLen(ctx, &len, argv[0]);
    if (!utf8) {
        return JS_EXCEPTION;
    }
    double font_size = 0;
    if (JS_ToFloat64(ctx, &font_size, argv[1])) {
        JS_FreeCString(ctx, utf8);
        return JS_EXCEPTION;
    }
    uint8_t color[4];
    if (read_color(ctx, argv[2], color)) {
        JS_FreeCString(ctx, utf8);
        return throw_msg(ctx, "color must be [r,g,b,a]");
    }
    uint64_t id = 0;
    int status =
        host->fns.text(host->fns.session, utf8, len, (float)font_size, color, &id);
    JS_FreeCString(ctx, utf8);
    return created(ctx, status, id, "text failed");
}

static JSValue js_append(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 2) {
        return throw_msg(ctx, "append(parent, child)");
    }
    uint64_t parent = 0;
    uint64_t child = 0;
    if (read_handle(ctx, argv[0], &parent) || read_handle(ctx, argv[1], &child)) {
        return JS_EXCEPTION;
    }
    return done(ctx, host->fns.append(host->fns.session, parent, child), "append failed");
}

static JSValue js_insert(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 2) {
        return throw_msg(ctx, "insert(parent, child, before)");
    }
    uint64_t parent = 0;
    uint64_t child = 0;
    uint64_t before = 0;
    if (read_handle(ctx, argv[0], &parent) || read_handle(ctx, argv[1], &child) ||
        read_optional_handle(ctx, argc, argv, 2, &before)) {
        return JS_EXCEPTION;
    }
    return done(ctx, host->fns.insert(host->fns.session, parent, child, before), "insert failed");
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

static JSValue js_remove(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 2) {
        return throw_msg(ctx, "remove(parent, child)");
    }
    uint64_t parent = 0;
    uint64_t child = 0;
    if (read_handle(ctx, argv[0], &parent) || read_handle(ctx, argv[1], &child)) {
        return JS_EXCEPTION;
    }
    if (host->fns.remove(host->fns.session, parent, child)) {
        return throw_msg(ctx, "remove failed");
    }
    uint64_t *ids = NULL;
    size_t len = 0;
    if (host->fns.discard(host->fns.session, child, &ids, &len)) {
        return throw_msg(ctx, "remove failed");
    }
    forget_taps(host, ids, len);
    host->fns.free_ids(ids, len);
    return JS_UNDEFINED;
}

static JSValue js_parent(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 1) {
        return throw_msg(ctx, "parent(node)");
    }
    uint64_t node = 0;
    if (read_handle(ctx, argv[0], &node)) {
        return JS_EXCEPTION;
    }
    uint64_t out = 0;
    int status = host->fns.parent(host->fns.session, node, &out);
    if (status) {
        return throw_msg(ctx, "parent failed");
    }
    return handle_value(ctx, out);
}

static JSValue js_first_child(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 1) {
        return throw_msg(ctx, "firstChild(node)");
    }
    uint64_t node = 0;
    if (read_handle(ctx, argv[0], &node)) {
        return JS_EXCEPTION;
    }
    uint64_t out = 0;
    int status = host->fns.first_child(host->fns.session, node, &out);
    if (status) {
        return throw_msg(ctx, "firstChild failed");
    }
    return handle_value(ctx, out);
}

static JSValue js_next_sibling(JSContext *ctx, JSValueConst this_val, int argc,
                               JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 1) {
        return throw_msg(ctx, "nextSibling(node)");
    }
    uint64_t node = 0;
    if (read_handle(ctx, argv[0], &node)) {
        return JS_EXCEPTION;
    }
    uint64_t out = 0;
    int status = host->fns.next_sibling(host->fns.session, node, &out);
    if (status) {
        return throw_msg(ctx, "nextSibling failed");
    }
    return handle_value(ctx, out);
}

static JSValue js_is_text(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 1) {
        return throw_msg(ctx, "isText(node)");
    }
    uint64_t node = 0;
    if (read_handle(ctx, argv[0], &node)) {
        return JS_EXCEPTION;
    }
    int flag = host->fns.is_text(host->fns.session, node);
    if (flag < 0) {
        return throw_msg(ctx, "isText failed");
    }
    return JS_NewBool(ctx, flag);
}

static JSValue js_set_text(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 2) {
        return throw_msg(ctx, "setText(node, string)");
    }
    uint64_t node = 0;
    if (read_handle(ctx, argv[0], &node)) {
        return JS_EXCEPTION;
    }
    size_t len = 0;
    const char *utf8 = JS_ToCStringLen(ctx, &len, argv[1]);
    if (!utf8) {
        return JS_EXCEPTION;
    }
    int status = host->fns.set_text(host->fns.session, node, utf8, len);
    JS_FreeCString(ctx, utf8);
    return done(ctx, status, "setText failed");
}

static JSValue js_set_font_size(JSContext *ctx, JSValueConst this_val, int argc,
                                JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 2) {
        return throw_msg(ctx, "setFontSize(node, size)");
    }
    uint64_t node = 0;
    double font_size = 0;
    if (read_handle(ctx, argv[0], &node) || JS_ToFloat64(ctx, &font_size, argv[1])) {
        return JS_EXCEPTION;
    }
    return done(ctx, host->fns.set_font_size(host->fns.session, node, (float)font_size),
                "setFontSize failed");
}

static JSValue js_set_color(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 2) {
        return throw_msg(ctx, "setColor(node, color)");
    }
    uint64_t node = 0;
    if (read_handle(ctx, argv[0], &node)) {
        return JS_EXCEPTION;
    }
    uint8_t color[4];
    if (read_color(ctx, argv[1], color)) {
        return throw_msg(ctx, "color must be [r,g,b,a]");
    }
    return done(ctx, host->fns.set_color(host->fns.session, node, color), "setColor failed");
}

static JSValue js_set_gap(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 2) {
        return throw_msg(ctx, "setGap(node, gap)");
    }
    uint64_t node = 0;
    double gap = 0;
    if (read_handle(ctx, argv[0], &node) || JS_ToFloat64(ctx, &gap, argv[1])) {
        return JS_EXCEPTION;
    }
    return done(ctx, host->fns.set_gap(host->fns.session, node, (float)gap), "setGap failed");
}

static JSValue js_set_padding(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 2) {
        return throw_msg(ctx, "setPadding(node, edges)");
    }
    uint64_t node = 0;
    double edges = 0;
    if (read_handle(ctx, argv[0], &node) || JS_ToFloat64(ctx, &edges, argv[1])) {
        return JS_EXCEPTION;
    }
    return done(ctx, host->fns.set_padding(host->fns.session, node, (float)edges),
                "setPadding failed");
}

static JSValue js_clip(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 1) {
        return throw_msg(ctx, "clip(radius)");
    }
    double radius = 0;
    if (JS_ToFloat64(ctx, &radius, argv[0])) {
        return JS_EXCEPTION;
    }
    uint64_t id = 0;
    int status = host->fns.clip(host->fns.session, (float)radius, &id);
    return created(ctx, status, id, "clip failed");
}

static JSValue js_opacity(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 1) {
        return throw_msg(ctx, "opacity(alpha)");
    }
    double alpha = 0;
    if (JS_ToFloat64(ctx, &alpha, argv[0])) {
        return JS_EXCEPTION;
    }
    if (!(alpha >= 0.0 && alpha <= 255.0)) {
        return throw_msg(ctx, "opacity alpha");
    }
    uint64_t id = 0;
    int status = host->fns.opacity(host->fns.session, (uint8_t)alpha, &id);
    return created(ctx, status, id, "opacity failed");
}

static JSValue js_set_grow(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 2) {
        return throw_msg(ctx, "setGrow(node, grow)");
    }
    uint64_t node = 0;
    double grow = 0;
    if (read_handle(ctx, argv[0], &node) || JS_ToFloat64(ctx, &grow, argv[1])) {
        return JS_EXCEPTION;
    }
    return done(ctx, host->fns.set_grow(host->fns.session, node, (float)grow), "setGrow failed");
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

static JSValue js_image(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 3) {
        return throw_msg(ctx, "image(source, w, h)");
    }
    size_t len = 0;
    const char *utf8 = JS_ToCStringLen(ctx, &len, argv[0]);
    if (!utf8) {
        return JS_EXCEPTION;
    }
    double width = 0;
    double height = 0;
    if (JS_ToFloat64(ctx, &width, argv[1]) || JS_ToFloat64(ctx, &height, argv[2])) {
        JS_FreeCString(ctx, utf8);
        return JS_EXCEPTION;
    }
    uint64_t id = 0;
    int status = host->fns.image(host->fns.session, utf8, len, (float)width, (float)height, &id);
    JS_FreeCString(ctx, utf8);
    return created(ctx, status, id, "image failed");
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

static JSValue js_on_key(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    return bind_gesture(ctx, argc, argv, 8, "onKey(node, function)");
}

static JSValue js_on_text(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    return bind_gesture(ctx, argc, argv, 9, "onText(node, function)");
}

static JSValue js_on_composing(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    return bind_gesture(ctx, argc, argv, 10, "onComposing(node, function)");
}

static JSValue js_sized(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 2) {
        return throw_msg(ctx, "sized(width, height)");
    }
    double width = 0;
    double height = 0;
    if (JS_ToFloat64(ctx, &width, argv[0]) || JS_ToFloat64(ctx, &height, argv[1])) {
        return JS_EXCEPTION;
    }
    uint64_t id = 0;
    int status = host->fns.sized(host->fns.session, (float)width, (float)height, &id);
    return created(ctx, status, id, "sized failed");
}

static JSValue js_set_focusable(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 2) {
        return throw_msg(ctx, "setFocusable(node, enabled)");
    }
    uint64_t node = 0;
    if (read_handle(ctx, argv[0], &node)) {
        return JS_EXCEPTION;
    }
    int enabled = JS_ToBool(ctx, argv[1]);
    if (enabled < 0) {
        return JS_EXCEPTION;
    }
    return done(ctx, host->fns.set_focusable(host->fns.session, node, enabled), "setFocusable failed");
}

static JSValue js_focus(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 1) {
        return throw_msg(ctx, "focus(node)");
    }
    uint64_t node = 0;
    if (read_handle(ctx, argv[0], &node)) {
        return JS_EXCEPTION;
    }
    return done(ctx, host->fns.focus(host->fns.session, node), "focus failed");
}

static JSValue js_blur(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    (void)argc;
    (void)argv;
    Host *host = host_of(ctx);
    return done(ctx, host->fns.blur(host->fns.session), "blur failed");
}

static JSValue js_set_ime_enabled(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 1) {
        return throw_msg(ctx, "setImeEnabled(enabled)");
    }
    int enabled = JS_ToBool(ctx, argv[0]);
    if (enabled < 0) {
        return JS_EXCEPTION;
    }
    return done(ctx, host->fns.set_ime_enabled(host->fns.session, enabled), "setImeEnabled failed");
}

static JSValue js_set_ime_text(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 1) {
        return throw_msg(ctx, "setImeText(text)");
    }
    size_t len = 0;
    const char *utf8 = JS_ToCStringLen(ctx, &len, argv[0]);
    if (!utf8) {
        return JS_EXCEPTION;
    }
    int status = host->fns.set_ime_text(host->fns.session, utf8, len);
    JS_FreeCString(ctx, utf8);
    return done(ctx, status, "setImeText failed");
}

static JSValue js_measure_text(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 4) {
        return throw_msg(ctx, "measureText(text, fontSize, maxWidth, color)");
    }
    size_t len = 0;
    const char *utf8 = JS_ToCStringLen(ctx, &len, argv[0]);
    if (!utf8) {
        return JS_EXCEPTION;
    }
    double font_size = 0;
    double max_width = 0;
    uint8_t color[4];
    if (JS_ToFloat64(ctx, &font_size, argv[1]) || JS_ToFloat64(ctx, &max_width, argv[2]) ||
        read_color(ctx, argv[3], color)) {
        JS_FreeCString(ctx, utf8);
        return JS_EXCEPTION;
    }
    float width = 0;
    float height = 0;
    int status = host->fns.measure_text(host->fns.session, utf8, len, (float)font_size,
                                        (float)max_width, color, &width, &height);
    JS_FreeCString(ctx, utf8);
    if (status) {
        return throw_msg(ctx, "measureText failed");
    }
    JSValue result = JS_NewObject(ctx);
    JS_SetPropertyStr(ctx, result, "w", JS_NewFloat64(ctx, width));
    JS_SetPropertyStr(ctx, result, "h", JS_NewFloat64(ctx, height));
    return result;
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

static JSValue js_set_radius(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 2) {
        return throw_msg(ctx, "setRadius(node, radius)");
    }
    uint64_t node = 0;
    double radius = 0;
    if (read_handle(ctx, argv[0], &node) || JS_ToFloat64(ctx, &radius, argv[1])) {
        return JS_EXCEPTION;
    }
    int status = host->fns.set_radius(host->fns.session, node, (float)radius);
    return done(ctx, status, "setRadius failed");
}

static JSValue js_set_alpha(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 2) {
        return throw_msg(ctx, "setAlpha(node, alpha)");
    }
    uint64_t node = 0;
    int32_t alpha = 0;
    if (read_handle(ctx, argv[0], &node) || JS_ToInt32(ctx, &alpha, argv[1])) {
        return JS_EXCEPTION;
    }
    if (alpha < 0) {
        alpha = 0;
    }
    if (alpha > 255) {
        alpha = 255;
    }
    int status = host->fns.set_alpha(host->fns.session, node, (uint8_t)alpha);
    return done(ctx, status, "setAlpha failed");
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

static JSValue js_set_align(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 2) {
        return throw_msg(ctx, "setAlign(node, align)");
    }
    uint64_t node = 0;
    if (read_handle(ctx, argv[0], &node)) {
        return JS_EXCEPTION;
    }
    const char *name = JS_ToCString(ctx, argv[1]);
    if (!name) {
        return JS_EXCEPTION;
    }
    uint32_t align = 0;
    int bad = named_align(name, &align, "start", 0, "center", 1, "end", 2);
    JS_FreeCString(ctx, name);
    if (bad) {
        return throw_msg(ctx, "setAlign align");
    }
    int status = host->fns.set_align(host->fns.session, node, align);
    return done(ctx, status, "setAlign failed");
}

static JSValue js_set_size(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 3) {
        return throw_msg(ctx, "setSize(node, width, height)");
    }
    uint64_t node = 0;
    double width = 0;
    double height = 0;
    if (read_handle(ctx, argv[0], &node) || JS_ToFloat64(ctx, &width, argv[1]) ||
        JS_ToFloat64(ctx, &height, argv[2])) {
        return JS_EXCEPTION;
    }
    int status = host->fns.set_size(host->fns.session, node, (float)width, (float)height);
    return done(ctx, status, "setSize failed");
}

static JSValue js_set_image(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 4) {
        return throw_msg(ctx, "setImage(node, source, width, height)");
    }
    uint64_t node = 0;
    if (read_handle(ctx, argv[0], &node)) {
        return JS_EXCEPTION;
    }
    size_t len = 0;
    const char *utf8 = JS_ToCStringLen(ctx, &len, argv[1]);
    if (!utf8) {
        return JS_EXCEPTION;
    }
    double width = 0;
    double height = 0;
    if (JS_ToFloat64(ctx, &width, argv[2]) || JS_ToFloat64(ctx, &height, argv[3])) {
        JS_FreeCString(ctx, utf8);
        return JS_EXCEPTION;
    }
    int status = host->fns.set_image(host->fns.session, node, utf8, len, (float)width, (float)height);
    JS_FreeCString(ctx, utf8);
    return done(ctx, status, "setImage failed");
}

static JSValue js_set_max_lines(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 2) {
        return throw_msg(ctx, "setMaxLines(node, lines)");
    }
    uint64_t node = 0;
    uint32_t lines = 0;
    if (read_handle(ctx, argv[0], &node) || JS_ToUint32(ctx, &lines, argv[1])) {
        return JS_EXCEPTION;
    }
    int status = host->fns.set_max_lines(host->fns.session, node, lines);
    return done(ctx, status, "setMaxLines failed");
}

static JSValue js_set_ellipsis(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 2) {
        return throw_msg(ctx, "setEllipsis(node, text)");
    }
    uint64_t node = 0;
    if (read_handle(ctx, argv[0], &node)) {
        return JS_EXCEPTION;
    }
    size_t len = 0;
    const char *utf8 = JS_ToCStringLen(ctx, &len, argv[1]);
    if (!utf8) {
        return JS_EXCEPTION;
    }
    int status = host->fns.set_ellipsis(host->fns.session, node, utf8, len);
    JS_FreeCString(ctx, utf8);
    return done(ctx, status, "setEllipsis failed");
}

static JSValue js_set_text_align(JSContext *ctx, JSValueConst this_val, int argc,
                                 JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 2) {
        return throw_msg(ctx, "setTextAlign(node, align)");
    }
    uint64_t node = 0;
    if (read_handle(ctx, argv[0], &node)) {
        return JS_EXCEPTION;
    }
    const char *name = JS_ToCString(ctx, argv[1]);
    if (!name) {
        return JS_EXCEPTION;
    }
    uint32_t align = 0;
    int bad = named_align(name, &align, "left", 0, "center", 1, "right", 2);
    JS_FreeCString(ctx, name);
    if (bad) {
        return throw_msg(ctx, "setTextAlign align");
    }
    int status = host->fns.set_text_align(host->fns.session, node, align);
    return done(ctx, status, "setTextAlign failed");
}

static JSValue js_set_font_weight(JSContext *ctx, JSValueConst this_val, int argc,
                                  JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 2) {
        return throw_msg(ctx, "setFontWeight(node, weight)");
    }
    uint64_t node = 0;
    uint32_t weight = 0;
    if (read_handle(ctx, argv[0], &node) || JS_ToUint32(ctx, &weight, argv[1])) {
        return JS_EXCEPTION;
    }
    int status = host->fns.set_font_weight(host->fns.session, node, weight);
    return done(ctx, status, "setFontWeight failed");
}

static JSValue js_set_text_direction(JSContext *ctx, JSValueConst this_val, int argc,
                                     JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 2) {
        return throw_msg(ctx, "setTextDirection(node, direction)");
    }
    uint64_t node = 0;
    if (read_handle(ctx, argv[0], &node)) {
        return JS_EXCEPTION;
    }
    const char *name = JS_ToCString(ctx, argv[1]);
    if (!name) {
        return JS_EXCEPTION;
    }
    int rtl = strcmp(name, "rtl") == 0;
    int bad = !rtl && strcmp(name, "ltr") != 0;
    JS_FreeCString(ctx, name);
    if (bad) {
        return throw_msg(ctx, "setTextDirection direction");
    }
    int status = host->fns.set_text_direction(host->fns.session, node, rtl);
    return done(ctx, status, "setTextDirection failed");
}

static JSValue js_set_line_height(JSContext *ctx, JSValueConst this_val, int argc,
                                  JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 2) {
        return throw_msg(ctx, "setLineHeight(node, height)");
    }
    uint64_t node = 0;
    double height = 0;
    if (read_handle(ctx, argv[0], &node) || JS_ToFloat64(ctx, &height, argv[1])) {
        return JS_EXCEPTION;
    }
    int status = host->fns.set_line_height(host->fns.session, node, (float)height);
    return done(ctx, status, "setLineHeight failed");
}

static JSValue js_set_family(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 2) {
        return throw_msg(ctx, "setFamily(node, family)");
    }
    uint64_t node = 0;
    if (read_handle(ctx, argv[0], &node)) {
        return JS_EXCEPTION;
    }
    size_t len = 0;
    const char *utf8 = JS_ToCStringLen(ctx, &len, argv[1]);
    if (!utf8) {
        return JS_EXCEPTION;
    }
    int status = host->fns.set_family(host->fns.session, node, utf8, len);
    JS_FreeCString(ctx, utf8);
    return done(ctx, status, "setFamily failed");
}

static JSValue js_scroll(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    (void)argc;
    (void)argv;
    Host *host = host_of(ctx);
    uint64_t id = 0;
    int status = host->fns.scroll(host->fns.session, &id);
    return created(ctx, status, id, "scroll failed");
}

static JSValue js_transform(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    (void)argc;
    (void)argv;
    Host *host = host_of(ctx);
    uint64_t id = 0;
    int status = host->fns.transform(host->fns.session, &id);
    return created(ctx, status, id, "transform failed");
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

static JSValue js_set_scroll_offset(JSContext *ctx, JSValueConst this_val, int argc,
                                    JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 3) {
        return throw_msg(ctx, "setScrollOffset(node, x, y)");
    }
    uint64_t node = 0;
    double x = 0;
    double y = 0;
    if (read_handle(ctx, argv[0], &node) || JS_ToFloat64(ctx, &x, argv[1]) ||
        JS_ToFloat64(ctx, &y, argv[2])) {
        return JS_EXCEPTION;
    }
    int status = host->fns.set_scroll_offset(host->fns.session, node, (float)x, (float)y);
    return done(ctx, status, "setScrollOffset failed");
}

static JSValue js_scroll_offset(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 1) {
        return throw_msg(ctx, "scrollOffset(node)");
    }
    uint64_t node = 0;
    float x = 0;
    float y = 0;
    if (read_handle(ctx, argv[0], &node)) {
        return JS_EXCEPTION;
    }
    if (host->fns.scroll_offset(host->fns.session, node, &x, &y)) {
        return throw_msg(ctx, "scrollOffset failed");
    }
    return point_object(ctx, x, y, 0, 0, 0);
}

static JSValue js_rect(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 1) {
        return throw_msg(ctx, "rect(node)");
    }
    uint64_t node = 0;
    float x = 0;
    float y = 0;
    float w = 0;
    float h = 0;
    if (read_handle(ctx, argv[0], &node)) {
        return JS_EXCEPTION;
    }
    if (host->fns.rect(host->fns.session, node, &x, &y, &w, &h)) {
        return throw_msg(ctx, "rect failed");
    }
    return point_object(ctx, x, y, w, h, 1);
}

static JSValue js_set_transform(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 5) {
        return throw_msg(ctx, "setTransform(node, x, y, rotate, scale)");
    }
    uint64_t node = 0;
    double x = 0;
    double y = 0;
    double rotate = 0;
    double scale = 0;
    if (read_handle(ctx, argv[0], &node) || JS_ToFloat64(ctx, &x, argv[1]) ||
        JS_ToFloat64(ctx, &y, argv[2]) || JS_ToFloat64(ctx, &rotate, argv[3]) ||
        JS_ToFloat64(ctx, &scale, argv[4])) {
        return JS_EXCEPTION;
    }
    int status = host->fns.set_transform(host->fns.session, node, (float)x, (float)y, (float)rotate,
                                         (float)scale);
    return done(ctx, status, "setTransform failed");
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

static JSValue js_set_justify(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 2) {
        return throw_msg(ctx, "setJustify(node, justify)");
    }
    uint64_t node = 0;
    if (read_handle(ctx, argv[0], &node)) {
        return JS_EXCEPTION;
    }
    const char *name = JS_ToCString(ctx, argv[1]);
    if (!name) {
        return JS_EXCEPTION;
    }
    uint32_t justify = 0;
    int bad = justify_value(name, &justify);
    JS_FreeCString(ctx, name);
    if (bad) {
        return throw_msg(ctx, "setJustify justify");
    }
    int status = host->fns.set_justify(host->fns.session, node, justify);
    return done(ctx, status, "setJustify failed");
}

static JSValue js_set_wrap(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 2) {
        return throw_msg(ctx, "setWrap(node, wrap)");
    }
    uint64_t node = 0;
    if (read_handle(ctx, argv[0], &node)) {
        return JS_EXCEPTION;
    }
    int wrap = JS_ToBool(ctx, argv[1]);
    if (wrap < 0) {
        return JS_EXCEPTION;
    }
    int status = host->fns.set_wrap(host->fns.session, node, wrap);
    return done(ctx, status, "setWrap failed");
}

static JSValue js_set_fill(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 1) {
        return throw_msg(ctx, "setFill(node)");
    }
    uint64_t node = 0;
    if (read_handle(ctx, argv[0], &node)) {
        return JS_EXCEPTION;
    }
    int status = host->fns.set_fill(host->fns.session, node);
    return done(ctx, status, "setFill failed");
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

static int bind_fn(JSContext *ctx, JSValue core, const char *name, JSCFunction *fn, int length);

static int bind_console(Host *host) {
    JSContext *ctx = host->ctx;
    JSValue global = JS_GetGlobalObject(ctx);
    JSValue console = JS_NewObject(ctx);
    int failed = bind_fn(ctx, console, "log", js_console_log, 0);
    failed |= bind_fn(ctx, console, "info", js_console_info, 0);
    failed |= bind_fn(ctx, console, "warn", js_console_warn, 0);
    failed |= bind_fn(ctx, console, "error", js_console_error, 0);
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

static int bind_core(Host *host) {
    JSContext *ctx = host->ctx;
    JSValue global = JS_GetGlobalObject(ctx);
    JSValue core = JS_NewObject(ctx);
    int failed = bind_fn(ctx, core, "column", js_column, 1);
    failed |= bind_fn(ctx, core, "row", js_row, 1);
    failed |= bind_fn(ctx, core, "stack", js_stack, 0);
    failed |= bind_fn(ctx, core, "padding", js_padding, 1);
    failed |= bind_fn(ctx, core, "pointerRegion", js_pointer_region, 0);
    failed |= bind_fn(ctx, core, "decorated", js_decorated, 1);
    failed |= bind_fn(ctx, core, "text", js_text, 3);
    failed |= bind_fn(ctx, core, "append", js_append, 2);
    failed |= bind_fn(ctx, core, "insert", js_insert, 3);
    failed |= bind_fn(ctx, core, "remove", js_remove, 2);
    failed |= bind_fn(ctx, core, "parent", js_parent, 1);
    failed |= bind_fn(ctx, core, "firstChild", js_first_child, 1);
    failed |= bind_fn(ctx, core, "nextSibling", js_next_sibling, 1);
    failed |= bind_fn(ctx, core, "isText", js_is_text, 1);
    failed |= bind_fn(ctx, core, "setText", js_set_text, 2);
    failed |= bind_fn(ctx, core, "setFontSize", js_set_font_size, 2);
    failed |= bind_fn(ctx, core, "setColor", js_set_color, 2);
    failed |= bind_fn(ctx, core, "setGap", js_set_gap, 2);
    failed |= bind_fn(ctx, core, "setPadding", js_set_padding, 2);
    failed |= bind_fn(ctx, core, "clip", js_clip, 1);
    failed |= bind_fn(ctx, core, "opacity", js_opacity, 1);
    failed |= bind_fn(ctx, core, "setGrow", js_set_grow, 2);
    failed |= bind_fn(ctx, core, "image", js_image, 3);
    failed |= bind_fn(ctx, core, "setRadius", js_set_radius, 2);
    failed |= bind_fn(ctx, core, "setAlpha", js_set_alpha, 2);
    failed |= bind_fn(ctx, core, "setAlign", js_set_align, 2);
    failed |= bind_fn(ctx, core, "setSize", js_set_size, 3);
    failed |= bind_fn(ctx, core, "setImage", js_set_image, 4);
    failed |= bind_fn(ctx, core, "setMaxLines", js_set_max_lines, 2);
    failed |= bind_fn(ctx, core, "setEllipsis", js_set_ellipsis, 2);
    failed |= bind_fn(ctx, core, "setTextAlign", js_set_text_align, 2);
    failed |= bind_fn(ctx, core, "setFontWeight", js_set_font_weight, 2);
    failed |= bind_fn(ctx, core, "setTextDirection", js_set_text_direction, 2);
    failed |= bind_fn(ctx, core, "setLineHeight", js_set_line_height, 2);
    failed |= bind_fn(ctx, core, "setFamily", js_set_family, 2);
    failed |= bind_fn(ctx, core, "scroll", js_scroll, 0);
    failed |= bind_fn(ctx, core, "setScrollOffset", js_set_scroll_offset, 3);
    failed |= bind_fn(ctx, core, "scrollOffset", js_scroll_offset, 1);
    failed |= bind_fn(ctx, core, "rect", js_rect, 1);
    failed |= bind_fn(ctx, core, "transform", js_transform, 0);
    failed |= bind_fn(ctx, core, "setTransform", js_set_transform, 5);
    failed |= bind_fn(ctx, core, "setJustify", js_set_justify, 2);
    failed |= bind_fn(ctx, core, "setWrap", js_set_wrap, 2);
    failed |= bind_fn(ctx, core, "setFill", js_set_fill, 1);
    failed |= bind_fn(ctx, core, "sized", js_sized, 2);
    failed |= bind_fn(ctx, core, "setFocusable", js_set_focusable, 2);
    failed |= bind_fn(ctx, core, "focus", js_focus, 1);
    failed |= bind_fn(ctx, core, "blur", js_blur, 0);
    failed |= bind_fn(ctx, core, "setImeEnabled", js_set_ime_enabled, 1);
    failed |= bind_fn(ctx, core, "setImeText", js_set_ime_text, 1);
    failed |= bind_fn(ctx, core, "measureText", js_measure_text, 4);
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
    failed |= bind_fn(ctx, core, "onComposing", js_on_composing, 2);
    failed |= bind_fn(ctx, core, "onFrame", js_on_frame, 1);
    /* JS_SetPropertyStr takes ownership of `core`. Success is 1, not 0. */
    failed |= JS_SetPropertyStr(ctx, global, "core", core) < 0;
    JS_FreeValue(ctx, global);
    return failed ? -1 : 0;
}

Host *qjs_open(const QjsFns *fns) {
    Host *host = calloc(1, sizeof(Host));
    if (!host) {
        return NULL;
    }
    host->fns = *fns;
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
    if (bind_core(host) || bind_console(host)) {
        JS_FreeContext(host->ctx);
        JS_FreeRuntime(host->rt);
        free(host->taps);
        free(host);
        return NULL;
    }
    return host;
}

int qjs_eval(Host *host, const char *src, size_t len, const char *name, char *err, size_t err_len) {
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
    JSValue result = JS_Eval(host->ctx, owned, len, name, JS_EVAL_TYPE_GLOBAL);
    free(owned);
    if (!JS_IsException(result)) {
        JS_FreeValue(host->ctx, result);
        return 0;
    }
    write_exception(host->ctx, err, err_len);
    return -1;
}

int qjs_dispatch(Host *host, uint64_t id, int kind, double a, double b, char *err, size_t err_len) {
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
        if (kind == 4 || kind == 5) {
            args[0] = JS_NewFloat64(host->ctx, a);
            args[1] = JS_NewFloat64(host->ctx, b);
            argc = 2;
        }
        JSValue result = JS_Call(host->ctx, *slot, JS_UNDEFINED, argc, argc ? args : NULL);
        if (argc) {
            JS_FreeValue(host->ctx, args[0]);
            JS_FreeValue(host->ctx, args[1]);
        }
        if (!JS_IsException(result)) {
            JS_FreeValue(host->ctx, result);
            continue;
        }
        if (!failed) {
            write_exception(host->ctx, err, err_len);
        } else {
            JSValue exception = JS_GetException(host->ctx);
            JS_FreeValue(host->ctx, exception);
        }
        failed = 1;
    }
    return failed ? -1 : 0;
}

int qjs_dispatch_input(Host *host, uint64_t id, int kind, const char *utf8, size_t len, char *err,
                       size_t err_len) {
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
        JSValue arg;
        if (utf8 == NULL) {
            arg = JS_NULL;
        } else {
            arg = JS_NewStringLen(host->ctx, utf8, len);
        }
        JSValue result = JS_Call(host->ctx, *slot, JS_UNDEFINED, 1, &arg);
        JS_FreeValue(host->ctx, arg);
        if (!JS_IsException(result)) {
            JS_FreeValue(host->ctx, result);
            continue;
        }
        if (!failed) {
            write_exception(host->ctx, err, err_len);
        } else {
            JSValue exception = JS_GetException(host->ctx);
            JS_FreeValue(host->ctx, exception);
        }
        failed = 1;
    }
    return failed ? -1 : 0;
}

#define UIFW_JOB_ROUND_LIMIT 32

int qjs_drain_jobs(Host *host, char *err, size_t err_len) {
    int rounds = 0;
    while (JS_IsJobPending(host->rt)) {
        if (rounds >= UIFW_JOB_ROUND_LIMIT) {
            if (err && err_len > 0) {
                snprintf(err, err_len, "quickjs job round limit");
            }
            return -1;
        }
        JSContext *ctx = NULL;
        int ran = JS_ExecutePendingJob(host->rt, &ctx);
        if (ran < 0) {
            write_exception(ctx ? ctx : host->ctx, err, err_len);
            return -1;
        }
        if (ran == 0) {
            return 0;
        }
        rounds++;
    }
    return 0;
}

int qjs_jobs_pending(Host *host) {
    return host && JS_IsJobPending(host->rt) ? 1 : 0;
}

int qjs_tap_count(Host *host) {
    return host ? (int)host->tap_len : 0;
}

int qjs_frame(Host *host, double time_ms, char *err, size_t err_len) {
    if (!host->has_frame) {
        return 0;
    }
    JSValue arg = JS_NewFloat64(host->ctx, time_ms);
    JSValue result = JS_Call(host->ctx, host->frame_cb, JS_UNDEFINED, 1, &arg);
    JS_FreeValue(host->ctx, arg);
    if (!JS_IsException(result)) {
        JS_FreeValue(host->ctx, result);
        return 0;
    }
    write_exception(host->ctx, err, err_len);
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

void qjs_close(Host *host) {
    if (!host) {
        return;
    }
    for (size_t index = 0; index < host->tap_len; index++) {
        free_tap(host, &host->taps[index]);
    }
    free(host->taps);
    if (host->has_frame) {
        JS_FreeValue(host->ctx, host->frame_cb);
    }
    if (host->ctx) {
        JS_FreeContext(host->ctx);
    }
    if (host->rt) {
        JS_FreeRuntime(host->rt);
    }
    free(host);
}
