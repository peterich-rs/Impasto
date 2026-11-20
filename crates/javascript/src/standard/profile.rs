/// One part of the standard environment in installation order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StandardModule {
    pub name: &'static str,
    pub dependencies: &'static [&'static str],
    pub globals: &'static [&'static str],
    declarations: &'static [Declaration],
    availability: Availability,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Availability {
    Always,
    Http,
    Socket,
    Legacy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Declaration {
    Shared(&'static str),
    PerProfile {
        legacy: &'static str,
        portable: &'static str,
    },
}

use Declaration::Shared;

/// A standard library profile and the transport capabilities it can expose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StandardProfile {
    portable: bool,
    external_http: bool,
    socket: bool,
}

impl StandardProfile {
    /// Compatibility environment used by existing Impasto applications.
    pub const fn legacy() -> Self {
        Self {
            portable: false,
            external_http: true,
            socket: true,
        }
    }

    /// The documented Web API subset, with external HTTP and socket support.
    pub const fn portable() -> Self {
        Self {
            portable: true,
            external_http: true,
            socket: true,
        }
    }

    /// Describe which external transports are available. The compatibility
    /// profile always keeps its historical globals, including offline stubs.
    pub const fn with_transports(self, external_http: bool, socket: bool) -> Self {
        if self.portable {
            Self {
                external_http: self.external_http && external_http,
                socket: self.socket && socket,
                ..self
            }
        } else {
            self
        }
    }

    pub(super) const fn is_portable(self) -> bool {
        self.portable
    }

    pub(super) const fn has_http(self) -> bool {
        self.external_http
    }

    pub(super) const fn has_socket(self) -> bool {
        self.socket
    }

    pub fn modules(self) -> impl Iterator<Item = &'static StandardModule> {
        MODULES
            .iter()
            .filter(move |module| match module.availability {
                Availability::Always => true,
                Availability::Http => self.external_http,
                Availability::Socket => self.socket,
                Availability::Legacy => !self.portable,
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
            .map(|declaration| match declaration {
                Shared(source) => source,
                Declaration::PerProfile { legacy, portable } => {
                    if self.portable {
                        portable
                    } else {
                        legacy
                    }
                }
            })
            .collect()
    }
}

impl Default for StandardProfile {
    fn default() -> Self {
        Self::legacy()
    }
}

const MODULES: &[StandardModule] = &[
    StandardModule {
        name: "console",
        dependencies: &[],
        globals: &["console"],
        declarations: &[Shared(super::CONSOLE_DECLARATIONS)],
        availability: Availability::Always,
    },
    StandardModule {
        name: "timers",
        dependencies: &[],
        globals: &["setTimeout", "clearTimeout", "setInterval", "clearInterval"],
        declarations: &[Shared(super::TIMER_DECLARATIONS)],
        availability: Availability::Always,
    },
    StandardModule {
        name: "performance",
        dependencies: &[],
        globals: &["performance"],
        declarations: &[Shared(super::PERFORMANCE_DECLARATIONS)],
        availability: Availability::Always,
    },
    StandardModule {
        name: "events",
        dependencies: &[],
        globals: &["Event", "EventTarget", "AbortSignal", "AbortController"],
        declarations: &[
            Shared(super::events::DECLARATIONS),
            Declaration::PerProfile {
                legacy: super::events::LEGACY_ABORT_SIGNAL_CONSTRUCTOR,
                portable: super::events::PORTABLE_ABORT_SIGNAL_CONSTRUCTOR,
            },
            Shared(super::events::ABORT_DECLARATIONS),
        ],
        availability: Availability::Always,
    },
    StandardModule {
        name: "encoding",
        dependencies: &[],
        globals: &["TextEncoder", "TextDecoder"],
        declarations: &[Shared(super::encoding::DECLARATIONS)],
        availability: Availability::Always,
    },
    StandardModule {
        name: "url",
        dependencies: &["encoding"],
        globals: &["URL", "URLSearchParams"],
        declarations: &[Shared(super::url::DECLARATIONS)],
        availability: Availability::Always,
    },
    StandardModule {
        name: "streams",
        dependencies: &[],
        globals: &["ReadableStream", "ReadableStreamDefaultReader"],
        declarations: &[Shared(super::streams::DECLARATIONS)],
        availability: Availability::Always,
    },
    StandardModule {
        name: "headers-body",
        dependencies: &["events", "encoding", "url", "streams"],
        globals: &["Headers", "Request", "Response"],
        declarations: &[
            Shared(super::headers::DECLARATIONS),
            Shared(super::http::BODY_DECLARATIONS),
        ],
        availability: Availability::Always,
    },
    StandardModule {
        name: "fetch",
        dependencies: &["headers-body"],
        globals: &["fetch"],
        declarations: &[Shared(super::http::FETCH_DECLARATIONS)],
        availability: Availability::Http,
    },
    StandardModule {
        name: "fetchStream",
        dependencies: &["fetch"],
        globals: &["fetchStream"],
        declarations: &[Shared(super::http::LEGACY_DECLARATIONS)],
        availability: Availability::Legacy,
    },
    StandardModule {
        name: "websocket",
        dependencies: &["events", "encoding", "url", "headers-body"],
        globals: &["WebSocket"],
        declarations: &[Shared(super::socket::DECLARATIONS)],
        availability: Availability::Socket,
    },
];
