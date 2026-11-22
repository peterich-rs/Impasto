/// One part of the standard environment in installation order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StandardModule {
    pub name: &'static str,
    pub dependencies: &'static [&'static str],
    pub globals: &'static [&'static str],
    declarations: &'static [&'static str],
    availability: Availability,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Availability {
    Always,
    Http,
    Socket,
}

/// A standard library profile and the transport capabilities it can expose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StandardProfile {
    http: bool,
    socket: bool,
}

impl StandardProfile {
    /// The shared Web API subset with both transport APIs enabled.
    pub const fn new() -> Self {
        Self {
            http: true,
            socket: true,
        }
    }

    /// Intersect the requested APIs with the capabilities of this host.
    pub const fn with_transports(self, http: bool, socket: bool) -> Self {
        Self {
            http: self.http && http,
            socket: self.socket && socket,
        }
    }

    pub(super) const fn has_http(self) -> bool {
        self.http
    }

    pub(super) const fn has_socket(self) -> bool {
        self.socket
    }

    pub fn modules(self) -> impl Iterator<Item = &'static StandardModule> {
        MODULES
            .iter()
            .filter(move |module| match module.availability {
                Availability::Always => true,
                Availability::Http => self.http,
                Availability::Socket => self.socket,
            })
    }

    pub fn globals(self) -> impl Iterator<Item = &'static str> {
        self.modules()
            .flat_map(|module| module.globals.iter().copied())
    }

    /// Ambient TypeScript declarations for the selected runtime profile.
    pub fn declarations(self) -> String {
        self.modules()
            .flat_map(|module| module.declarations.iter().copied())
            .collect()
    }
}

impl Default for StandardProfile {
    fn default() -> Self {
        Self::new()
    }
}

const MODULES: &[StandardModule] = &[
    StandardModule {
        name: "web-primitives",
        dependencies: &[],
        globals: &[
            "queueMicrotask",
            "DOMException",
            "atob",
            "btoa",
            "structuredClone",
            "crypto",
        ],
        declarations: &[super::primitives::DECLARATIONS],
        availability: Availability::Always,
    },
    StandardModule {
        name: "console",
        dependencies: &[],
        globals: &["console"],
        declarations: &[super::CONSOLE_DECLARATIONS],
        availability: Availability::Always,
    },
    StandardModule {
        name: "timers",
        dependencies: &[],
        globals: &["setTimeout", "clearTimeout", "setInterval", "clearInterval"],
        declarations: &[super::TIMER_DECLARATIONS],
        availability: Availability::Always,
    },
    StandardModule {
        name: "performance",
        dependencies: &[],
        globals: &["performance"],
        declarations: &[super::PERFORMANCE_DECLARATIONS],
        availability: Availability::Always,
    },
    StandardModule {
        name: "events",
        dependencies: &[],
        globals: &["Event", "EventTarget", "AbortSignal", "AbortController"],
        declarations: &[
            super::events::DECLARATIONS,
            super::events::ABORT_SIGNAL_CONSTRUCTOR,
            super::events::ABORT_DECLARATIONS,
        ],
        availability: Availability::Always,
    },
    StandardModule {
        name: "encoding",
        dependencies: &[],
        globals: &["TextEncoder", "TextDecoder"],
        declarations: &[super::encoding::DECLARATIONS],
        availability: Availability::Always,
    },
    StandardModule {
        name: "url",
        dependencies: &["encoding"],
        globals: &["URL", "URLSearchParams"],
        declarations: &[super::url::DECLARATIONS],
        availability: Availability::Always,
    },
    StandardModule {
        name: "streams",
        dependencies: &[],
        globals: &[
            "ReadableStream",
            "ReadableStreamDefaultController",
            "ReadableStreamDefaultReader",
        ],
        declarations: &[super::streams::DECLARATIONS],
        availability: Availability::Always,
    },
    StandardModule {
        name: "headers-body",
        dependencies: &["events", "encoding", "url", "streams"],
        globals: &["Headers", "Request", "Response"],
        declarations: &[super::headers::DECLARATIONS, super::http::BODY_DECLARATIONS],
        availability: Availability::Always,
    },
    StandardModule {
        name: "fetch",
        dependencies: &["headers-body"],
        globals: &["fetch"],
        declarations: &[super::http::FETCH_DECLARATIONS],
        availability: Availability::Http,
    },
    StandardModule {
        name: "websocket",
        dependencies: &["events", "encoding", "url", "headers-body"],
        globals: &["WebSocket"],
        declarations: &[super::socket::DECLARATIONS],
        availability: Availability::Socket,
    },
];
