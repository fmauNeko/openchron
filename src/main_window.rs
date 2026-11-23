//! Main window: the device list and the selected device's page.

use gpui_kit::assets::IconName;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{
    ActiveTheme as _, Icon, Sizable as _, Theme, TitleBar, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, Context, FocusHandle, FontWeight, IntoElement, Render, TestSupportExt as _, Window,
    div, px,
};

use crate::app::{APP_NAME, OpenSettings};
use crate::device_page;
use crate::devices::model::{Details, DeviceId};
use crate::devices::store::{Device, DeviceStoreEvent, GlobalDevices};
use crate::settings::GlobalSettings;
use crate::settings_ui;

/// The main window's view.
pub struct MainWindow {
    selected: Option<DeviceId>,
    focus_handle: FocusHandle,
}

impl MainWindow {
    /// Creates the view and keeps it in sync with the devices, settings and system theme.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let devices = cx.global::<GlobalDevices>().0.clone();
        cx.subscribe_in(
            &devices,
            window,
            |_, _, event: &DeviceStoreEvent, window, cx| {
                match event {
                    DeviceStoreEvent::CommandFailed { name, error, .. } => {
                        window.push_notification(
                            Notification::warning(format!("{name}: {error}")),
                            cx,
                        );
                    }
                    DeviceStoreEvent::Changed
                    | DeviceStoreEvent::DpiChanged { .. }
                    | DeviceStoreEvent::BatteryChanged { .. } => {}
                }
                cx.notify();
            },
        )
        .detach();
        let settings = cx.global::<GlobalSettings>().0.clone();
        cx.observe(&settings, |_, _, cx| cx.notify()).detach();
        window
            .observe_window_appearance(|window, cx| {
                Theme::sync_system_appearance(Some(window), cx);
            })
            .detach();
        let focus_handle = cx.focus_handle();
        focus_handle.focus(window, cx);
        Self {
            selected: None,
            focus_handle,
        }
    }

    /// The selected device, else the first one.
    fn selected<'a>(&self, devices: &'a [Device]) -> Option<&'a Device> {
        self.selected
            .as_ref()
            .and_then(|id| devices.iter().find(|device| &device.id == id))
            .or_else(|| devices.first())
    }

    fn sidebar(
        devices: &[Device],
        selected: Option<&DeviceId>,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        v_flex()
            .id("sidebar")
            .w(px(260.))
            .h_full()
            .flex_none()
            .bg(theme.sidebar)
            .border_r_1()
            .border_color(theme.sidebar_border)
            .text_color(theme.sidebar_foreground)
            .child(
                div()
                    .px_4()
                    .pt_4()
                    .pb_2()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(muted)
                    .child("Devices"),
            )
            .child(
                v_flex()
                    .id("device-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px_2()
                    .gap_1()
                    .children(devices.iter().enumerate().map(|(index, device)| {
                        Self::device_row(index, device, selected == Some(&device.id), cx)
                    }))
                    .when(devices.is_empty(), |list| {
                        list.child(
                            div()
                                .id("device-list-empty")
                                .test_support()
                                .px_2()
                                .py_1()
                                .text_sm()
                                .text_color(muted)
                                .child("No device found"),
                        )
                    }),
            )
            .child(
                div()
                    .p_2()
                    .border_t_1()
                    .border_color(theme.sidebar_border)
                    .child(
                        Button::new("open-settings")
                            .ghost()
                            .w_full()
                            .icon(IconName::Settings)
                            .label("Settings")
                            .on_click(|_, window, cx| settings_ui::open(window, cx)),
                    ),
            )
    }

    fn device_row(
        index: usize,
        device: &Device,
        selected: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let hover = theme.sidebar_accent;
        let id = device.id.clone();
        h_flex()
            .id(("device-row", index))
            .test_support()
            .gap_2()
            .px_2()
            .py_1p5()
            .rounded(theme.radius)
            .cursor_pointer()
            .when(selected, |row| {
                row.bg(theme.sidebar_accent)
                    .text_color(theme.sidebar_accent_foreground)
            })
            .hover(move |style| style.bg(hover))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.selected = Some(id.clone());
                cx.notify();
            }))
            .child(Icon::new(device_page::device_icon(device)).small())
            .child(
                v_flex()
                    .min_w_0()
                    .child(div().text_sm().truncate().child(device.name.clone()))
                    .child(
                        div()
                            .text_xs()
                            .truncate()
                            .text_color(theme.muted_foreground)
                            .child(summary(device)),
                    ),
            )
    }
}

