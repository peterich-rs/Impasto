#ifndef IMPASTO_MACOS_H
#define IMPASTO_MACOS_H

#include <stdint.h>
#include <stdbool.h>

/// Starts the Rust host on the AppKit main thread and returns its exit status.
uint8_t impasto_macos_run(bool (*open_url)(const char *url));

typedef struct ImpastoMacOSCallbacks {
    void *context;
    bool (*open_url)(void *context, const char *url);
    void (*service_request)(void *context, uint64_t id, const char *operation, const char *payload);
    void (*closed)(void *context);
} ImpastoMacOSCallbacks;

/// Starts the host on the AppKit main thread. Callbacks and context live until it returns.
uint8_t impasto_macos_run_host(const ImpastoMacOSCallbacks *callbacks);
bool impasto_macos_service_pending(uint64_t request_id);
bool impasto_macos_service_result(uint64_t request_id, uint32_t status, const char *payload);
bool impasto_macos_service_event(const char *topic, const char *payload);

#endif
