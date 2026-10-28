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
    int (*detach)(void *session, uint64_t parent, uint64_t child);
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
    int (*set_ime_state)(void *session, int shown, const char *utf8, size_t len, uint32_t sel_start,
                         uint32_t sel_end, int comp_start, int comp_end, float x, float y, float w,
                         float h);
    int (*set_semantics)(void *session, uint64_t node, int role, const char *label, size_t label_len,
                         const char *value, size_t value_len, int checked, int sel_start,
                         int sel_end, int disabled, int editable, int read_only,
                         const char *description, size_t description_len, int order_set, int order,
                         int live);
    double (*text_scale)(void *session);
    int (*reduce_motion)(void *session);
    int (*measure_text)(void *session, const char *utf8, size_t len, float font_size,
                        float max_width, const uint8_t color[4], float *w, float *h);
    int (*grid)(void *session, const char *columns, size_t columns_len, const char *rows,
                size_t rows_len, float gap, uint64_t *out);
    int (*set_padding_edges)(void *session, uint64_t node, float top, float right, float bottom,
                             float left);
    int (*set_margin)(void *session, uint64_t node, float top, float right, float bottom,
                      float left);
    int (*set_min_size)(void *session, uint64_t node, float width, float height);
    int (*set_max_size)(void *session, uint64_t node, float width, float height);
    int (*set_shrink)(void *session, uint64_t node, float shrink);
    int (*set_basis)(void *session, uint64_t node, float basis);
    int (*set_align_self)(void *session, uint64_t node, uint32_t align);
    int (*set_drag_claim)(void *session, uint64_t node, int claim);
    int (*caret_at)(void *session, uint64_t node, float x, float y, uint32_t *unit, float *out_x,
                    float *out_y, float *out_w, float *out_h);
    int (*word_range)(void *session, uint64_t node, uint32_t unit, uint32_t *start, uint32_t *end);
    int (*focus_step)(void *session, int direction);
    int (*push_focus_scope)(void *session, uint64_t node);
    int (*pop_focus_scope)(void *session);
    int (*pointer_position)(void *session, float *x, float *y);
    int (*text_edit)(void *session, uint64_t node, int op, const char *utf8, size_t len,
                     uint32_t *sel_start, uint32_t *sel_end);
    const uint8_t *(*text_edit_bytes)(void *session, size_t *len);
    int (*set_text_scale)(void *session, float scale);
    int (*set_soft_wrap)(void *session, uint64_t node, int wrap);
    int (*set_overflow)(void *session, uint64_t node, int overflow);
    int (*caret_at_unit)(void *session, uint64_t node, uint32_t unit, float *x, float *y, float *w,
                         float *h);
    int (*selection_rects)(void *session, uint64_t node, uint32_t start, uint32_t end);
    const uint8_t *(*selection_rect_bytes)(void *session, size_t *len);
    int (*set_ime_config)(void *session, int keyboard, int action);
    int (*http)(void *session, uint64_t token, const char *method, size_t method_len,
                const char *url, size_t url_len, const char *headers, size_t headers_len,
                const uint8_t *body, size_t body_len);
    void (*abort_token)(void *session, uint64_t token);
    int (*module_has)(void *session, const char *name, size_t len);
    /* 0 synchronous, 1 promise, -1 not linked. */
    int (*module_prepare)(void *session, const char *name, size_t name_len, const char *method,
                          size_t method_len);
    void (*socket_open)(void *session, uint64_t id, const char *url, size_t url_len);
    void (*socket_send)(void *session, uint64_t id, const uint8_t *data, size_t len, int is_text);
    void (*socket_close)(void *session, uint64_t id);
    uint64_t (*focused)(void *session);
    int (*is_focusable)(void *session, uint64_t node);
    uint32_t (*grapheme_index)(const char *utf8, size_t len, uint32_t unit);
    uint32_t (*grapheme_count)(const char *utf8, size_t len);
    uint32_t (*grapheme_unit)(const char *utf8, size_t len, uint32_t index);
    int (*set_reverse)(void *session, uint64_t node, int reverse);
    int (*set_axis_size)(void *session, uint64_t node, int width_tag, float width, int height_tag,
                         float height);
    int (*set_aspect_ratio)(void *session, uint64_t node, int present, float ratio);
    int (*set_position)(void *session, uint64_t node, int absolute);
    int (*set_inset)(void *session, uint64_t node, int top_auto, float top, int right_auto, float right,
                     int bottom_auto, float bottom, int left_auto, float left);
    int (*set_grid_placement)(void *session, uint64_t node, int column_start_tag, int column_start,
                              int column_end_tag, int column_end, int row_start_tag, int row_start,
                              int row_end_tag, int row_end);
    int (*set_scale_xy)(void *session, uint64_t node, float scale_x, float scale_y);
    int (*register_font)(void *session, const char *family, size_t family_len, const uint8_t *bytes,
                         size_t byte_len);
    int (*set_image_mode)(void *session, uint64_t node, int mode, int required);
    int (*reload)(void *session, const char *source, size_t source_len);
    int (*push_mutation)(void *session, uint32_t op, uint64_t id, uint64_t parent, uint64_t before,
                         uint32_t field, double number, double number2, const uint8_t color[4],
                         const char *text, size_t text_len);
    int (*layer_stats)(void *session, uint64_t *epoch, uint32_t *layers, uint32_t *pictures);
    int (*scroll_into_view)(void *session, uint64_t node, int edge);
    int (*animate)(void *session, uint64_t node, const char *prop, size_t prop_len, double from,
                   double to, double stiffness, double damping);
    int (*fling_target)(void *session, uint64_t node, float *x, float *y);
    int (*set_recognizers)(void *session, uint64_t node, int tap, int long_press, int drag,
                           double double_tap_ms);
    int (*set_stroke)(void *session, uint64_t node, float width, const uint8_t color[4]);
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
    int (*module_count)(void *session, const char *name, size_t len);
    const char *(*module_method_at)(void *session, const char *name, size_t name_len, int index,
                                    size_t *out_len);
    int (*align)(void *session, uint32_t align, uint64_t *out);
    int (*set_cross)(void *session, uint64_t node, uint32_t align);
    int (*viewport)(void *session, float *width, float *height);
    int (*set_scroll_physics)(void *session, uint64_t node, float tau, int mode, float stiffness,
                              float damping, float factor, float snap_x, int align_x, float snap_y,
                              int align_y);
    int (*set_scroll_chain)(void *session, uint64_t node, int chain);
    int (*overscroll)(void *session, uint64_t node, float *x, float *y);
    int (*arm_fling)(void *session, uint64_t node, float vx, float vy);
    int (*stop_scroll)(void *session, uint64_t node);
    int (*scroll_to)(void *session, uint64_t node, float x, float y, float stiffness, float damping);
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
} Host;

#define UIFW_JS_MEMORY_LIMIT (16u << 20)
/* Debug quickjs-ng takes about 44 ms to eval dist/counter.js. Mounting the
   gallery list does the same kind of host calls and also exceeds one frame.
   Crossing the limit throws InternalError: interrupted and discards the turn,
   so this is a runaway cutoff, not a frame budget. */
#define UIFW_JS_TURN_MS 128

static int64_t now_ms(void) {
    struct timespec ts;
    if (clock_gettime(CLOCK_MONOTONIC, &ts) != 0) {
        return 0;
    }
    return (int64_t)ts.tv_sec * 1000 + (int64_t)ts.tv_nsec / 1000000;
}

static void arm_slice(Host *host) {
    host->deadline_ms = now_ms() + UIFW_JS_TURN_MS;
}

static void arm_eval(Host *host) {
    host->deadline_ms = now_ms() + UIFW_JS_TURN_MS;
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

/* `report` logs and keeps dispatching. `freeze` fails the call so the frame holds. */
static void listener_failed(Host *host, char *err, size_t err_len, int *failed) {
    if (!host->listener_freeze) {
        char local[512];
        local[0] = 0;
        write_exception(host->ctx, local, sizeof(local));
        if (host->fns.log) {
            host->fns.log(host->fns.session, local, strlen(local), 3);
        }
        return;
    }
    if (!*failed) {
        write_exception(host->ctx, err, err_len);
    } else {
        JSValue exception = JS_GetException(host->ctx);
        JS_FreeValue(host->ctx, exception);
    }
    *failed = 1;
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
    return JS_UNDEFINED;
}

static JSValue js_detach(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 2) {
        return throw_msg(ctx, "detach(parent, child)");
    }
    uint64_t parent = 0;
    uint64_t child = 0;
    if (read_handle(ctx, argv[0], &parent) || read_handle(ctx, argv[1], &child)) {
        return JS_EXCEPTION;
    }
    if (host->fns.detach(host->fns.session, parent, child)) {
        return throw_msg(ctx, "detach failed");
    }
    return JS_UNDEFINED;
}

