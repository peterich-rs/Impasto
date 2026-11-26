use std::cell::RefCell;
use std::path::PathBuf;

use block2::RcBlock;
use futures_channel::oneshot;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSModalResponseOK, NSOpenPanel};

use impasto::Runtime;

pub(super) async fn pick(runtime: &Runtime, root: PathBuf) -> Result<Option<String>, String> {
    let (send, receive) = oneshot::channel();
    runtime
        .spawn_main(move || {
            let Some(main) = MainThreadMarker::new() else {
                return;
            };
            let app = NSApplication::sharedApplication(main);
            let panel = NSOpenPanel::openPanel(main);
            panel.setCanChooseDirectories(false);
            panel.setCanChooseFiles(true);
            let selected = panel.clone();
            let send = RefCell::new(Some(send));
            let finish = RcBlock::new(move |result| {
                if let Some(send) = send.borrow_mut().take() {
                    let path = (result == NSModalResponseOK)
                        .then(|| selected.URL())
                        .flatten()
                        .and_then(|url| url.path())
                        .map(|path| PathBuf::from(path.to_string()));
                    let _ = send.send(path);
                }
            });
            if let Some(window) = app.keyWindow().or_else(|| app.mainWindow()) {
                panel.beginSheetModalForWindow_completionHandler(&window, &finish);
            } else {
                panel.beginWithCompletionHandler(&finish);
            }
        })
        .await
        .map_err(|error| error.to_string())?;
    let Some(source) = receive
        .await
        .map_err(|_| "the document picker is unavailable".to_owned())?
    else {
        return Ok(None);
    };
    runtime
        .spawn_blocking(move || copy_document(root, source))
        .await
        .map_err(|error| error.to_string())?
        .map(Some)
}

fn copy_document(root: PathBuf, source: PathBuf) -> Result<String, String> {
    let name = source
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("the selected document has no valid file name")?;
    super::files::document_relative(name)?;
    let imports = root.join("Imports");
    std::fs::create_dir_all(&imports).map_err(|error| error.to_string())?;
    let folder = tempfile::Builder::new()
        .prefix("document-")
        .tempdir_in(&imports)
        .map_err(|error| error.to_string())?;
    let destination = folder.path().join(name);
    std::fs::copy(&source, &destination).map_err(|error| error.to_string())?;
    let relative = destination
        .strip_prefix(&root)
        .map_err(|error| error.to_string())?
        .to_string_lossy()
        .into_owned();
    let _ = folder.keep();
    Ok(relative)
}
