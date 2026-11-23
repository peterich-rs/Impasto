//! Behavior owned by `js-host`. Session tests cover the compatibility wrapper.

use std::sync::{Arc, mpsc};
use std::thread;

use javascript::host::{
    ImmediateExec, Invoke, IoEvent, Method, MethodExec, Module, ModuleRegistry, PluginResult,
    PluginValue, Settle, StreamEvent, Work,
};

fn sync_method(name: &'static str, text: &'static str) -> Method {
    Method::sync(name, move |_| Ok(PluginValue::Str(text.into())))
}

#[test]
fn host_values_preserve_structure() {
    let value = PluginValue::List(vec![
        PluginValue::Null,
        PluginValue::Bool(true),
        PluginValue::F64(1.5),
        PluginValue::Str("s".into()),
        PluginValue::Bytes(vec![0, 255]),
        PluginValue::Map(vec![
            ("a".into(), PluginValue::F64(1.0)),
            ("a".into(), PluginValue::Bytes(b"a".to_vec())),
        ]),
    ]);
    assert_eq!(value.clone(), value);
    let PluginValue::List(items) = &value else {
        panic!("list");
    };
    assert_eq!(items[4], PluginValue::Bytes(vec![0, 255]));
    assert_ne!(items[4], PluginValue::Str("\0ÿ".into()));
    let PluginValue::Map(pairs) = &items[5] else {
        panic!("map");
    };
    assert_eq!(pairs.len(), 2);
    assert_eq!(pairs[0].0, "a");
    assert_eq!(pairs[1].0, "a");
}

#[test]
fn host_settle_send_and_drop_once() {
    let (tx, rx) = mpsc::channel();
    let settle = Settle::new(move |result| {
        let _ = tx.send(result);
    });
    let worker = thread::spawn(move || drop(settle));
    if worker.join().is_err() {
        panic!("drop worker panicked");
    }
    match rx.try_recv() {
        Ok(PluginResult::Failed { message }) => assert_eq!(message, "settle dropped"),
        other => panic!("{other:?}"),
    }
    assert!(rx.try_recv().is_err());

    let (tx, rx) = mpsc::channel();
    let settle = Settle::new(move |result| {
        let _ = tx.send(result);
    });
    let worker = thread::spawn(move || {
        settle.send(PluginResult::Done(PluginValue::Null));
    });
    if worker.join().is_err() {
        panic!("send worker panicked");
    }
    match rx.try_recv() {
        Ok(PluginResult::Done(PluginValue::Null)) => {}
        other => panic!("{other:?}"),
    }
    assert!(rx.try_recv().is_err());
}

#[test]
fn host_registry_is_namespace_neutral() {
    let first = Module {
        name: "Clipboard",
        exports: Default::default(),
        specifier: "app:clipboard",
        methods: vec![sync_method("get", "one"), sync_method("get", "two")],
    };
    let second = Module {
        name: "Clipboard",
        exports: Default::default(),
        specifier: "other:clipboard",
        methods: vec![sync_method("get", "other")],
    };
    let kept_method = sync_method("ping", "pong");
    let Invoke::Sync(kept_callback) = kept_method.invoke.clone() else {
        panic!("sync callback");
    };
    let third = Module {
        name: "Ping",
        exports: Default::default(),
        specifier: "not-impasto",
        methods: vec![kept_method],
    };
    let registry = ModuleRegistry::new(vec![first, second, third]);
    match registry.call("Clipboard", "get", &[]) {
        Ok(PluginValue::Str(text)) => assert_eq!(text, "one"),
        other => panic!("{other:?}"),
    }
    assert_eq!(
        registry
            .by_specifier("app:clipboard")
            .map(|module| module.name),
        Some("Clipboard")
    );
    let filtered = registry.filtered(|module| module.specifier != "other:clipboard");
    assert!(filtered.by_specifier("other:clipboard").is_none());
    assert!(filtered.module("Clipboard").is_some());
    assert!(filtered.module("Ping").is_some());
    let Invoke::Sync(found) = &filtered
        .method("Ping", "ping")
        .unwrap_or_else(|| panic!("ping"))
        .invoke
    else {
        panic!("sync callback");
    };
    assert!(Arc::ptr_eq(&kept_callback, found));
    let again = filtered.filtered(|_| true);
    match again.module("Clipboard").map(|module| module.specifier) {
        Some("app:clipboard") => {}
        other => panic!("{other:?}"),
    }
}