static JSValue js_discard(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 1) {
        return throw_msg(ctx, "discard(node)");
    }
    uint64_t node = 0;
    if (read_handle(ctx, argv[0], &node)) {
        return JS_EXCEPTION;
    }
    uint64_t *ids = NULL;
    size_t len = 0;
    if (host->fns.discard(host->fns.session, node, &ids, &len)) {
        return throw_msg(ctx, "discard failed");
    }
    forget_taps(host, ids, len);
    host->fns.free_ids(ids, len);
    return JS_UNDEFINED;
}

void qjs_forget_ids(Host *host, const uint64_t *ids, size_t len) {
    if (!host || !ids) {
        return;
    }
    forget_taps(host, ids, len);
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
    if (status != 0) {
        return created(ctx, status, id, "image failed");
    }
    if (argc >= 4) {
        int mode = 0;
        int mode_status = image_mode_tag(ctx, argv[3], &mode);
        if (mode_status > 0) {
            return JS_EXCEPTION;
        }
        if (mode_status < 0) {
            return throw_msg(ctx, "bad image");
        }
        int required = 0;
        if (argc >= 5 && !JS_IsUndefined(argv[4]) && !JS_IsNull(argv[4])) {
            required = JS_ToBool(ctx, argv[4]);
            if (required < 0) {
                return JS_EXCEPTION;
            }
        }
        status = host->fns.set_image_mode(host->fns.session, id, mode, required);
        if (status != 0) {
            return throw_msg(ctx, "bad image");
        }
    }
    return created(ctx, 0, id, "image failed");
}

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

static JSValue js_set_drag_claim(JSContext *ctx, JSValueConst this_val, int argc,
                                 JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 2) {
        return throw_msg(ctx, "setDragClaim(node, claim)");
    }
    uint64_t node = 0;
    const char *name = JS_ToCString(ctx, argv[1]);
    if (read_handle(ctx, argv[0], &node) || !name) {
        return JS_EXCEPTION;
    }
    int claim = 0;
    if (strcmp(name, "horizontal") == 0) {
        claim = 1;
    } else if (strcmp(name, "free") == 0) {
        claim = 2;
    } else if (strcmp(name, "none") != 0) {
        JS_FreeCString(ctx, name);
        return throw_msg(ctx, "drag claim");
    }
    JS_FreeCString(ctx, name);
    return done(ctx, host->fns.set_drag_claim(host->fns.session, node, claim), "setDragClaim failed");
}

static JSValue js_caret_at(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 3) {
        return throw_msg(ctx, "caretAt(node, x, y)");
    }
    uint64_t node = 0;
    double x = 0;
    double y = 0;
    if (read_handle(ctx, argv[0], &node) || JS_ToFloat64(ctx, &x, argv[1]) ||
        JS_ToFloat64(ctx, &y, argv[2])) {
        return JS_EXCEPTION;
    }
    uint32_t unit = 0;
    float cx = 0;
    float cy = 0;
    float cw = 0;
    float ch = 0;
    if (host->fns.caret_at(host->fns.session, node, (float)x, (float)y, &unit, &cx, &cy, &cw,
                           &ch)) {
        return throw_msg(ctx, "caretAt failed");
    }
    JSValue result = JS_NewObject(ctx);
    JS_SetPropertyStr(ctx, result, "unit", JS_NewUint32(ctx, unit));
    JS_SetPropertyStr(ctx, result, "x", JS_NewFloat64(ctx, cx));
    JS_SetPropertyStr(ctx, result, "y", JS_NewFloat64(ctx, cy));
    JS_SetPropertyStr(ctx, result, "w", JS_NewFloat64(ctx, cw));
    JS_SetPropertyStr(ctx, result, "h", JS_NewFloat64(ctx, ch));
    return result;
}

static JSValue js_word_range(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 2) {
        return throw_msg(ctx, "wordRange(node, unit)");
    }
    uint64_t node = 0;
    uint32_t unit = 0;
    if (read_handle(ctx, argv[0], &node) || JS_ToUint32(ctx, &unit, argv[1])) {
        return JS_EXCEPTION;
    }
    uint32_t start = 0;
    uint32_t end = 0;
    if (host->fns.word_range(host->fns.session, node, unit, &start, &end)) {
        return throw_msg(ctx, "wordRange failed");
    }
    JSValue result = JS_NewObject(ctx);
    JS_SetPropertyStr(ctx, result, "start", JS_NewUint32(ctx, start));
    JS_SetPropertyStr(ctx, result, "end", JS_NewUint32(ctx, end));
    return result;
}

static JSValue js_focus_step(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 1) {
        return throw_msg(ctx, "focusStep(direction)");
    }
    const char *name = JS_ToCString(ctx, argv[0]);
    if (!name) {
        return JS_EXCEPTION;
    }
    int direction = focus_direction(name);
    JS_FreeCString(ctx, name);
    if (direction < 0) {
        return throw_msg(ctx, "focus direction");
    }
    return done(ctx, host->fns.focus_step(host->fns.session, direction), "focusStep failed");
}

static JSValue js_push_focus_scope(JSContext *ctx, JSValueConst this_val, int argc,
                                   JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 1) {
        return throw_msg(ctx, "pushFocusScope(node)");
    }
    uint64_t node = 0;
    if (read_handle(ctx, argv[0], &node)) {
        return JS_EXCEPTION;
    }
    return done(ctx, host->fns.push_focus_scope(host->fns.session, node), "pushFocusScope failed");
}

static JSValue js_pop_focus_scope(JSContext *ctx, JSValueConst this_val, int argc,
                                  JSValueConst *argv) {
    (void)this_val;
    (void)argc;
    (void)argv;
    Host *host = host_of(ctx);
    return done(ctx, host->fns.pop_focus_scope(host->fns.session), "popFocusScope failed");
}

static JSValue js_pointer_position(JSContext *ctx, JSValueConst this_val, int argc,
                                   JSValueConst *argv) {
    (void)this_val;
    (void)argc;
    (void)argv;
    Host *host = host_of(ctx);
    float x = 0;
    float y = 0;
    if (host->fns.pointer_position(host->fns.session, &x, &y)) {
        return throw_msg(ctx, "pointerPosition failed");
    }
    JSValue result = JS_NewObject(ctx);
    JS_SetPropertyStr(ctx, result, "x", JS_NewFloat64(ctx, x));
    JS_SetPropertyStr(ctx, result, "y", JS_NewFloat64(ctx, y));
    return result;
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

static JSValue js_focused(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    (void)argc;
    (void)argv;
    Host *host = host_of(ctx);
    return handle_value(ctx, host->fns.focused(host->fns.session));
}

static JSValue js_is_focusable(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 1) {
        return throw_msg(ctx, "isFocusable(node)");
    }
    uint64_t node = 0;
    if (read_handle(ctx, argv[0], &node)) {
        return JS_EXCEPTION;
    }
    int status = host->fns.is_focusable(host->fns.session, node);
    if (status < 0) {
        return throw_msg(ctx, "isFocusable failed");
    }
    return JS_NewBool(ctx, status);
}

static JSValue js_grapheme_index(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 2) {
        return throw_msg(ctx, "graphemeIndex(text, unit)");
    }
    size_t len = 0;
    const char *text = JS_ToCStringLen(ctx, &len, argv[0]);
    if (!text) {
        return JS_EXCEPTION;
    }
    uint32_t unit = 0;
    if (JS_ToUint32(ctx, &unit, argv[1])) {
        JS_FreeCString(ctx, text);
        return JS_EXCEPTION;
    }
    uint32_t index = host->fns.grapheme_index(text, len, unit);
    JS_FreeCString(ctx, text);
    return JS_NewUint32(ctx, index);
}

static JSValue js_grapheme_count(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 1) {
        return throw_msg(ctx, "graphemeCount(text)");
    }
    size_t len = 0;
    const char *text = JS_ToCStringLen(ctx, &len, argv[0]);
    if (!text) {
        return JS_EXCEPTION;
    }
    uint32_t count = host->fns.grapheme_count(text, len);
    JS_FreeCString(ctx, text);
    return JS_NewUint32(ctx, count);
}

