//! Application bootstrap: global setup, key bindings, menus and the main window.

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyWindowHandle, App, Bounds, Context, Menu, MenuItem, TestSupportExt as _, TitlebarOptions,
    Window, WindowBounds, WindowOptions, actions, div, px, size,
};

use crate::keybindings;

/// Human-readable application name, used for the window title and the app menu.
pub const APP_NAME: &str = "Openchron";

/// Reverse-DNS identifier shared by the macOS bundle, the Wayland `app_id`,
/// the Windows `AppUserModelID` and the Linux desktop entry.
pub const APP_ID: &str = "ovh.dissidence.openchron";

actions!(openchron, [Quit]);

/// Configures the application and opens the main window.
pub fn run(cx: &mut App) {
    gpui_kit::init(cx);

    cx.on_action(|_: &Quit, cx| cx.quit());
    keybindings::bind(cx, "secondary-q", Quit, None);
    cx.set_menus([Menu::new(APP_NAME).items([MenuItem::action("Quit Openchron", Quit)])]);

    // Openchron has no background mode yet: closing the last window ends the process.
    cx.on_window_closed(|cx, _| {
        if cx.windows().is_empty() {
            cx.quit();
        }
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

fn open_main_window(cx: &mut App) -> gpui_kit::Result<AnyWindowHandle> {
    let options = WindowOptions {
        titlebar: Some(TitlebarOptions {
            title: Some(APP_NAME.into()),
            ..Default::default()
        }),
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(1000.), px(700.)),
            cx,
        ))),
        window_min_size: Some(size(px(640.), px(480.))),
        app_id: Some(APP_ID.to_owned()),
        ..Default::default()
    };
    let (window, _) = gpui_kit::open_window(options, cx, |_, cx| cx.new(|_| Welcome))?;
    Ok(window)
}

/// Placeholder content shown until devices are listed.
struct Welcome;

impl Render for Welcome {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .id("welcome")
            .test_support()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_2()
            .bg(theme.background)
            .child(
                div()
                    .text_3xl()
                    .text_color(theme.foreground)
                    .child(APP_NAME),
            )
            .child(
                div()
                    .text_color(theme.muted_foreground)
                    .child("Configure your Keychron keyboards and mice."),
            )
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{AppContext as _, TestAppContext};

    use super::open_main_window;

    #[gpui_kit::test]
    fn main_window_renders_the_welcome_view(cx: &mut TestAppContext) {
        // Given: an initialized application.
        cx.update(gpui_kit::init);

        // When: the main window opens.
        let window = cx.update(open_main_window).expect("the main window opens");

        // Then: the welcome view is laid out and visible.
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.find("welcome").visible());
        })
        .expect("the main window is still open");
    }
}