#[test]
fn transport_variants_roundtrip() {
    let (tx, rx) = mpsc::channel();
    let event = IoEvent::NativeStream {
        id: 4,
        event: StreamEvent::Failed("Canceled".into()),
    };
    if tx.send(event).is_err() {
        panic!("send");
    }
    let event = match rx.try_recv() {
        Ok(event) => event,
        Err(error) => panic!("{error}"),
    };
    match event {
        IoEvent::NativeStream {
            id: 4,
            event: StreamEvent::Failed(message),
        } => assert_eq!(message, "Canceled"),
        _ => panic!("native stream"),
    }
}

fn assert_send<T: Send>() {}
fn assert_sync<T: Sync>() {}

#[test]
fn host_contracts_are_send() {
    assert_send::<PluginValue>();
    assert_send::<PluginResult>();
    assert_send::<Settle>();
    assert_send::<Work>();
    assert_send::<IoEvent>();
    assert_send::<ImmediateExec>();
    assert_sync::<ImmediateExec>();
    assert_send::<Arc<dyn MethodExec>>();
    assert_sync::<Arc<dyn MethodExec>>();
}

#[test]
fn strict_module_registry_validates_shared_export_metadata() {
    use javascript::host::{EventType, ModuleExports, RegistryError};

    let module = Module {
        name: "Storage",
        specifier: "app:storage",
        methods: vec![
            Method::promise("delete", |_, _, _| {}).with_types("path: string", "void"),
            sync_method("$delete", "present").with_types("", "string"),
        ],
        exports: ModuleExports {
            object: true,
            events: &[EventType {
                name: "change",
                payload: "{ path: string }",
            }],
            ..ModuleExports::default()
        },
    };
    assert!(ModuleRegistry::try_new(vec![module.clone()]).is_ok());
    assert_eq!(
        module.export_names().collect::<Vec<_>>(),
        [
            "delete",
            "$delete",
            "addEventListener",
            "removeEventListener",
            "Storage"
        ],
    );
    let declaration = module
        .typescript_declaration()
        .unwrap_or_else(|error| panic!("{error}"));
    assert!(declaration.contains("delete(path: string): Promise<void>;"));
    assert!(declaration.contains("export { $$delete as delete };"));
    assert!(declaration.contains("export declare function $delete(): string;"));
    assert!(declaration.contains(
        "export declare function addEventListener(type: \"change\", listener: (event: { path: string }) => void): void;"
    ));

    let with_method = |name| Module {
        methods: vec![sync_method(name, "")],
        ..module.clone()
    };
    for name in ["Storage", "addEventListener", "removeEventListener"] {
        assert!(matches!(
            ModuleRegistry::try_new(vec![with_method(name)]),
            Err(RegistryError::DuplicateExport { name: found, .. }) if found == name
        ));
    }
    assert!(matches!(
        ModuleRegistry::try_new(vec![with_method("bad-name")]),
        Err(RegistryError::InvalidExport { .. })
    ));
    assert!(matches!(
        ModuleRegistry::try_new(vec![module.clone(), module.clone()]),
        Err(RegistryError::DuplicateSpecifier("app:storage"))
    ));
    assert!(matches!(
        ModuleRegistry::try_new(vec![
            module.clone(),
            Module {
                specifier: "other",
                ..module.clone()
            },
        ]),
        Err(RegistryError::DuplicateName("Storage"))
    ));
    assert!(matches!(
        ModuleRegistry::try_new(vec![Module {
            specifier: "app/../storage",
            ..module
        }]),
        Err(RegistryError::InvalidSpecifier("app/../storage"))
    ));
}