static JSValue js_grapheme_unit(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 2) {
        return throw_msg(ctx, "graphemeUnit(text, index)");
    }
    size_t len = 0;
    const char *text = JS_ToCStringLen(ctx, &len, argv[0]);
    if (!text) {
        return JS_EXCEPTION;
    }
    uint32_t index = 0;
    if (JS_ToUint32(ctx, &index, argv[1])) {
        JS_FreeCString(ctx, text);
        return JS_EXCEPTION;
    }
    uint32_t unit = host->fns.grapheme_unit(text, len, index);
    JS_FreeCString(ctx, text);
    return JS_NewUint32(ctx, unit);
}

static JSValue js_set_reverse(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 2) {
        return throw_msg(ctx, "setReverse(node, reverse)");
    }
    uint64_t node = 0;
    if (read_handle(ctx, argv[0], &node)) {
        return JS_EXCEPTION;
    }
    int reverse = JS_ToBool(ctx, argv[1]);
    if (reverse < 0) {
        return JS_EXCEPTION;
    }
    return done(ctx, host->fns.set_reverse(host->fns.session, node, reverse), "setReverse failed");
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

static JSValue js_set_axis_size(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 3) {
        return throw_msg(ctx, "setAxisSize(node, width, height)");
    }
    uint64_t node = 0;
    int width_tag = 0;
    int height_tag = 0;
    float width = 0;
    float height = 0;
    if (read_handle(ctx, argv[0], &node) || read_axis(ctx, argv[1], &width_tag, &width) ||
        read_axis(ctx, argv[2], &height_tag, &height)) {
        return JS_EXCEPTION;
    }
    return done(ctx,
                host->fns.set_axis_size(host->fns.session, node, width_tag, width, height_tag, height),
                "setAxisSize failed");
}

static JSValue js_set_aspect_ratio(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 2) {
        return throw_msg(ctx, "setAspectRatio(node, ratio)");
    }
    uint64_t node = 0;
    if (read_handle(ctx, argv[0], &node)) {
        return JS_EXCEPTION;
    }
    int present = 1;
    float ratio = 0;
    if (JS_IsNull(argv[1]) || JS_IsUndefined(argv[1])) {
        present = 0;
    } else {
        double parsed = 0;
        if (JS_ToFloat64(ctx, &parsed, argv[1]) || !isfinite(parsed) || parsed <= 0.0) {
            return throw_msg(ctx, "bad aspect");
        }
        ratio = (float)parsed;
    }
    return done(ctx, host->fns.set_aspect_ratio(host->fns.session, node, present, ratio),
                "setAspectRatio failed");
}

static JSValue js_set_position(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 2) {
        return throw_msg(ctx, "setPosition(node, absolute)");
    }
    uint64_t node = 0;
    if (read_handle(ctx, argv[0], &node)) {
        return JS_EXCEPTION;
    }
    int absolute = JS_ToBool(ctx, argv[1]);
    if (absolute < 0) {
        return JS_EXCEPTION;
    }
    return done(ctx, host->fns.set_position(host->fns.session, node, absolute), "setPosition failed");
}

static JSValue js_set_inset(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 5) {
        return throw_msg(ctx, "setInset(node, top, right, bottom, left)");
    }
    uint64_t node = 0;
    int top_auto = 0;
    int right_auto = 0;
    int bottom_auto = 0;
    int left_auto = 0;
    float top = 0;
    float right = 0;
    float bottom = 0;
    float left = 0;
    if (read_handle(ctx, argv[0], &node) || read_inset_edge(ctx, argv[1], &top_auto, &top) ||
        read_inset_edge(ctx, argv[2], &right_auto, &right) ||
        read_inset_edge(ctx, argv[3], &bottom_auto, &bottom) ||
        read_inset_edge(ctx, argv[4], &left_auto, &left)) {
        return JS_EXCEPTION;
    }
    return done(ctx,
                host->fns.set_inset(host->fns.session, node, top_auto, top, right_auto, right, bottom_auto,
                                    bottom, left_auto, left),
                "setInset failed");
}

static JSValue js_set_grid_placement(JSContext *ctx, JSValueConst this_val, int argc,
                                     JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 5) {
        return throw_msg(ctx, "setGridPlacement(node, columnStart, columnEnd, rowStart, rowEnd)");
    }
    uint64_t node = 0;
    int tags[4] = {0, 0, 0, 0};
    int values[4] = {0, 0, 0, 0};
    if (read_handle(ctx, argv[0], &node)) {
        return JS_EXCEPTION;
    }
    for (int i = 0; i < 4; i++) {
        if (read_grid_line(ctx, argv[i + 1], &tags[i], &values[i])) {
            return JS_EXCEPTION;
        }
    }
    return done(ctx,
                host->fns.set_grid_placement(host->fns.session, node, tags[0], values[0], tags[1], values[1],
                                             tags[2], values[2], tags[3], values[3]),
                "setGridPlacement failed");
}

static JSValue js_set_scale_xy(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 3) {
        return throw_msg(ctx, "setScaleXY(node, scaleX, scaleY)");
    }
    uint64_t node = 0;
    double scale_x = 0;
    double scale_y = 0;
    if (read_handle(ctx, argv[0], &node) || JS_ToFloat64(ctx, &scale_x, argv[1]) ||
        JS_ToFloat64(ctx, &scale_y, argv[2])) {
        return JS_EXCEPTION;
    }
    return done(ctx, host->fns.set_scale_xy(host->fns.session, node, (float)scale_x, (float)scale_y),
                "setScaleXY failed");
}

static JSValue js_register_font(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 2) {
        return throw_msg(ctx, "registerFont(family, bytes)");
    }
    size_t family_len = 0;
    const char *family = JS_ToCStringLen(ctx, &family_len, argv[0]);
    if (!family) {
        return JS_EXCEPTION;
    }
    size_t offset = 0;
    size_t byte_len = 0;
    JSValue buffer = JS_GetTypedArrayBuffer(ctx, argv[1], &offset, &byte_len, NULL);
    if (JS_IsException(buffer)) {
        JS_FreeCString(ctx, family);
        return JS_EXCEPTION;
    }
    size_t buffer_len = 0;
    uint8_t *data = JS_GetArrayBuffer(ctx, &buffer_len, buffer);
    JS_FreeValue(ctx, buffer);
    if (!data || offset > buffer_len || byte_len > buffer_len - offset) {
        JS_FreeCString(ctx, family);
        return throw_msg(ctx, "registerFont failed");
    }
    int status = host->fns.register_font(host->fns.session, family, family_len, data + offset, byte_len);
    JS_FreeCString(ctx, family);
    if (status != 0) {
        return throw_msg(ctx, "registerFont failed");
    }
    return JS_UNDEFINED;
}

static JSValue js_reload(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 1) {
        return throw_msg(ctx, "reload(source)");
    }
    size_t len = 0;
    const char *text = JS_ToCStringLen(ctx, &len, argv[0]);
    if (!text) {
        return JS_EXCEPTION;
    }
    if (len == 0) {
        JS_FreeCString(ctx, text);
        return throw_msg(ctx, "reload(source)");
    }
    int status = host->fns.reload(host->fns.session, text, len);
    JS_FreeCString(ctx, text);
    return done(ctx, status, "reload failed");
}

static JSValue js_set_image_mode(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 3) {
        return throw_msg(ctx, "setImageMode(node, mode, required)");
    }
    uint64_t node = 0;
    if (read_handle(ctx, argv[0], &node)) {
        return JS_EXCEPTION;
    }
    int mode = 0;
    int mode_status = image_mode_tag(ctx, argv[1], &mode);
    if (mode_status > 0) {
        return JS_EXCEPTION;
    }
    if (mode_status < 0) {
        return throw_msg(ctx, "bad image");
    }
    int required = JS_ToBool(ctx, argv[2]);
    if (required < 0) {
        return JS_EXCEPTION;
    }
    return done(ctx, host->fns.set_image_mode(host->fns.session, node, mode, required), "bad image");
}

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

