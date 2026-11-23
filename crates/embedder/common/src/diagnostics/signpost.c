#include <dispatch/dispatch.h>
#include <os/log.h>
#include <os/signpost.h>
#include <stdbool.h>
#include <stdint.h>

static os_log_t impasto_log;

static void init_log(void *context) {
    (void)context;
    impasto_log = os_log_create("org.impasto", OS_LOG_CATEGORY_POINTS_OF_INTEREST);
}

bool impasto_signpost_enabled(void) {
    static dispatch_once_t once;
    dispatch_once_f(&once, NULL, init_log);
    return os_signpost_enabled(impasto_log);
}

uint64_t impasto_signpost_begin(const char *phase) {
    if (!impasto_signpost_enabled()) {
        return 0;
    }
    os_signpost_id_t id = os_signpost_id_generate(impasto_log);
    os_signpost_interval_begin(impasto_log, id, "Impasto phase", "%{public}s", phase);
    return id;
}

void impasto_signpost_end(uint64_t id) {
    os_signpost_interval_end(impasto_log, id, "Impasto phase");
}
