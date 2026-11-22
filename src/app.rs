//! Application bootstrap: init order, actions, key bindings, menus and the main window.

use gpui_kit::component::notification::Notification;
use gpui_kit::component::{Theme, TitleBar, WindowExt as _};
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Bounds, Global, Menu, MenuItem, QuitMode, WindowBounds,
    WindowOptions, actions, px, size,
};

use crate::hid::{BackendFactory, NativeBackend};
use crate::main_window::MainWindow;
use crate::paths::ConfigPaths;
use crate::settings::{GlobalSettings, LoadNotice};
use crate::{battery, devices, keybindings, notifications, osd, paths, settings};

/// Human-readable application name, used for the window title and the app menu.
pub const APP_NAME: &str = "Openchron";

/// Reverse-DNS identifier shared by the macOS bundle, the Wayland `app_id`,
/// the Windows `AppUserModelID` and the Linux desktop entry.
pub const APP_ID: &str = "ovh.dissidence.openchron";

actions!(openchron, [Quit, OpenSettings]);

/// The main window, while it is open.
#[derive(Default)]
struct MainWindowHandle(Option<AnyWindowHandle>);

impl Global for MainWindowHandle {}

/// Configures the application and opens the main window.
pub fn run(cx: &mut App) {
    gpui_kit::init(cx);
    cx.set_app_identity(APP_ID, APP_NAME);
    let paths = match paths::resolve() {
        Ok(paths) => paths,
        Err(error) => {
            eprintln!("Openchron could not resolve its config directory: {error}");
            cx.quit();
            return;
        }
    };
    if let Err(error) = paths.ensure_dirs() {
        eprintln!("Openchron could not create its config directory: {error}");
    }
    notifications::install(cx);
    init_services(cx, &paths, Some(NativeBackend::factory()));
    cx.on_app_quit(|cx| {
        devices::store::shutdown(cx);
        std::future::ready(())
    })
    .detach();
    match open_main_window(cx) {
        Ok(_) => cx.activate(true),
        Err(error) => {
            eprintln!("Openchron could not open its main window: {error:#}");
            cx.quit();
        }
    }
}

/// Initializes settings, devices and the features built on them. Without `backend`, no HID
/// thread starts and tests feed the device store themselves.
fn init_services(cx: &mut App, paths: &ConfigPaths, backend: Option<BackendFactory>) {
    settings::init(cx, paths);
    Theme::sync_system_appearance(None, cx);
    devices::store::init(cx, backend);
    battery::init(cx);
    osd::init(cx);
    cx.set_global(MainWindowHandle::default());
    // The DPI popup is a window too: only closing the main window ends the app.
    cx.set_quit_mode(QuitMode::Explicit);
    cx.on_window_closed(|cx, id| {
        let main = cx.global::<MainWindowHandle>().0;
        if main.is_some_and(|main| main.window_id() == id) {
            cx.global_mut::<MainWindowHandle>().0 = None;
            cx.quit();
        }
    })
    .detach();
    init_actions(cx);
}

fn init_actions(cx: &mut App) {
    cx.on_action(|_: &Quit, cx| cx.quit());
    keybindings::bind(cx, "secondary-q", Quit, None);
    keybindings::bind(cx, "secondary-,", OpenSettings, None);
    cx.set_menus([Menu::new(APP_NAME).items([
        MenuItem::action("Settings…", OpenSettings),
        MenuItem::separator(),
        MenuItem::action("Quit Openchron", Quit),
    ])]);
    cx.on_system_notification_response(|_, cx| {
        if let Err(error) = open_main_window(cx) {
            eprintln!("Openchron could not open its main window: {error:#}");
        }
        cx.activate(true);
    });
}

/// Opens the main window, or activates the existing one.
///
/// # Errors
/// Returns the platform error if opening the window fails.
pub fn open_main_window(cx: &mut App) -> gpui_kit::Result<AnyWindowHandle> {
    if let Some(main) = cx.global::<MainWindowHandle>().0
        && main
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
    {
        return Ok(main);
    }
    let mut options = TitleBar::window_options();
    if let Some(titlebar) = options.titlebar.as_mut() {
        titlebar.title = Some(APP_NAME.into());
    }
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(960.), px(640.)),
            cx,
        ))),
        window_min_size: Some(size(px(720.), px(480.))),
        app_id: Some(APP_ID.to_owned()),
        ..options
    };
    let (handle, _) = gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| MainWindow::new(window, cx))
    })?;
    cx.global_mut::<MainWindowHandle>().0 = Some(handle);
    handle.update(cx, |_, window, cx| show_load_notice(window, cx))?;
    Ok(handle)
}

/// Tells the user once when the settings file could not be used normally.
fn show_load_notice(window: &mut gpui_kit::Window, cx: &mut App) {
    let store = cx.global::<GlobalSettings>().0.clone();
    let message = match store.update(cx, |store, _| store.take_load_notice()) {
        Some(LoadNotice::Corrupt { backup }) => format!(
            "settings.json was corrupt and was backed up to {}",
            backup.display()
        ),
        Some(LoadNotice::NewerVersion) => {
            "settings.json was written by a newer Openchron version and is read-only".to_owned()
        }
        None => return,
    };
    window.push_notification(Notification::warning(message), cx);
}

/// Initializes the production wiring without HID threads or OS notifications; returns the
/// notifications shown.
///
/// # Panics
/// Panics when the isolated config directory cannot be created.
#[cfg(test)]
pub fn init_for_tests(
    cx: &mut App,
    root: &std::path::Path,
) -> std::rc::Rc<std::cell::RefCell<Vec<gpui_kit::SystemNotification>>> {
    gpui_kit::init(cx);
    cx.set_reduce_motion(true);
    let paths = ConfigPaths::at(root.to_path_buf());
    paths.ensure_dirs().expect("isolated config directory");
    let shown = notifications::install_recorder(cx);
    init_services(cx, &paths, None);
    shown
}

#[cfg(test)]
mod tests {
    use gpui_kit::TestAppContext;

    use super::{init_for_tests, open_main_window};

    #[gpui_kit::test]
    fn main_window_is_reused(cx: &mut TestAppContext) {
        // Given initialized services
        let dir = tempfile::tempdir().unwrap();
        cx.update(|cx| {
            init_for_tests(cx, dir.path());
        });
        // When opening the main window twice
        let first = cx.update(open_main_window).unwrap();
        let second = cx.update(open_main_window).unwrap();
        // Then the second call activates the first window
        assert_eq!(first, second);
        assert_eq!(cx.windows().len(), 1);
    }

    #[gpui_kit::test]
    fn corrupt_settings_are_reported_once(cx: &mut TestAppContext) {
        // Given a corrupt settings file
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("settings.json"), b"{ broken").unwrap();
        cx.update(|cx| {
            init_for_tests(cx, dir.path());
        });
        // When the main window opens
        cx.update(open_main_window).unwrap();
        // Then the file was backed up and the notice consumed
        let backups = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("settings.corrupt-")
            })
            .count();
        assert_eq!(backups, 1);
        let notice = cx.update(|cx| {
            let store = cx.global::<crate::settings::GlobalSettings>().0.clone();
            store.update(cx, |store, _| store.take_load_notice())
        });
        assert_eq!(notice, None);
    }
}
