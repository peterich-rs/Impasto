#![cfg(feature = "runtime")]
#![allow(clippy::expect_used, reason = "compile and catalog test assertions")]

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use javascript::compiler::compile;
use javascript::host::{Method, Module, ModuleRegistry, PluginValue};
use javascript::runtime::{ModuleCatalog, ModuleOutcome, Runtime};

#[test]
fn compiler_checks_metadata_without_running_javascript_or_host_methods() {
    let calls = Arc::new(AtomicUsize::new(0));
    let host_calls = Arc::clone(&calls);
    let registry = ModuleRegistry::try_new(vec![Module {
        name: "Files",
        specifier: "app:files",
        exports: Default::default(),
        methods: vec![Method::sync("delete", move |_| {
            host_calls.fetch_add(1, Ordering::Relaxed);
            Ok(PluginValue::Null)
        })],
    }])
    .expect("registry");
    let mut catalog = ModuleCatalog::new(registry).expect("catalog");
    catalog
        .add_source(
            "pkg/tools.js",
            "export { delete as remove } from 'app:files'; throw new Error('not executed');",
        )
        .expect("register relative module");

    compile(
        "import { remove } from './tools.js'; await remove();",
        "pkg/main.js",
        true,
        &catalog,
    )
    .expect("valid re-export and top-level await");
    let error = compile(
        "import { missing } from 'app:files';",
        "main.js",
        true,
        &catalog,
    )
    .expect_err("missing export");
    assert!(
        error.to_string().contains("does not export \"missing\""),
        "{error}"
    );
    let error = compile("import '../outside.js';", "main.js", true, &catalog)
        .expect_err("out of the virtual directory");
    assert!(error.to_string().contains("is not registered"), "{error}");
    assert_eq!(calls.load(Ordering::Relaxed), 0);
}

#[test]
fn compiler_links_cycles_and_detects_ambiguous_star_exports() {
    let mut catalog = ModuleCatalog::default();
    for (name, source) in [
        (
            "pkg/a.js",
            "import { entry } from './main.js'; export { entry }; export const value = 1;",
        ),
        ("pkg/b.js", "export { value } from './a.js';"),
        ("pkg/c.js", "export const value = 2;"),
        (
            "pkg/same.js",
            "export * from './a.js'; export * from './b.js';",
        ),
        (
            "pkg/ambiguous.js",
            "export * from './a.js'; export * from './c.js';",
        ),
    ] {
        catalog.add_source(name, source).expect("register module");
    }
    compile(
        "import { value } from './same.js'; export const entry = value;",
        "pkg/main.js",
        true,
        &catalog,
    )
    .expect("the cycle resolves to a single binding");
    let error = compile(
        "import { value } from './ambiguous.js'; export const entry = value;",
        "pkg/main.js",
        true,
        &catalog,
    )
    .expect_err("two different bindings");
    assert!(
        error.to_string().contains("ambiguous export \"value\""),
        "{error}"
    );
    compile(
        "const specifier = './somewhere.js'; export const value = await import(specifier);",
        "pkg/main.js",
        true,
        &catalog,
    )
    .expect("dynamic module names are resolved at their execution point");
}

#[test]
fn virtual_paths_and_reserved_names_are_checked_on_registration() {
    let mut catalog = ModuleCatalog::default();
    catalog.add_source("app/main.js", "").expect("plain path");
    assert!(catalog.add_source("app/main.js", "").is_err());
    for name in [
        "",
        "./main.js",
        "../main.js",
        "app//main.js",
        "app/./main.js",
        "app\\main.js",
    ] {
        assert!(catalog.add_source(name, "").is_err(), "{name}");
    }
}

#[test]
fn runtime_resolves_dynamic_names_against_the_same_catalog() {
    let mut catalog = ModuleCatalog::default();
    let entry =
        "const { result } = await import('./data.js'); if (result !== 42) throw Error('value');";
    let rejected = "await import('../../outside.js');";
    for (name, source) in [
        ("pkg/main.js", entry),
        (
            "pkg/data.js",
            "export const result = await Promise.resolve(42);",
        ),
        ("pkg/rejected.js", rejected),
    ] {
        catalog.add_source(name, source).expect("source module");
    }
    compile(entry, "pkg/main.js", true, &catalog).expect("compiled dynamic import");
    let mut runtime = Runtime::builder()
        .modules(catalog)
        .build()
        .expect("runtime");
    let fulfilled = runtime.start_module("pkg/main.js").expect("dynamic entry");
    let rejected = runtime
        .start_module("pkg/rejected.js")
        .expect("rejecting entry");
    runtime.drive(47).expect("module jobs");
    assert_eq!(
        runtime
            .take_module_result(fulfilled)
            .expect("take fulfillment"),
        Some(ModuleOutcome::Fulfilled)
    );
    let Some(ModuleOutcome::Rejected(error)) = runtime
        .take_module_result(rejected)
        .expect("take rejection")
    else {
        panic!("the disallowed dynamic import must reject its entry");
    };
    assert!(error.to_string().contains("not registered"), "{error}");
}