static JSValue js_layer_tree_stats(JSContext *ctx, JSValueConst this_val, int argc,
                                   JSValueConst *argv) {
    (void)this_val;
    (void)argc;
    (void)argv;
    Host *host = host_of(ctx);
    uint64_t epoch = 0;
    uint32_t layers = 0;
    uint32_t pictures = 0;
    if (!host->fns.layer_stats ||
        host->fns.layer_stats(host->fns.session, &epoch, &layers, &pictures)) {
        return throw_msg(ctx, "layerTreeStats");
    }
    JSValue object = JS_NewObject(ctx);
    JS_SetPropertyStr(ctx, object, "epoch", JS_NewFloat64(ctx, (double)epoch));
    JS_SetPropertyStr(ctx, object, "layers", JS_NewUint32(ctx, layers));
    JS_SetPropertyStr(ctx, object, "pictures", JS_NewUint32(ctx, pictures));
    return object;
}

static JSValue js_scroll_into_view(JSContext *ctx, JSValueConst this_val, int argc,
                                   JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 2) {
        return throw_msg(ctx, "scrollIntoView(node, edge)");
    }
    uint64_t node = 0;
    int32_t edge = 0;
    if (read_handle(ctx, argv[0], &node) || JS_ToInt32(ctx, &edge, argv[1])) {
        return JS_EXCEPTION;
    }
    return done(ctx, host->fns.scroll_into_view(host->fns.session, node, edge), "scrollIntoView");
}

static JSValue js_animate(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 6) {
        return throw_msg(ctx, "animate(node, prop, from, to, stiffness, damping)");
    }
    uint64_t node = 0;
    size_t prop_len = 0;
    double from = 0;
    double to = 0;
    double stiffness = 0;
    double damping = 0;
    if (read_handle(ctx, argv[0], &node)) {
        return JS_EXCEPTION;
    }
    const char *prop = JS_ToCStringLen(ctx, &prop_len, argv[1]);
    if (!prop || JS_ToFloat64(ctx, &from, argv[2]) || JS_ToFloat64(ctx, &to, argv[3]) ||
        JS_ToFloat64(ctx, &stiffness, argv[4]) || JS_ToFloat64(ctx, &damping, argv[5])) {
        if (prop) {
            JS_FreeCString(ctx, prop);
        }
        return prop ? JS_EXCEPTION : throw_msg(ctx, "animate(node, prop, from, to, stiffness, damping)");
    }
    int status = host->fns.animate(host->fns.session, node, prop, prop_len, from, to, stiffness,
                                   damping);
    JS_FreeCString(ctx, prop);
    return done(ctx, status, "animate");
}

static JSValue js_fling_target(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 1) {
        return throw_msg(ctx, "flingTarget(node)");
    }
    uint64_t node = 0;
    float x = 0;
    float y = 0;
    if (read_handle(ctx, argv[0], &node)) {
        return JS_EXCEPTION;
    }
    int status = host->fns.fling_target(host->fns.session, node, &x, &y);
    if (status > 0) {
        return JS_NULL;
    }
    if (status < 0) {
        return throw_msg(ctx, "flingTarget");
    }
    JSValue object = JS_NewObject(ctx);
    JS_SetPropertyStr(ctx, object, "x", JS_NewFloat64(ctx, x));
    JS_SetPropertyStr(ctx, object, "y", JS_NewFloat64(ctx, y));
    return object;
}

static JSValue js_set_recognizers(JSContext *ctx, JSValueConst this_val, int argc,
                                  JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 2 || !JS_IsObject(argv[1])) {
        return throw_msg(ctx, "setRecognizers(node, spec)");
    }
    uint64_t node = 0;
    if (read_handle(ctx, argv[0], &node)) {
        return JS_EXCEPTION;
    }
    int tap = 0;
    int long_press = 0;
    int drag = -1;
    double window_ms = 0;
    JSValue tap_value = JS_GetPropertyStr(ctx, argv[1], "tap");
    if (!JS_IsUndefined(tap_value) && !JS_IsNull(tap_value)) {
        tap = JS_ToBool(ctx, tap_value) == 1;
    }
    JS_FreeValue(ctx, tap_value);
    JSValue long_value = JS_GetPropertyStr(ctx, argv[1], "longPress");
    if (!JS_IsUndefined(long_value) && !JS_IsNull(long_value)) {
        long_press = JS_ToBool(ctx, long_value) == 1;
    }
    JS_FreeValue(ctx, long_value);
    JSValue drag_value = JS_GetPropertyStr(ctx, argv[1], "drag");
    if (!JS_IsUndefined(drag_value) && !JS_IsNull(drag_value)) {
        const char *text = JS_ToCString(ctx, drag_value);
        if (text) {
            if (strcmp(text, "free") == 0) {
                drag = 0;
            } else if (strcmp(text, "horizontal") == 0) {
                drag = 1;
            } else if (strcmp(text, "vertical") == 0) {
                drag = 2;
            }
            JS_FreeCString(ctx, text);
        }
    }
    JS_FreeValue(ctx, drag_value);
    JSValue window_value = JS_GetPropertyStr(ctx, argv[1], "doubleTapMs");
    if (!JS_IsUndefined(window_value) && !JS_IsNull(window_value)) {
        if (JS_ToFloat64(ctx, &window_ms, window_value)) {
            JS_FreeValue(ctx, window_value);
            return JS_EXCEPTION;
        }
    }
    JS_FreeValue(ctx, window_value);
    return done(ctx,
                host->fns.set_recognizers(host->fns.session, node, tap, long_press, drag, window_ms),
                "setRecognizers");
}

static JSValue js_set_stroke(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 3) {
        return throw_msg(ctx, "setStroke(node, width, color)");
    }
    uint64_t node = 0;
    double width = 0;
    uint8_t color[4];
    if (read_handle(ctx, argv[0], &node) || JS_ToFloat64(ctx, &width, argv[1]) ||
        read_color(ctx, argv[2], color)) {
        return JS_EXCEPTION;
    }
    return done(ctx, host->fns.set_stroke(host->fns.session, node, (float)width, color),
                "setStroke");
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

static JSValue js_set_ime_state(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 1) {
        return throw_msg(ctx, "setImeState");
    }
    if (JS_IsNull(argv[0]) || JS_IsUndefined(argv[0])) {
        return done(ctx, host->fns.set_ime_state(host->fns.session, 0, "", 0, 0, 0, -1, -1, 0, 0, 0, 0),
                    "setImeState failed");
    }
    if (argc < 9) {
        return throw_msg(ctx, "setImeState(text, selection, composing, caret)");
    }
    size_t len = 0;
    const char *utf8 = JS_ToCStringLen(ctx, &len, argv[0]);
    if (!utf8) {
        return JS_EXCEPTION;
    }
    uint32_t sel_start = 0;
    uint32_t sel_end = 0;
    int32_t comp_start = -1;
    int32_t comp_end = -1;
    double x = 0;
    double y = 0;
    double w = 0;
    double h = 0;
    int bad = JS_ToUint32(ctx, &sel_start, argv[1]) || JS_ToUint32(ctx, &sel_end, argv[2]) ||
              JS_ToInt32(ctx, &comp_start, argv[3]) || JS_ToInt32(ctx, &comp_end, argv[4]) ||
              JS_ToFloat64(ctx, &x, argv[5]) || JS_ToFloat64(ctx, &y, argv[6]) ||
              JS_ToFloat64(ctx, &w, argv[7]) || JS_ToFloat64(ctx, &h, argv[8]);
    if (bad) {
        JS_FreeCString(ctx, utf8);
        return JS_EXCEPTION;
    }
    int status = host->fns.set_ime_state(host->fns.session, 1, utf8, len, sel_start, sel_end,
                                         comp_start, comp_end, (float)x, (float)y, (float)w,
                                         (float)h);
    JS_FreeCString(ctx, utf8);
    return done(ctx, status, "setImeState failed");
}

