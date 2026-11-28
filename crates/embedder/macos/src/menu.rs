//! Standard responder-chain menus installed on the AppKit thread.

use objc2::runtime::Sel;
use objc2::{MainThreadMarker, MainThreadOnly, sel};
use objc2_app_kit::{NSApplication, NSEventModifierFlags, NSMenu, NSMenuItem};
use objc2_foundation::NSString;

pub(crate) fn install(app: &NSApplication, mtm: MainThreadMarker, title: &str) {
    let main = NSMenu::new(mtm);
    let app_menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str(title));
    add(
        &app_menu,
        &format!("About {title}"),
        sel!(orderFrontStandardAboutPanel:),
        "",
    );
    app_menu.addItem(&NSMenuItem::separatorItem(mtm));
    add(&app_menu, &format!("Hide {title}"), sel!(hide:), "h");
    add(&app_menu, "Hide Others", sel!(hideOtherApplications:), "h")
        .setKeyEquivalentModifierMask(NSEventModifierFlags::Command | NSEventModifierFlags::Option);
    add(&app_menu, "Show All", sel!(unhideAllApplications:), "");
    app_menu.addItem(&NSMenuItem::separatorItem(mtm));
    add(&app_menu, &format!("Quit {title}"), sel!(terminate:), "q");
    attach(mtm, &main, &app_menu);

    let edit = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str("Edit"));
    add(&edit, "Cut", sel!(cut:), "x");
    add(&edit, "Copy", sel!(copy:), "c");
    add(&edit, "Paste", sel!(paste:), "v");
    edit.addItem(&NSMenuItem::separatorItem(mtm));
    add(&edit, "Select All", sel!(selectAll:), "a");
    attach(mtm, &main, &edit);

    let window = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str("Window"));
    add(&window, "Minimize", sel!(performMiniaturize:), "m");
    add(&window, "Zoom", sel!(performZoom:), "");
    add(&window, "Close", sel!(performClose:), "w");
    attach(mtm, &main, &window);
    app.setWindowsMenu(Some(&window));
    app.setMainMenu(Some(&main));
}

fn add(menu: &NSMenu, title: &str, action: Sel, shortcut: &str) -> objc2::rc::Retained<NSMenuItem> {
    // SAFETY: all selectors come from the AppKit responder chain and take a single sender.
    unsafe {
        menu.addItemWithTitle_action_keyEquivalent(
            &NSString::from_str(title),
            Some(action),
            &NSString::from_str(shortcut),
        )
    }
}

fn attach(mtm: MainThreadMarker, main: &NSMenu, submenu: &NSMenu) {
    let item = NSMenuItem::new(mtm);
    item.setTitle(&submenu.title());
    item.setSubmenu(Some(submenu));
    main.addItem(&item);
}