/// One-line status of a device in the list.
fn summary(device: &Device) -> String {
    if device.error.is_some() {
        return "Unavailable".to_owned();
    }
    let Some(state) = &device.state else {
        return "Connecting…".to_owned();
    };
    let connection = device_page::connection_label(&state.connection);
    if !state.online {
        return match state.details {
            Details::Mouse(_) => format!("Asleep · {connection}"),
            Details::Keyboard(_) => format!("Offline · {connection}"),
        };
    }
    match state.mouse().and_then(|mouse| mouse.battery) {
        Some(battery) => format!("{}% · {connection}", battery.percent),
        None => connection,
    }
}

fn empty_state(backend_error: Option<&str>, cx: &Context<MainWindow>) -> AnyElement {
    let muted = cx.theme().muted_foreground;
    v_flex()
        .id("devices-empty")
        .test_support()
        .size_full()
        .items_center()
        .justify_center()
        .gap_3()
        .child(Icon::new(IconName::Mouse).size_12().text_color(muted))
        .child(
            div()
                .text_lg()
                .font_weight(FontWeight::SEMIBOLD)
                .child("No Keychron device found"),
        )
        .child(
            div()
                .text_sm()
                .text_color(muted)
                .child("Connect a Keychron mouse, keyboard or receiver."),
        )
        .children(backend_error.map(|error| {
            div()
                .id("backend-error")
                .test_support()
                .w(px(420.))
                .child(Alert::error(
                    "backend-error-alert",
                    format!("Openchron cannot access HID devices: {error}"),
                ))
        }))
        .into_any_element()
}