static JSValue js_set_semantics(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 2) {
        return throw_msg(ctx, "setSemantics(node, role)");
    }
    uint64_t node = 0;
    int32_t role = 0;
    int32_t checked = -1;
    int32_t sel_start = -1;
    int32_t sel_end = -1;
    int32_t disabled = -1;
    int32_t editable = -1;
    int32_t read_only = -1;
    int status = 0;
    if (read_handle(ctx, argv[0], &node) || JS_ToInt32(ctx, &role, argv[1])) {
        return JS_EXCEPTION;
    }
    const char *label = "";
    size_t label_len = 0;
    const char *value = "";
    size_t value_len = 0;
    const char *description = "";
    size_t description_len = 0;
    if (argc > 2 && !JS_IsUndefined(argv[2]) && !JS_IsNull(argv[2])) {
        label = JS_ToCStringLen(ctx, &label_len, argv[2]);
        if (!label) {
            return JS_EXCEPTION;
        }
    }
    if (argc > 3 && !JS_IsUndefined(argv[3]) && !JS_IsNull(argv[3])) {
        value = JS_ToCStringLen(ctx, &value_len, argv[3]);
        if (!value) {
            if (label_len) {
                JS_FreeCString(ctx, label);
            }
            return JS_EXCEPTION;
        }
    }
    if (argc > 4 && !JS_IsUndefined(argv[4]) && JS_ToInt32(ctx, &checked, argv[4])) {
        if (label_len) {
            JS_FreeCString(ctx, label);
        }
        if (value_len) {
            JS_FreeCString(ctx, value);
        }
        return JS_EXCEPTION;
    }
    if (argc > 6) {
        if (JS_ToInt32(ctx, &sel_start, argv[5]) || JS_ToInt32(ctx, &sel_end, argv[6])) {
            if (label_len) {
                JS_FreeCString(ctx, label);
            }
            if (value_len) {
                JS_FreeCString(ctx, value);
            }
            return JS_EXCEPTION;
        }
    }
    if (argc > 7 && !JS_IsUndefined(argv[7]) && JS_ToInt32(ctx, &disabled, argv[7])) {
        goto fail_strings;
    }
    if (argc > 8 && !JS_IsUndefined(argv[8]) && JS_ToInt32(ctx, &editable, argv[8])) {
        goto fail_strings;
    }
    if (argc > 9 && !JS_IsUndefined(argv[9]) && JS_ToInt32(ctx, &read_only, argv[9])) {
        goto fail_strings;
    }
    if (argc > 10 && !JS_IsUndefined(argv[10]) && !JS_IsNull(argv[10])) {
        description = JS_ToCStringLen(ctx, &description_len, argv[10]);
        if (!description) {
            goto fail_strings;
        }
    }
    int order_set = 0;
    int32_t order = 0;
    int32_t live = -1;
    if (argc > 11 && JS_IsObject(argv[11])) {
        JSValue order_value = JS_GetPropertyStr(ctx, argv[11], "order");
        if (!JS_IsUndefined(order_value) && !JS_IsNull(order_value)) {
            if (JS_ToInt32(ctx, &order, order_value)) {
                JS_FreeValue(ctx, order_value);
                goto fail_strings;
            }
            order_set = 1;
        }
        JS_FreeValue(ctx, order_value);
        JSValue live_value = JS_GetPropertyStr(ctx, argv[11], "live");
        if (!JS_IsUndefined(live_value) && !JS_IsNull(live_value)) {
            const char *text = JS_ToCString(ctx, live_value);
            if (!text) {
                JS_FreeValue(ctx, live_value);
                goto fail_strings;
            }
            if (strcmp(text, "off") == 0) {
                live = 0;
            } else if (strcmp(text, "polite") == 0) {
                live = 1;
            } else if (strcmp(text, "assertive") == 0) {
                live = 2;
            }
            JS_FreeCString(ctx, text);
        }
        JS_FreeValue(ctx, live_value);
    }
    status = host->fns.set_semantics(
        host->fns.session, node, role, label, label_len, value, value_len, checked, sel_start,
        sel_end, disabled, editable, read_only, description, description_len, order_set, order,
        live);
    if (label_len) {
        JS_FreeCString(ctx, label);
    }
    if (value_len) {
        JS_FreeCString(ctx, value);
    }
    if (description_len) {
        JS_FreeCString(ctx, description);
    }
    return done(ctx, status, "setSemantics failed");
fail_strings:
    if (label_len) {
        JS_FreeCString(ctx, label);
    }
    if (value_len) {
        JS_FreeCString(ctx, value);
    }
    return JS_EXCEPTION;
}

static JSValue js_text_scale(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    (void)argc;
    (void)argv;
    Host *host = host_of(ctx);
    return JS_NewFloat64(ctx, host->fns.text_scale(host->fns.session));
}

static JSValue js_reduce_motion(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    (void)argc;
    (void)argv;
    Host *host = host_of(ctx);
    return JS_NewBool(ctx, host->fns.reduce_motion(host->fns.session));
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

static JSValue js_align(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 1) {
        return throw_msg(ctx, "align(align)");
    }
    const char *name = JS_ToCString(ctx, argv[0]);
    if (!name) {
        return JS_EXCEPTION;
    }
    uint32_t align = 0;
    int bad = named_align(name, &align, "start", 0, "center", 1, "end", 2);
    JS_FreeCString(ctx, name);
    if (bad) {
        return throw_msg(ctx, "align align");
    }
    uint64_t id = 0;
    int status = host->fns.align(host->fns.session, align, &id);
    return created(ctx, status, id, "align failed");
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

static JSValue js_set_cross(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 2) {
        return throw_msg(ctx, "setCross(node, align)");
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
    int bad = named_cross(name, &align);
    JS_FreeCString(ctx, name);
    if (bad) {
        return throw_msg(ctx, "setCross align");
    }
    int status = host->fns.set_cross(host->fns.session, node, align);
    return done(ctx, status, "setCross failed");
}

static JSValue js_metrics(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    (void)argc;
    (void)argv;
    Host *host = host_of(ctx);
    float width = 0;
    float height = 0;
    if (host->fns.viewport(host->fns.session, &width, &height)) {
        return throw_msg(ctx, "metrics failed");
    }
    JSValue object = JS_NewObject(ctx);
    JSValue insets = JS_NewObject(ctx);
    JS_SetPropertyStr(ctx, insets, "top", JS_NewFloat64(ctx, 0));
    JS_SetPropertyStr(ctx, insets, "right", JS_NewFloat64(ctx, 0));
    JS_SetPropertyStr(ctx, insets, "bottom", JS_NewFloat64(ctx, 0));
    JS_SetPropertyStr(ctx, insets, "left", JS_NewFloat64(ctx, 0));
    JS_SetPropertyStr(ctx, object, "width", JS_NewFloat64(ctx, width));
    JS_SetPropertyStr(ctx, object, "height", JS_NewFloat64(ctx, height));
    JS_SetPropertyStr(ctx, object, "viewInsets", insets);
    return object;
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

static JSValue js_set_scroll_physics(JSContext *ctx, JSValueConst this_val, int argc,
                                     JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 10) {
        return throw_msg(ctx,
                         "setScrollPhysics(node, tau, mode, stiffness, damping, factor, snapX, "
                         "alignX, snapY, alignY)");
    }
    uint64_t node = 0;
    double tau = 0;
    int32_t mode = 0;
    double stiffness = 0;
    double damping = 0;
    double factor = 0;
    double snap_x = 0;
    int32_t align_x = 0;
    double snap_y = 0;
    int32_t align_y = 0;
    if (read_handle(ctx, argv[0], &node) || JS_ToFloat64(ctx, &tau, argv[1]) ||
        JS_ToInt32(ctx, &mode, argv[2]) || JS_ToFloat64(ctx, &stiffness, argv[3]) ||
        JS_ToFloat64(ctx, &damping, argv[4]) || JS_ToFloat64(ctx, &factor, argv[5]) ||
        JS_ToFloat64(ctx, &snap_x, argv[6]) || JS_ToInt32(ctx, &align_x, argv[7]) ||
        JS_ToFloat64(ctx, &snap_y, argv[8]) || JS_ToInt32(ctx, &align_y, argv[9])) {
        return JS_EXCEPTION;
    }
    int status = host->fns.set_scroll_physics(
        host->fns.session, node, (float)tau, mode, (float)stiffness, (float)damping, (float)factor,
        (float)snap_x, align_x, (float)snap_y, align_y);
    return done(ctx, status, "setScrollPhysics failed");
}

static JSValue js_set_scroll_chain(JSContext *ctx, JSValueConst this_val, int argc,
                                   JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 2) {
        return throw_msg(ctx, "setScrollChain(node, chain)");
    }
    uint64_t node = 0;
    int chain = JS_ToBool(ctx, argv[1]);
    if (read_handle(ctx, argv[0], &node) || chain < 0) {
        return JS_EXCEPTION;
    }
    int status = host->fns.set_scroll_chain(host->fns.session, node, chain);
    return done(ctx, status, "setScrollChain failed");
}

