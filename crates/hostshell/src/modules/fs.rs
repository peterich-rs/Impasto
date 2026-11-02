use std::path::PathBuf;
use std::sync::Arc;

use session::{HostMethod, HostModule, PluginResult, PluginValue, document_relative};

use super::{done_null, str_arg, string_list, string_pairs};

pub trait StorageHost: Send + Sync {
    fn document_dir(&self) -> Result<String, String>;
    fn pick_document(&self) -> Result<Option<String>, String>;
}

pub fn module(host: Arc<dyn StorageHost>) -> HostModule {
    let dir = host.document_dir();
    HostModule {
        name: "FileSystem",
        specifier: "@impasto/file-system/host",
        methods: vec![
            HostMethod::sync("documentDirectory", {
                let dir = dir.clone();
                move |_| match &dir {
                    Ok(path) => Ok(PluginValue::Str(path.clone())),
                    Err(message) => Err(message.clone()),
                }
            }),
            HostMethod::promise("read", {
                let dir = dir.clone();
                move |args, settle, exec| {
                    let path = str_arg(args);
                    match document_relative(&path) {
                        Err(message) => exec.now(settle, PluginResult::Failed { message }),
                        Ok(relative) => match dir.clone() {
                            Err(message) => exec.now(settle, PluginResult::Failed { message }),
                            Ok(root) => exec.worker(
                                settle,
                                Box::new(move || {
                                    match crate::files::read_document(
                                        &PathBuf::from(root),
                                        &relative,
                                    ) {
                                        Ok(bytes) => PluginResult::Done(PluginValue::Bytes(bytes)),
                                        Err(message) => PluginResult::Failed { message },
                                    }
                                }),
                            ),
                        },
                    }
                }
            }),
            HostMethod::promise("write", {
                let dir = dir.clone();
                move |args, settle, exec| {
                    let path = str_arg(args);
                    let bytes = match args.get(1) {
                        Some(PluginValue::Bytes(bytes)) => bytes.clone(),
                        Some(PluginValue::Str(value)) => value.as_bytes().to_vec(),
                        _ => Vec::new(),
                    };
                    match document_relative(&path) {
                        Err(message) => exec.now(settle, PluginResult::Failed { message }),
                        Ok(relative) => match dir.clone() {
                            Err(message) => exec.now(settle, PluginResult::Failed { message }),
                            Ok(root) => exec.worker(
                                settle,
                                Box::new(move || {
                                    match crate::files::write_document(
                                        &PathBuf::from(root),
                                        &relative,
                                        &bytes,
                                    ) {
                                        Ok(()) => done_null(),
                                        Err(message) => PluginResult::Failed { message },
                                    }
                                }),
                            ),
                        },
                    }
                }
            }),
            HostMethod::promise("delete", {
                move |args, settle, exec| {
                    let path = str_arg(args);
                    match document_relative(&path) {
                        Err(message) => exec.now(settle, PluginResult::Failed { message }),
                        Ok(relative) => match dir.clone() {
                            Err(message) => exec.now(settle, PluginResult::Failed { message }),
                            Ok(root) => exec.worker(
                                settle,
                                Box::new(move || {
                                    match crate::files::delete_document(
                                        &PathBuf::from(root),
                                        &relative,
                                    ) {
                                        Ok(()) => done_null(),
                                        Err(message) => PluginResult::Failed { message },
                                    }
                                }),
                            ),
                        },
                    }
                }
            }),
            HostMethod::promise("pick", {
                let host = Arc::clone(&host);
                move |_args, settle, exec| {
                    let host = Arc::clone(&host);
                    exec.main_thread(
                        settle,
                        Box::new(move || match host.pick_document() {
                            Ok(Some(name)) => PluginResult::Done(PluginValue::Map(vec![(
                                "name".to_owned(),
                                PluginValue::Str(name),
                            )])),
                            Ok(None) => done_null(),
                            Err(message) => PluginResult::Failed { message },
                        }),
                    );
                }
            }),
        ],
    }
}

pub fn async_storage(host: Arc<dyn StorageHost>) -> HostModule {
    let dir = host.document_dir();
    HostModule {
        name: "AsyncStorage",
        specifier: "@impasto/async-storage/host",
        methods: vec![
            HostMethod::promise("multiGet", {
                let dir = dir.clone();
                move |args, settle, exec| {
                    let keys = string_list(args.first());
                    match dir.clone() {
                        Err(message) => exec.now(settle, PluginResult::Failed { message }),
                        Ok(root) => exec.worker(
                            settle,
                            Box::new(move || {
                                match crate::files::store_get(&PathBuf::from(root), &keys) {
                                    Ok(rows) => PluginResult::Done(PluginValue::List(
                                        rows.into_iter()
                                            .map(|(key, value)| {
                                                PluginValue::List(vec![
                                                    PluginValue::Str(key),
                                                    value
                                                        .map(PluginValue::Str)
                                                        .unwrap_or(PluginValue::Null),
                                                ])
                                            })
                                            .collect(),
                                    )),
                                    Err(message) => PluginResult::Failed { message },
                                }
                            }),
                        ),
                    }
                }
            }),
            HostMethod::promise("multiSet", {
                let dir = dir.clone();
                move |args, settle, exec| {
                    let rows = string_pairs(args.first());
                    match dir.clone() {
                        Err(message) => exec.now(settle, PluginResult::Failed { message }),
                        Ok(root) => exec.worker(
                            settle,
                            Box::new(move || {
                                match crate::files::store_set(&PathBuf::from(root), &rows) {
                                    Ok(()) => done_null(),
                                    Err(message) => PluginResult::Failed { message },
                                }
                            }),
                        ),
                    }
                }
            }),
            HostMethod::promise("multiRemove", {
                let dir = dir.clone();
                move |args, settle, exec| {
                    let keys = string_list(args.first());
                    match dir.clone() {
                        Err(message) => exec.now(settle, PluginResult::Failed { message }),
                        Ok(root) => exec.worker(
                            settle,
                            Box::new(move || {
                                match crate::files::store_remove(&PathBuf::from(root), &keys) {
                                    Ok(()) => done_null(),
                                    Err(message) => PluginResult::Failed { message },
                                }
                            }),
                        ),
                    }
                }
            }),
            HostMethod::promise("clear", {
                move |_args, settle, exec| match dir.clone() {
                    Err(message) => exec.now(settle, PluginResult::Failed { message }),
                    Ok(root) => exec.worker(
                        settle,
                        Box::new(
                            move || match crate::files::store_clear(&PathBuf::from(root)) {
                                Ok(()) => done_null(),
                                Err(message) => PluginResult::Failed { message },
                            },
                        ),
                    ),
                }
            }),
        ],
    }
}
