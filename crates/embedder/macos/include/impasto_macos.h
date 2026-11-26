#ifndef IMPASTO_MACOS_H
#define IMPASTO_MACOS_H

#include <stdint.h>
#include <stdbool.h>

/// Starts the Rust host on the AppKit main thread and returns its exit status.
uint8_t impasto_macos_run(bool (*open_url)(const char *url));

#endif