static JSValue js_overscroll(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 1) {
        return throw_msg(ctx, "overscroll(node)");
    }
    uint64_t node = 0;
    float x = 0;
    float y = 0;
    if (read_handle(ctx, argv[0], &node)) {
        return JS_EXCEPTION;
    }
    int status = host->fns.overscroll(host->fns.session, node, &x, &y);
    if (status) {
        return throw_msg(ctx, "overscroll failed");
    }
    JSValue result = JS_NewObject(ctx);
    JS_SetPropertyStr(ctx, result, "x", JS_NewFloat64(ctx, x));
    JS_SetPropertyStr(ctx, result, "y", JS_NewFloat64(ctx, y));
    return result;
}

static JSValue js_arm_fling(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 3) {
        return throw_msg(ctx, "fling(node, vx, vy)");
    }
    uint64_t node = 0;
    double vx = 0;
    double vy = 0;
    if (read_handle(ctx, argv[0], &node) || JS_ToFloat64(ctx, &vx, argv[1]) ||
        JS_ToFloat64(ctx, &vy, argv[2])) {
        return JS_EXCEPTION;
    }
    int status = host->fns.arm_fling(host->fns.session, node, (float)vx, (float)vy);
    return done(ctx, status, "fling failed");
}

static JSValue js_stop_scroll(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 1) {
        return throw_msg(ctx, "stopScroll(node)");
    }
    uint64_t node = 0;
    if (read_handle(ctx, argv[0], &node)) {
        return JS_EXCEPTION;
    }
    int status = host->fns.stop_scroll(host->fns.session, node);
    return done(ctx, status, "stopScroll failed");
}

static JSValue js_scroll_to(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 5) {
        return throw_msg(ctx, "scrollTo(node, x, y, stiffness, damping)");
    }
    uint64_t node = 0;
    double x = 0;
    double y = 0;
    double stiffness = 0;
    double damping = 0;
    if (read_handle(ctx, argv[0], &node) || JS_ToFloat64(ctx, &x, argv[1]) ||
        JS_ToFloat64(ctx, &y, argv[2]) || JS_ToFloat64(ctx, &stiffness, argv[3]) ||
        JS_ToFloat64(ctx, &damping, argv[4])) {
        return JS_EXCEPTION;
    }
    int status = host->fns.scroll_to(host->fns.session, node, (float)x, (float)y, (float)stiffness,
                                    (float)damping);
    return done(ctx, status, "scrollTo failed");
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

static JSValue js_grid(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 3) {
        return throw_msg(ctx, "grid(columns, rows, gap)");
    }
    char *columns = NULL;
    char *rows = NULL;
    size_t columns_len = 0;
    size_t rows_len = 0;
    double gap = 0;
    int columns_status = join_array(ctx, argv[0], &columns, &columns_len);
    int rows_status = columns_status == 0 ? join_array(ctx, argv[1], &rows, &rows_len) : 0;
    int gap_status = columns_status == 0 && rows_status == 0 ? JS_ToFloat64(ctx, &gap, argv[2]) : 0;
    if (columns_status < 0 || rows_status < 0 || gap_status) {
        free(columns);
        free(rows);
        return JS_EXCEPTION;
    }
    if (columns_status > 0 || rows_status > 0) {
        free(columns);
        free(rows);
        return throw_msg(ctx, "grid(columns, rows, gap)");
    }
    uint64_t id = 0;
    int status = host->fns.grid(host->fns.session, columns, columns_len, rows, rows_len, (float)gap,
                                &id);
    free(columns);
    free(rows);
    return created(ctx, status, id, "grid failed");
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

static JSValue js_set_padding_edges(JSContext *ctx, JSValueConst this_val, int argc,
                                    JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    uint64_t node = 0;
    float top = 0, right = 0, bottom = 0, left = 0;
    if (argc < 5) {
        return throw_msg(ctx, "setPaddingEdges(node, top, right, bottom, left)");
    }
    if (read_edges(ctx, argc, argv, &node, &top, &right, &bottom, &left)) {
        return JS_EXCEPTION;
    }
    int status = host->fns.set_padding_edges(host->fns.session, node, top, right, bottom, left);
    return done(ctx, status, "setPaddingEdges failed");
}

static JSValue js_set_margin(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    uint64_t node = 0;
    float top = 0, right = 0, bottom = 0, left = 0;
    if (argc < 5) {
        return throw_msg(ctx, "setMargin(node, top, right, bottom, left)");
    }
    if (read_edges(ctx, argc, argv, &node, &top, &right, &bottom, &left)) {
        return JS_EXCEPTION;
    }
    int status = host->fns.set_margin(host->fns.session, node, top, right, bottom, left);
    return done(ctx, status, "setMargin failed");
}

static JSValue js_set_min_size(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 3) {
        return throw_msg(ctx, "setMinSize(node, width, height)");
    }
    uint64_t node = 0;
    double width = 0, height = 0;
    if (read_handle(ctx, argv[0], &node) || JS_ToFloat64(ctx, &width, argv[1]) ||
        JS_ToFloat64(ctx, &height, argv[2])) {
        return JS_EXCEPTION;
    }
    int status = host->fns.set_min_size(host->fns.session, node, (float)width, (float)height);
    return done(ctx, status, "setMinSize failed");
}

static JSValue js_set_max_size(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 3) {
        return throw_msg(ctx, "setMaxSize(node, width, height)");
    }
    uint64_t node = 0;
    double width = 0, height = 0;
    if (read_handle(ctx, argv[0], &node) || JS_ToFloat64(ctx, &width, argv[1]) ||
        JS_ToFloat64(ctx, &height, argv[2])) {
        return JS_EXCEPTION;
    }
    int status = host->fns.set_max_size(host->fns.session, node, (float)width, (float)height);
    return done(ctx, status, "setMaxSize failed");
}

static JSValue js_set_shrink(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 2) {
        return throw_msg(ctx, "setShrink(node, shrink)");
    }
    uint64_t node = 0;
    double shrink = 0;
    if (read_handle(ctx, argv[0], &node) || JS_ToFloat64(ctx, &shrink, argv[1])) {
        return JS_EXCEPTION;
    }
    int status = host->fns.set_shrink(host->fns.session, node, (float)shrink);
    return done(ctx, status, "setShrink failed");
}

static JSValue js_set_basis(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 2) {
        return throw_msg(ctx, "setBasis(node, basis)");
    }
    uint64_t node = 0;
    double basis = 0;
    if (read_handle(ctx, argv[0], &node) || JS_ToFloat64(ctx, &basis, argv[1])) {
        return JS_EXCEPTION;
    }
    int status = host->fns.set_basis(host->fns.session, node, (float)basis);
    return done(ctx, status, "setBasis failed");
}

static JSValue js_set_align_self(JSContext *ctx, JSValueConst this_val, int argc,
                                 JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 2) {
        return throw_msg(ctx, "setAlignSelf(node, align)");
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
        return throw_msg(ctx, "align must be start, center, or end");
    }
    int status = host->fns.set_align_self(host->fns.session, node, align);
    return done(ctx, status, "setAlignSelf failed");
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

static JSValue js_set_text_scale(JSContext *ctx, JSValueConst this_val, int argc,
                                 JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 1) {
        return throw_msg(ctx, "setTextScale(scale)");
    }
    double scale = 0;
    if (JS_ToFloat64(ctx, &scale, argv[0])) {
        return JS_EXCEPTION;
    }
    int status = host->fns.set_text_scale(host->fns.session, (float)scale);
    return done(ctx, status, "setTextScale failed");
}

static JSValue js_set_soft_wrap(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 2) {
        return throw_msg(ctx, "setSoftWrap(node, wrap)");
    }
    uint64_t node = 0;
    int32_t wrap = 0;
    if (read_handle(ctx, argv[0], &node) || JS_ToInt32(ctx, &wrap, argv[1])) {
        return JS_EXCEPTION;
    }
    int status = host->fns.set_soft_wrap(host->fns.session, node, wrap);
    return done(ctx, status, "setSoftWrap failed");
}

static JSValue js_set_overflow(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 2) {
        return throw_msg(ctx, "setOverflow(node, overflow)");
    }
    uint64_t node = 0;
    int32_t overflow = 0;
    if (read_handle(ctx, argv[0], &node) || JS_ToInt32(ctx, &overflow, argv[1])) {
        return JS_EXCEPTION;
    }
    int status = host->fns.set_overflow(host->fns.session, node, overflow);
    return done(ctx, status, "setOverflow failed");
}

