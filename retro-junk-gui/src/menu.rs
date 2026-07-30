//! Native menubar built via [`muda`].
//!
//! Menu items the app reacts to carry stable IDs collected in
//! [`AppMenuIds`]; [`RetroJunkApp::process_menu_events`] matches each
//! incoming [`muda::MenuEvent`] against them. Predefined items (Quit, Close
//! Window, Cut/Copy/Paste, About, Hide, Minimize, Zoom) are handled by the
//! OS — no event reaches us for those.
//!
//! Accelerators live here rather than as `consume_key` calls scattered
//! through the frame: the menubar is where a user discovers them, and one
//! definition avoids a shortcut that works but is invisible.
//!
//! [`RetroJunkApp::process_menu_events`]: crate::app::RetroJunkApp

use muda::accelerator::{Accelerator, Code, Modifiers};
use muda::{Menu, MenuId, MenuItem, PredefinedMenuItem, Submenu};

/// IDs of the custom (non-predefined) menu items, so the app can dispatch
/// incoming `MenuEvent`s without holding a borrow on the menu itself.
#[derive(Debug, Clone)]
pub struct AppMenuIds {
    pub open_library: MenuId,
    pub preferences: MenuId,
    pub find: MenuId,
    pub view_collection: MenuId,
    pub view_library: MenuId,
    pub view_inbox: MenuId,
    pub view_settings: MenuId,
    pub view_tools: MenuId,
    pub toggle_log_viewer: MenuId,
}

/// Owned handle to the constructed menu. Holding it keeps the native menu
/// and its items alive for the life of the app.
pub struct AppMenu {
    pub menu: Menu,
    pub ids: AppMenuIds,
    #[cfg(target_os = "macos")]
    pub window_menu: Submenu,
    #[cfg(target_os = "macos")]
    pub help_menu: Submenu,
}

const CMD: Modifiers = Modifiers::META;

fn accel(mods: Modifiers, code: Code) -> Accelerator {
    Accelerator::new(Some(mods), code)
}

/// Build the full application menu.
pub fn build() -> AppMenu {
    let menu = Menu::new();

    let preferences = MenuItem::new("Preferences\u{2026}", true, Some(accel(CMD, Code::Comma)));

    #[cfg(target_os = "macos")]
    {
        let app_menu = Submenu::new("retro-junk", true);
        app_menu
            .append_items(&[
                &PredefinedMenuItem::about(Some("About retro-junk"), None),
                &PredefinedMenuItem::separator(),
                &preferences,
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::services(None),
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::hide(None),
                &PredefinedMenuItem::hide_others(None),
                &PredefinedMenuItem::show_all(None),
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::quit(None),
            ])
            .expect("append app menu");
        menu.append(&app_menu).expect("append app submenu");
    }

    let open_library = MenuItem::new("Open Library\u{2026}", true, Some(accel(CMD, Code::KeyO)));
    let file_menu = Submenu::new("File", true);
    file_menu
        .append_items(&[
            &open_library,
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::close_window(None),
        ])
        .expect("append file menu");
    menu.append(&file_menu).expect("append file submenu");

    let find = MenuItem::new("Find\u{2026}", true, Some(accel(CMD, Code::KeyF)));
    let edit_menu = Submenu::new("Edit", true);
    edit_menu
        .append_items(&[
            &PredefinedMenuItem::undo(None),
            &PredefinedMenuItem::redo(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::cut(None),
            &PredefinedMenuItem::copy(None),
            &PredefinedMenuItem::paste(None),
            &PredefinedMenuItem::select_all(None),
            &PredefinedMenuItem::separator(),
            &find,
        ])
        .expect("append edit menu");
    menu.append(&edit_menu).expect("append edit submenu");

    let view_collection = MenuItem::new("Collection", true, Some(accel(CMD, Code::Digit1)));
    let view_library = MenuItem::new("Library", true, Some(accel(CMD, Code::Digit2)));
    let view_inbox = MenuItem::new("Inbox", true, Some(accel(CMD, Code::Digit3)));
    let view_settings = MenuItem::new("Settings", true, Some(accel(CMD, Code::Digit4)));
    let view_tools = MenuItem::new("Tools", true, Some(accel(CMD, Code::Digit5)));
    let toggle_log_viewer = MenuItem::new(
        "Toggle Log Viewer",
        true,
        Some(accel(CMD | Modifiers::SHIFT, Code::KeyL)),
    );
    let view_menu = Submenu::new("View", true);
    view_menu
        .append_items(&[
            &view_collection,
            &view_library,
            &view_inbox,
            &view_settings,
            &view_tools,
            &PredefinedMenuItem::separator(),
            &toggle_log_viewer,
        ])
        .expect("append view menu");
    menu.append(&view_menu).expect("append view submenu");

    #[cfg(target_os = "macos")]
    let window_menu = {
        let submenu = Submenu::new("Window", true);
        submenu
            .append_items(&[
                &PredefinedMenuItem::minimize(None),
                &PredefinedMenuItem::maximize(None),
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::bring_all_to_front(None),
            ])
            .expect("append window menu");
        menu.append(&submenu).expect("append window submenu");
        submenu
    };

    #[cfg(target_os = "macos")]
    let help_menu = {
        let submenu = Submenu::new("Help", true);
        menu.append(&submenu).expect("append help submenu");
        submenu
    };

    let ids = AppMenuIds {
        open_library: open_library.id().clone(),
        preferences: preferences.id().clone(),
        find: find.id().clone(),
        view_collection: view_collection.id().clone(),
        view_library: view_library.id().clone(),
        view_inbox: view_inbox.id().clone(),
        view_settings: view_settings.id().clone(),
        view_tools: view_tools.id().clone(),
        toggle_log_viewer: toggle_log_viewer.id().clone(),
    };

    AppMenu {
        menu,
        ids,
        #[cfg(target_os = "macos")]
        window_menu,
        #[cfg(target_os = "macos")]
        help_menu,
    }
}

impl AppMenu {
    /// Install the menu as the global app menu. macOS-only: on other
    /// platforms the menu is built but not attached (Windows/Linux need a
    /// window handle, which eframe does not expose here).
    pub fn install(&self) {
        #[cfg(target_os = "macos")]
        {
            self.menu.init_for_nsapp();
            self.window_menu.set_as_windows_menu_for_nsapp();
            self.help_menu.set_as_help_menu_for_nsapp();
        }
    }
}