impl Render for MainWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let store = cx.global::<GlobalDevices>().0.clone();
        let store = store.read(cx);
        let selected = self.selected(store.devices());
        let content = selected.map_or_else(
            || empty_state(store.backend_error(), cx),
            |device| device_page::render(device, cx),
        );
        let sidebar = Self::sidebar(store.devices(), selected.map(|device| &device.id), cx);
        let theme = cx.theme();
        v_flex()
            .size_full()
            .track_focus(&self.focus_handle)
            .bg(theme.background)
            .text_color(theme.foreground)
            .on_action(cx.listener(|_, _: &OpenSettings, window, cx| {
                settings_ui::open(window, cx);
            }))
            .child(TitleBar::new().child(div().child(APP_NAME)))
            .child(
                h_flex().flex_1().min_h_0().child(sidebar).child(
                    div()
                        .id("device-content")
                        .flex_1()
                        .h_full()
                        .min_w_0()
                        .overflow_y_scroll()
                        .p_6()
                        .child(content),
                ),
            )
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{AnyWindowHandle, App, AppContext as _, TestAppContext, Window};

    use crate::app::{init_for_tests, open_main_window};
    use crate::devices::model::{Command, DeviceId, DeviceUpdate};
    use crate::devices::store::tests::{apply, mouse};

    struct Fixture {
        _dir: tempfile::TempDir,
        window: AnyWindowHandle,
    }

    fn setup(cx: &mut TestAppContext) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        cx.update(|cx| {
            init_for_tests(cx, dir.path());
        });
        let window = cx.update(open_main_window).unwrap();
        Fixture { _dir: dir, window }
    }

    fn ui<R>(
        cx: &mut TestAppContext,
        fx: &Fixture,
        f: impl FnOnce(&mut Window, &mut App) -> R,
    ) -> R {
        cx.update_window(fx.window, |_, window, cx| {
            window.render_frame(cx);
            let result = f(window, cx);
            window.render_frame(cx);
            result
        })
        .unwrap()
    }

    fn attach(cx: &mut TestAppContext) -> mpsc::Receiver<Command> {
        let (commands, worker) = mpsc::channel();
        let id = DeviceId("turbolink".into());
        cx.update(|cx| {
            apply(
                cx,
                DeviceUpdate::Attached {
                    id: id.clone(),
                    name: "Keychron TurboLink 8K".into(),
                    commands,
                },
            );
            apply(
                cx,
                DeviceUpdate::State {
                    id,
                    state: mouse(59, 2),
                },
            );
        });
        worker
    }

    #[gpui_kit::test]
    fn without_devices_the_empty_state_shows(cx: &mut TestAppContext) {
        // Given the main window and no device
        let fx = setup(cx);
        // When rendering it
        ui(cx, &fx, |window, _| {
            // Then the list and the page explain that nothing was found
            assert!(window.find("device-list-empty").visible());
            assert!(window.find("devices-empty").visible());
        });
    }

    #[gpui_kit::test]
    fn a_mouse_is_listed_with_its_page(cx: &mut TestAppContext) {
        // Given the main window
        let fx = setup(cx);
        // When a mouse reports its state
        attach(cx);
        ui(cx, &fx, |window, _| {
            // Then it is listed and selected, with its battery and DPI stages
            assert!(window.find(("device-row", 0usize)).visible());
            assert!(window.find("device-name").visible());
            assert!(window.find("battery-percent").visible());
            assert!(window.find(("dpi-stage", 4usize)).visible());
            assert!(window.try_find("devices-empty").is_none());
        });
    }

    #[gpui_kit::test]
    fn clicking_a_dpi_stage_sends_it_to_the_worker(cx: &mut TestAppContext) {
        // Given a listed mouse
        let fx = setup(cx);
        let worker = attach(cx);
        // When clicking the 800 DPI stage
        ui(cx, &fx, |window, cx| {
            window.click(("dpi-stage", 1usize), cx);
        });
        // Then the worker is asked to select stage 1
        assert_eq!(worker.try_recv(), Ok(Command::SelectDpiStage(1)));
    }

    #[gpui_kit::test]
    fn offline_mouse_shows_why(cx: &mut TestAppContext) {
        // Given the main window
        let fx = setup(cx);
        // When a receiver reports its mouse asleep
        let mut asleep = mouse(59, 2);
        asleep.online = false;
        cx.update(|cx| {
            apply(
                cx,
                DeviceUpdate::State {
                    id: DeviceId("turbolink".into()),
                    state: asleep,
                },
            );
        });
        ui(cx, &fx, |window, _| {
            // Then the page explains it instead of showing stale settings
            assert!(window.find("device-offline").visible());
            assert!(window.try_find(("dpi-stage", 0usize)).is_none());
        });
    }

    #[gpui_kit::test]
    fn access_problems_are_explained(cx: &mut TestAppContext) {
        // Given the main window
        let fx = setup(cx);
        // When the HID stack fails
        cx.update(|cx| apply(cx, DeviceUpdate::BackendFailed("no HID".into())));
        // Then the empty state explains it
        ui(cx, &fx, |window, _| {
            assert!(window.find("backend-error").visible());
        });
        // When a device cannot be opened
        cx.update(|cx| {
            apply(
                cx,
                DeviceUpdate::Unavailable {
                    id: DeviceId("turbolink".into()),
                    name: "Keychron TurboLink 8K".into(),
                    error: "access denied".into(),
                },
            );
        });
        // Then it is listed and its page explains why it cannot be used
        ui(cx, &fx, |window, _| {
            assert!(window.find(("device-row", 0usize)).visible());
            assert!(window.find("device-unavailable").visible());
        });
    }

    #[gpui_kit::test]
    fn settings_shortcut_opens_the_dialog(cx: &mut TestAppContext) {
        // Given the main window
        let fx = setup(cx);
        // When pressing the settings shortcut
        ui(cx, &fx, |window, cx| window.press("secondary-,", cx));
        cx.run_until_parked();
        // Then the settings dialog is open
        ui(cx, &fx, |window, _| {
            assert!(window.find("settings-battery-notifications").visible());
        });
    }
}