static JSValue js_caret_at_unit(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 2) {
        return throw_msg(ctx, "caretAtUnit(node, unit)");
    }
    uint64_t node = 0;
    uint32_t unit = 0;
    if (read_handle(ctx, argv[0], &node) || JS_ToUint32(ctx, &unit, argv[1])) {
        return JS_EXCEPTION;
    }
    float x = 0;
    float y = 0;
    float w = 0;
    float h = 0;
    int status = host->fns.caret_at_unit(host->fns.session, node, unit, &x, &y, &w, &h);
    if (status) {
        return throw_msg(ctx, "caretAtUnit failed");
    }
    JSValue result = JS_NewObject(ctx);
    JS_SetPropertyStr(ctx, result, "x", JS_NewFloat64(ctx, x));
    JS_SetPropertyStr(ctx, result, "y", JS_NewFloat64(ctx, y));
    JS_SetPropertyStr(ctx, result, "w", JS_NewFloat64(ctx, w));
    JS_SetPropertyStr(ctx, result, "h", JS_NewFloat64(ctx, h));
    return result;
}

static JSValue js_selection_rects(JSContext *ctx, JSValueConst this_val, int argc,
                                  JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 3) {
        return throw_msg(ctx, "selectionRects(node, start, end)");
    }
    uint64_t node = 0;
    uint32_t start = 0;
    uint32_t end = 0;
    if (read_handle(ctx, argv[0], &node) || JS_ToUint32(ctx, &start, argv[1]) ||
        JS_ToUint32(ctx, &end, argv[2])) {
        return JS_EXCEPTION;
    }
    int count = host->fns.selection_rects(host->fns.session, node, start, end);
    if (count < 0) {
        return throw_msg(ctx, "selectionRects failed");
    }
    size_t bytes = 0;
    const uint8_t *raw = host->fns.selection_rect_bytes(host->fns.session, &bytes);
    JSValue list = JS_NewArray(ctx);
    const float *values = (const float *)raw;
    int floats = (int)(bytes / sizeof(float));
    for (int i = 0; i + 3 < floats; i += 4) {
        JSValue rect = JS_NewObject(ctx);
        JS_SetPropertyStr(ctx, rect, "x", JS_NewFloat64(ctx, values[i]));
        JS_SetPropertyStr(ctx, rect, "y", JS_NewFloat64(ctx, values[i + 1]));
        JS_SetPropertyStr(ctx, rect, "w", JS_NewFloat64(ctx, values[i + 2]));
        JS_SetPropertyStr(ctx, rect, "h", JS_NewFloat64(ctx, values[i + 3]));
        JS_SetPropertyUint32(ctx, list, (uint32_t)(i / 4), rect);
    }
    return list;
}

static JSValue js_set_ime_config(JSContext *ctx, JSValueConst this_val, int argc,
                                 JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 2) {
        return throw_msg(ctx, "setImeConfig(keyboard, action)");
    }
    int32_t keyboard = 0;
    int32_t action = 0;
    if (JS_ToInt32(ctx, &keyboard, argv[0]) || JS_ToInt32(ctx, &action, argv[1])) {
        return JS_EXCEPTION;
    }
    int status = host->fns.set_ime_config(host->fns.session, keyboard, action);
    return done(ctx, status, "setImeConfig failed");
}

