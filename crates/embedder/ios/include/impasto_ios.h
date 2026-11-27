#ifndef IMPASTO_IOS_H
#define IMPASTO_IOS_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define IMPASTO_IOS_ABI_VERSION 2

typedef struct ImpastoIOSHost ImpastoIOSHost;
typedef struct ImpastoIOSWindowGroup ImpastoIOSWindowGroup;

enum {
    IMPASTO_IOS_EVENT_READY = 1,
    IMPASTO_IOS_EVENT_FAILED = 2,
    IMPASTO_IOS_EVENT_FRAME_REQUESTED = 3,
    IMPASTO_IOS_EVENT_FRAME_CANCELLED = 4,
    IMPASTO_IOS_EVENT_FRAME_SUBMITTED = 5,
    IMPASTO_IOS_EVENT_FRAME_PRESENTED = 6,
    IMPASTO_IOS_EVENT_CLOSED = 7,
    IMPASTO_IOS_EVENT_IME = 8,
    IMPASTO_IOS_EVENT_SEMANTICS = 9,
    IMPASTO_IOS_EVENT_BACK_ENABLED = 10,
    IMPASTO_IOS_EVENT_PLATFORM_VIEWS = 11,
    IMPASTO_IOS_EVENT_COMPOSITOR_LAYERS = 12,
    IMPASTO_IOS_EVENT_COMPOSED_FRAME = 13,
    IMPASTO_IOS_EVENT_SOURCE_UPDATED = 14,
    IMPASTO_IOS_EVENT_SERVICE_REQUEST = 20,
    IMPASTO_IOS_EVENT_RESTORATION_CHANGED = 21,
    IMPASTO_IOS_EVENT_BACK_UNHANDLED = 22,
};

typedef void (*ImpastoIOSEvent)(void *context, uint32_t kind, uint64_t value,
                               const uint8_t *bytes, size_t length);
typedef void (*ImpastoIOSRelease)(void *context);

/// bundle_root is a UTF-8 path to trusted, immutable app resources. initial_url
/// may be null. Call these functions on the UIKit main thread. Callbacks can
/// run on any thread and must return promptly without calling into the host.
/// Event bytes are borrowed until the callback returns. The context's release
/// runs once after the final reference, including after synchronous failure.
ImpastoIOSHost *impasto_ios_start(const char *bundle_root, const char *initial_url,
                                 void *context, ImpastoIOSEvent event,
                                 ImpastoIOSRelease release);
ImpastoIOSHost *impasto_ios_start_restored(const char *bundle_root, const char *initial_url,
                                          const char *restored,
                                          const ImpastoIOSWindowGroup *group,
                                          void *context, ImpastoIOSEvent event,
                                          ImpastoIOSRelease release);
ImpastoIOSWindowGroup *impasto_ios_window_group_new(void);
void impasto_ios_window_group_free(ImpastoIOSWindowGroup *group);
/// Returns required UTF-8 byte count. Pass NULL to query the size.
size_t impasto_ios_restoration_copy(ImpastoIOSHost *host, uint8_t *buffer, size_t capacity);
bool impasto_ios_service_pending(ImpastoIOSHost *host, uint64_t request);
bool impasto_ios_service_result(ImpastoIOSHost *host, uint64_t request, uint32_t status,
                                const char *payload);
bool impasto_ios_service_event(ImpastoIOSHost *host, const char *topic, const char *payload);
bool impasto_ios_update_source(ImpastoIOSHost *host, uint64_t revision,
                               const uint8_t *bytes, size_t length);
bool impasto_ios_compositor_layers(ImpastoIOSHost *host, uint64_t surface,
                                   const void *const *layers, size_t count);
void impasto_ios_compositor_finish(ImpastoIOSHost *host, uint64_t surface,
                                    uint64_t serial, bool submitted);
/// The caller transfers context ownership regardless of the result.
bool impasto_ios_texture_frame(ImpastoIOSHost *host, const char *name, void *metal_texture,
                                uint32_t width, uint32_t height, void *context,
                                ImpastoIOSRelease release);
void impasto_ios_texture_unregister(ImpastoIOSHost *host, const char *name);
/// Consumes the handle. CLOSED reports render-thread teardown asynchronously.
void impasto_ios_close(ImpastoIOSHost *host);

