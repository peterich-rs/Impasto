/// One part of the standard environment in installation order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StandardModule {
    pub name: &'static str,
    pub dependencies: &'static [&'static str],
    pub globals: &'static [&'static str],
    declarations: &'static [&'static str],
}

/// The standard library profile installed into every Impasto JavaScript runtime.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StandardProfile;

impl StandardProfile {
    /// The shared Web API subset installed into every runtime.
    pub const fn new() -> Self {
        Self
    }

    pub fn modules(self) -> impl Iterator<Item = &'static StandardModule> {
        MODULES.iter()
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
    },
    StandardModule {
        name: "console",
        dependencies: &[],
        globals: &["console"],
        declarations: &[super::CONSOLE_DECLARATIONS],
    },
    StandardModule {
        name: "timers",
        dependencies: &[],
        globals: &["setTimeout", "clearTimeout", "setInterval", "clearInterval"],
        declarations: &[super::TIMER_DECLARATIONS],
    },
    StandardModule {
        name: "performance",
        dependencies: &[],
        globals: &["performance"],
        declarations: &[super::PERFORMANCE_DECLARATIONS],
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
    },
    StandardModule {
        name: "encoding",
        dependencies: &[],
        globals: &["TextEncoder", "TextDecoder"],
        declarations: &[super::encoding::DECLARATIONS],
    },
    StandardModule {
        name: "url",
        dependencies: &["encoding"],
        globals: &["URL", "URLSearchParams"],
        declarations: &[super::url::DECLARATIONS],
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
    },
];