static JSValue js_text_edit(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    Host *host = host_of(ctx);
    if (argc < 2) {
        return throw_msg(ctx, "textEdit(node, op, text)");
    }
    uint64_t node = 0;
    int32_t op = 0;
    if (read_handle(ctx, argv[0], &node) || JS_ToInt32(ctx, &op, argv[1])) {
        return JS_EXCEPTION;
    }
    const char *utf8 = NULL;
    size_t len = 0;
    if (argc >= 3 && !JS_IsUndefined(argv[2]) && !JS_IsNull(argv[2])) {
        utf8 = JS_ToCStringLen(ctx, &len, argv[2]);
        if (!utf8) {
            return JS_EXCEPTION;
        }
    }
    uint32_t start = 0;
    uint32_t end = 0;
    int status = host->fns.text_edit(host->fns.session, node, op, utf8, len, &start, &end);
    if (utf8) {
        JS_FreeCString(ctx, utf8);
    }
    if (status) {
        return throw_msg(ctx, "textEdit failed");
    }
    size_t out_len = 0;
    const uint8_t *bytes = host->fns.text_edit_bytes(host->fns.session, &out_len);
    JSValue result = JS_NewObject(ctx);
    JS_SetPropertyStr(ctx, result, "text", JS_NewStringLen(ctx, (const char *)bytes, out_len));
    JS_SetPropertyStr(ctx, result, "start", JS_NewUint32(ctx, start));
    JS_SetPropertyStr(ctx, result, "end", JS_NewUint32(ctx, end));
    return result;
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
    int failed = bind_fn(ctx, core, "column", js_column, 1);
    failed |= bind_fn(ctx, core, "row", js_row, 1);
    failed |= bind_fn(ctx, core, "stack", js_stack, 0);
    failed |= bind_fn(ctx, core, "align", js_align, 1);
    failed |= bind_fn(ctx, core, "padding", js_padding, 1);
    failed |= bind_fn(ctx, core, "pointerRegion", js_pointer_region, 0);
    failed |= bind_fn(ctx, core, "decorated", js_decorated, 1);
    failed |= bind_fn(ctx, core, "text", js_text, 3);
    failed |= bind_fn(ctx, core, "append", js_append, 2);
    failed |= bind_fn(ctx, core, "insert", js_insert, 3);
    failed |= bind_fn(ctx, core, "remove", js_remove, 2);
    failed |= bind_fn(ctx, core, "detach", js_detach, 2);
    failed |= bind_fn(ctx, core, "discard", js_discard, 1);
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
    failed |= bind_fn(ctx, core, "setCross", js_set_cross, 2);
    failed |= bind_fn(ctx, core, "metrics", js_metrics, 0);
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
    failed |= bind_fn(ctx, core, "setScrollPhysics", js_set_scroll_physics, 10);
    failed |= bind_fn(ctx, core, "setScrollChain", js_set_scroll_chain, 2);
    failed |= bind_fn(ctx, core, "overscroll", js_overscroll, 1);
    failed |= bind_fn(ctx, core, "fling", js_arm_fling, 3);
    failed |= bind_fn(ctx, core, "stopScroll", js_stop_scroll, 1);
    failed |= bind_fn(ctx, core, "scrollTo", js_scroll_to, 5);
    failed |= bind_fn(ctx, core, "onScrollState", js_on_scroll_state, 1);
    failed |= bind_fn(ctx, core, "scrollOffset", js_scroll_offset, 1);
    failed |= bind_fn(ctx, core, "rect", js_rect, 1);
    failed |= bind_fn(ctx, core, "applyMutation", js_apply_mutation, 8);
    failed |= bind_fn(ctx, core, "transform", js_transform, 0);
    failed |= bind_fn(ctx, core, "setTransform", js_set_transform, 5);
    failed |= bind_fn(ctx, core, "setJustify", js_set_justify, 2);
    failed |= bind_fn(ctx, core, "setWrap", js_set_wrap, 2);
    failed |= bind_fn(ctx, core, "setFill", js_set_fill, 1);
    failed |= bind_fn(ctx, core, "grid", js_grid, 3);
    failed |= bind_fn(ctx, core, "setPaddingEdges", js_set_padding_edges, 5);
    failed |= bind_fn(ctx, core, "setMargin", js_set_margin, 5);
    failed |= bind_fn(ctx, core, "setMinSize", js_set_min_size, 3);
    failed |= bind_fn(ctx, core, "setMaxSize", js_set_max_size, 3);
    failed |= bind_fn(ctx, core, "setShrink", js_set_shrink, 2);
    failed |= bind_fn(ctx, core, "setBasis", js_set_basis, 2);
    failed |= bind_fn(ctx, core, "setAlignSelf", js_set_align_self, 2);
    failed |= bind_fn(ctx, core, "onHover", js_on_hover, 2);
    failed |= bind_fn(ctx, core, "onLeave", js_on_leave, 2);
    failed |= bind_fn(ctx, core, "onWheel", js_on_wheel, 2);
    failed |= bind_fn(ctx, core, "onSecondaryClick", js_on_secondary_click, 2);
    failed |= bind_fn(ctx, core, "onScroll", js_on_scroll, 2);
    failed |= bind_fn(ctx, core, "onZoom", js_on_zoom, 2);
    failed |= bind_fn(ctx, core, "textEdit", js_text_edit, 2);
    failed |= bind_fn(ctx, core, "setTextScale", js_set_text_scale, 1);
    failed |= bind_fn(ctx, core, "setSoftWrap", js_set_soft_wrap, 2);
    failed |= bind_fn(ctx, core, "setOverflow", js_set_overflow, 2);
    failed |= bind_fn(ctx, core, "caretAtUnit", js_caret_at_unit, 2);
    failed |= bind_fn(ctx, core, "selectionRects", js_selection_rects, 3);
    failed |= bind_fn(ctx, core, "setImeConfig", js_set_ime_config, 2);
    failed |= bind_fn(ctx, core, "sized", js_sized, 2);
    failed |= bind_fn(ctx, core, "setFocusable", js_set_focusable, 2);
    failed |= bind_fn(ctx, core, "focus", js_focus, 1);
    failed |= bind_fn(ctx, core, "blur", js_blur, 0);
    failed |= bind_fn(ctx, core, "setImeEnabled", js_set_ime_enabled, 1);
    failed |= bind_fn(ctx, core, "setImeText", js_set_ime_text, 1);
    failed |= bind_fn(ctx, core, "setImeState", js_set_ime_state, 1);
    failed |= bind_fn(ctx, core, "setSemantics", js_set_semantics, 2);
    failed |= bind_fn(ctx, core, "textScale", js_text_scale, 0);
    failed |= bind_fn(ctx, core, "reduceMotion", js_reduce_motion, 0);
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
    failed |= bind_fn(ctx, core, "onTextReplaced", js_on_text_replaced, 2);
    failed |= bind_fn(ctx, core, "onLink", js_on_link, 2);
    failed |= bind_fn(ctx, core, "onComposing", js_on_composing, 2);
    failed |= bind_fn(ctx, core, "onFrame", js_on_frame, 1);
    failed |= bind_fn(ctx, core, "onLayout", js_on_layout, 1);
    failed |= bind_fn(ctx, core, "setDragClaim", js_set_drag_claim, 2);
    failed |= bind_fn(ctx, core, "caretAt", js_caret_at, 3);
    failed |= bind_fn(ctx, core, "wordRange", js_word_range, 2);
    failed |= bind_fn(ctx, core, "focusStep", js_focus_step, 1);
    failed |= bind_fn(ctx, core, "pushFocusScope", js_push_focus_scope, 1);
    failed |= bind_fn(ctx, core, "popFocusScope", js_pop_focus_scope, 0);
    failed |= bind_fn(ctx, core, "pointerPosition", js_pointer_position, 0);
    failed |= bind_fn(ctx, core, "onBack", js_on_back, 1);
    failed |= bind_fn(ctx, core, "onMotionEnd", js_on_motion_end, 3);
    failed |= bind_fn(ctx, core, "offMotionEnd", js_off_motion_end, 1);
    failed |= bind_fn(ctx, core, "offBack", js_off_back, 1);
    failed |= bind_fn(ctx, core, "onShortcut", js_on_shortcut, 2);
    failed |= bind_fn(ctx, core, "setListenerErrorPolicy", js_set_listener_error_policy, 1);
    failed |= bind_fn(ctx, core, "layerTreeStats", js_layer_tree_stats, 0);
    failed |= bind_fn(ctx, core, "scrollIntoView", js_scroll_into_view, 2);
    failed |= bind_fn(ctx, core, "animate", js_animate, 6);
    failed |= bind_fn(ctx, core, "flingTarget", js_fling_target, 1);
    failed |= bind_fn(ctx, core, "setRecognizers", js_set_recognizers, 2);
    failed |= bind_fn(ctx, core, "setStroke", js_set_stroke, 3);
    failed |= bind_fn(ctx, core, "settings", js_settings, 0);
    failed |= bind_fn(ctx, core, "onSettings", js_on_settings, 1);
    failed |= bind_fn(ctx, core, "offSettings", js_off_settings, 1);
    failed |= bind_fn(ctx, core, "focused", js_focused, 0);
    failed |= bind_fn(ctx, core, "isFocusable", js_is_focusable, 1);
    failed |= bind_fn(ctx, core, "graphemeIndex", js_grapheme_index, 2);
    failed |= bind_fn(ctx, core, "graphemeCount", js_grapheme_count, 1);
    failed |= bind_fn(ctx, core, "graphemeUnit", js_grapheme_unit, 2);
    failed |= bind_fn(ctx, core, "setReverse", js_set_reverse, 2);
    failed |= bind_fn(ctx, core, "setAxisSize", js_set_axis_size, 3);
    failed |= bind_fn(ctx, core, "setAspectRatio", js_set_aspect_ratio, 2);
    failed |= bind_fn(ctx, core, "setPosition", js_set_position, 2);
    failed |= bind_fn(ctx, core, "setInset", js_set_inset, 5);
    failed |= bind_fn(ctx, core, "setGridPlacement", js_set_grid_placement, 5);
    failed |= bind_fn(ctx, core, "setScaleXY", js_set_scale_xy, 3);
    failed |= bind_fn(ctx, core, "registerFont", js_register_font, 2);
    failed |= bind_fn(ctx, core, "setImageMode", js_set_image_mode, 3);
    failed |= bind_fn(ctx, core, "reload", js_reload, 1);
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
    JS_SetMemoryLimit(host->rt, UIFW_JS_MEMORY_LIMIT);
    /* Solid 2's render effects nest deeper than 1 MiB on the gallery page. */
    JS_SetMaxStackSize(host->rt, 8u << 20);
    JS_SetInterruptHandler(host->rt, interrupt_handler, host);
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
    write_exception(host->ctx, err, err_len);
    return -1;
}

int qjs_dispatch(Host *host, uint64_t id, int kind, double a, double b, char *err, size_t err_len) {
    arm_slice(host);
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
    arm_slice(host);
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

#define UIFW_JOB_ROUND_LIMIT 32

int qjs_drain_jobs(Host *host, char *err, size_t err_len) {
    arm_slice(host);
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

int qjs_scroll_state(Host *host, uint64_t id, int phase, float vx, float vy, float ox, float oy,
                      char *err, size_t err_len) {
    if (!host || !host->has_scroll_state) {
        return 0;
    }
    arm_slice(host);
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
    write_exception(host->ctx, err, err_len);
    return -1;
}

int qjs_driver(Host *host, int kind, uint32_t index, uint32_t generation, int tag, int i0, int i1,
               float f0, float f1, float f2, float f3, float f4, float f5, const char *s0, size_t n0,
               const char *s1, size_t n1, char *err, size_t err_len) {
    if (!host || !host->has_driver) {
        return 0;
    }
    arm_slice(host);
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
    write_exception(host->ctx, err, err_len);
    return -1;
}

int qjs_layout(Host *host, char *err, size_t err_len) {
    if (!host || !host->has_layout) {
        return 0;
    }
    arm_slice(host);
    JSValue result = JS_Call(host->ctx, host->layout_cb, JS_UNDEFINED, 0, NULL);
    if (!JS_IsException(result)) {
        JS_FreeValue(host->ctx, result);
        return 0;
    }
    write_exception(host->ctx, err, err_len);
    return -1;
}

int qjs_frame(Host *host, double target_ms, double interval_ms, char *err, size_t err_len) {
    arm_slice(host);
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
    arm_slice(host);
    JSValue object = settings_object(host->ctx, host);
    for (size_t index = 0; index < host->settings_len; index++) {
        JSValue fn = host->settings_handlers[index].fn;
        JSValue result = JS_Call(host->ctx, fn, JS_UNDEFINED, 1, &object);
        if (JS_IsException(result)) {
            JS_FreeValue(host->ctx, object);
            write_exception(host->ctx, err, err_len);
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
    arm_slice(host);
    for (size_t index = host->back_len; index > 0; index--) {
        JSValue fn = host->backs[index - 1].fn;
        JSValue result = JS_Call(host->ctx, fn, JS_UNDEFINED, 0, NULL);
        if (JS_IsException(result)) {
            write_exception(host->ctx, err, err_len);
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
        arm_slice(host);
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

void qjs_close(Host *host) {
    if (!host) {
        return;
    }
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