/// Returns a nonzero surface ID. The host retains the CAMetalLayer through
/// asynchronous retirement. Set its device and drawable size before attaching.
uint64_t impasto_ios_attach(ImpastoIOSHost *host, void *metal_layer,
                            float width, float height, float scale, float left,
                            float top, float right, float bottom);
void impasto_ios_resize(ImpastoIOSHost *host, uint64_t surface, float width,
                        float height, float scale, float left, float top,
                        float right, float bottom);
void impasto_ios_detach(ImpastoIOSHost *host, uint64_t surface);
bool impasto_ios_surface_dropped(ImpastoIOSHost *host, uint64_t surface);

void impasto_ios_frame(ImpastoIOSHost *host, uint64_t token,
                       double timestamp_seconds, double target_seconds);
/// 0 detached, 1 resumed, 2 inactive, 3 hidden, 4 paused.
void impasto_ios_lifecycle(ImpastoIOSHost *host, uint32_t state);
void impasto_ios_focus(ImpastoIOSHost *host, bool focused);
void impasto_ios_invalidate(ImpastoIOSHost *host);
void impasto_ios_memory_warning(ImpastoIOSHost *host);
void impasto_ios_open_url(ImpastoIOSHost *host, const char *url);
void impasto_ios_settings(ImpastoIOSHost *host, const char *locale,
                          float text_scale, bool dark, bool reduce_motion,
                          bool bold_text, bool high_contrast);
void impasto_ios_display(ImpastoIOSHost *host, int32_t display_id, float hz);
void impasto_ios_keyboard_inset(ImpastoIOSHost *host, uint64_t surface, float bottom);
void impasto_ios_keyboard_occlusion(ImpastoIOSHost *host, uint64_t surface,
                                    float x, float y, float w, float h);
void impasto_ios_ime_replace(ImpastoIOSHost *host, uint64_t surface, uint64_t client,
                             uint64_t sequence, const uint8_t *text, size_t length,
                             uint32_t start, uint32_t end, int32_t compose_start,
                             int32_t compose_end);
void impasto_ios_ime_dismiss(ImpastoIOSHost *host, uint64_t surface, uint64_t client);
void impasto_ios_key(ImpastoIOSHost *host, uint64_t surface, const char *key,
                     uint32_t modifiers);
void impasto_ios_semantics_enabled(ImpastoIOSHost *host, bool enabled, bool autofill);
void impasto_ios_semantics_action(ImpastoIOSHost *host, uint64_t surface, uint64_t id,
                                  uint32_t action, float dx, float dy, uint32_t start,
                                  uint32_t end, const uint8_t *text, size_t length);
void impasto_ios_back(ImpastoIOSHost *host, uint64_t surface, uint32_t phase,
                      float progress, bool from_right);

/// Kinds: touch, mouse, pen, trackpad. Phases: down, move, up, cancel, wheel, zoom.
/// Button: 0 none, 1 primary, 2 secondary, 3 auxiliary. Buttons use bits 1/2/4.
/// Modifier bits are shift/control/alt/meta in that order.
void impasto_ios_pointer(ImpastoIOSHost *host, uint64_t pointer, uint32_t kind,
                         uint32_t phase, float x, float y, double seconds,
                         uint32_t button, uint32_t buttons, uint32_t modifiers,
                         float dx, float dy, float zoom);
void impasto_ios_pen(ImpastoIOSHost *host, uint64_t surface, uint64_t pointer,
                     uint32_t phase, float x, float y, double seconds, float force,
                     float radius, float altitude, float azimuth, uint32_t modifiers);

typedef struct ImpastoIOSPointerPoint {
    float x, y;
    double seconds;
} ImpastoIOSPointerPoint;

/// The latest point is dispatched. The borrowed history is copied during the call.
void impasto_ios_pointer_move(ImpastoIOSHost *host, uint64_t surface, uint64_t pointer,
                              uint32_t kind, ImpastoIOSPointerPoint latest, uint32_t buttons,
                              uint32_t modifiers, float force, float radius, float altitude,
                              float azimuth, const ImpastoIOSPointerPoint *history, size_t length);

#ifdef __cplusplus
}
#endif
#endif
