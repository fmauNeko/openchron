//! Device page: the selected device's state and settings.

use gpui_kit::assets::IconName;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::group_box::{GroupBox, GroupBoxVariants as _};
use gpui_kit::component::progress::Progress;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{ActiveTheme as _, Icon, Selectable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, FontWeight, IntoElement, TestSupportExt as _, div, px};

use crate::battery::Thresholds;
use crate::devices::model::{
    Battery, Command, Connection, Details, DeviceId, DeviceState, DpiStages, KeyboardState,
    MouseState, PollingRates, Profiles,
};
use crate::devices::store::{Device, GlobalDevices};
use crate::settings;

/// Short name of a connection.
#[must_use]
pub fn connection_label(connection: &Connection) -> String {
    match connection {
        Connection::Wired => "USB".to_owned(),
        Connection::Receiver(receiver) => receiver.clone(),
        Connection::Bluetooth => "Bluetooth".to_owned(),
    }
}

/// Icon of a connection.
#[must_use]
pub const fn connection_icon(connection: &Connection) -> IconName {
    match connection {
        Connection::Wired => IconName::Usb,
        Connection::Receiver(_) => IconName::Radio,
        Connection::Bluetooth => IconName::Bluetooth,
    }
}

/// Icon of a device kind; a device not read yet shows as a mouse.
#[must_use]
pub fn device_icon(device: &Device) -> IconName {
    match device.state.as_ref().map(|state| &state.details) {
        Some(Details::Keyboard(_)) => IconName::Keyboard,
        Some(Details::Mouse(_)) | None => IconName::Mouse,
    }
}

/// Renders the page of `device`.
pub fn render(device: &Device, cx: &App) -> AnyElement {
    let body = match (&device.state, &device.error) {
        (_, Some(error)) => unavailable(error).into_any_element(),
        (None, None) => h_flex()
            .id("device-loading")
            .test_support()
            .gap_2()
            .text_color(cx.theme().muted_foreground)
            .child(Spinner::new().small())
            .child("Reading the device…")
            .into_any_element(),
        (Some(state), None) if !state.online => offline(state).into_any_element(),
        (Some(state), None) => match &state.details {
            Details::Mouse(mouse) => mouse_sections(&device.id, mouse, cx).into_any_element(),
            Details::Keyboard(keyboard) => keyboard_section(keyboard, cx).into_any_element(),
        },
    };
    v_flex()
        .id("device-page")
        .test_support()
        .gap_6()
        .max_w(px(720.))
        .child(header(device, cx))
        .child(body)
        .into_any_element()
}

fn header(device: &Device, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let tags = device.state.as_ref().map(|state| {
        h_flex()
            .gap_2()
            .child(
                Tag::secondary().small().child(
                    h_flex()
                        .gap_1()
                        .child(Icon::new(connection_icon(&state.connection)).xsmall())
                        .child(connection_label(&state.connection)),
                ),
            )
            .children(state.firmware.as_ref().map(|firmware| {
                Tag::secondary()
                    .outline()
                    .small()
                    .child(format!("Firmware {firmware}"))
            }))
    });
    h_flex()
        .gap_4()
        .items_center()
        .child(
            div()
                .p_3()
                .rounded(theme.radius_lg)
                .bg(theme.muted)
                .child(Icon::new(device_icon(device)).large()),
        )
        .child(
            v_flex()
                .gap_1()
                .child(
                    div()
                        .id("device-name")
                        .test_support()
                        .text_xl()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(device.name.clone()),
                )
                .children(tags),
        )
}

fn unavailable(error: &str) -> impl IntoElement {
    let hint = if cfg!(target_os = "linux") {
        " Install the udev rules shipped with Openchron to give your user access."
    } else {
        ""
    };
    div()
        .id("device-unavailable")
        .test_support()
        .child(Alert::warning(
            "device-unavailable-alert",
            format!("Openchron cannot open this device: {error}.{hint}"),
        ))
}

fn offline(state: &DeviceState) -> impl IntoElement {
    let message = match state.details {
        Details::Mouse(_) => {
            "The mouse does not answer: it is asleep, off or out of range. Move it to wake it up."
        }
        Details::Keyboard(_) => "The keyboard does not answer.",
    };
    div()
        .id("device-offline")
        .test_support()
        .child(Alert::info("device-offline-alert", message))
}

fn section(id: &'static str, title: &'static str, content: impl IntoElement) -> impl IntoElement {
    GroupBox::new().id(id).outline().title(title).child(content)
}

fn mouse_sections(id: &DeviceId, mouse: &MouseState, cx: &App) -> impl IntoElement {
    v_flex()
        .gap_4()
        .children(
            mouse
                .battery
                .map(|battery| section("battery", "Battery", battery_row(battery, cx))),
        )
        .children(
            mouse
                .dpi
                .as_ref()
                .map(|dpi| section("dpi", "DPI", dpi_buttons(id, dpi))),
        )
        .children(
            mouse.polling.as_ref().map(|polling| {
                section("polling-rate", "Polling rate", polling_buttons(id, polling))
            }),
        )
        .children(
            mouse
                .profiles
                .filter(|profiles| profiles.count > 1)
                .map(|profiles| section("profiles", "Profile", profile_buttons(id, profiles))),
        )
}

fn battery_row(battery: Battery, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let settings = settings::current(cx).battery;
    let thresholds = Thresholds::new(settings.low, settings.critical);
    let (icon, color) = if battery.charging {
        (IconName::BatteryCharging, theme.success)
    } else if battery.percent <= thresholds.critical {
        (IconName::BatteryWarning, theme.danger)
    } else if battery.percent <= thresholds.low {
        (IconName::BatteryWarning, theme.warning)
    } else {
        (IconName::Battery, theme.success)
    };
    h_flex()
        .gap_3()
        .items_center()
        .child(Icon::new(icon).text_color(color))
        .child(
            Progress::new("battery-level")
                .value(f32::from(battery.percent))
                .color(color)
                .w(px(240.)),
        )
        .child(
            div()
                .id("battery-percent")
                .test_support()
                .font_weight(FontWeight::MEDIUM)
                .child(format!("{}%", battery.percent)),
        )
        .when(battery.charging, |row| {
            row.child(Tag::success().small().child("Charging"))
        })
}

/// Sends `command` to the device's worker when clicked.
fn on_click_send(
    id: &DeviceId,
    command: Command,
) -> impl Fn(&gpui_kit::ClickEvent, &mut gpui_kit::Window, &mut App) + 'static {
    let id = id.clone();
    move |_, _, cx| {
        // A device unplugged since the last frame has no worker left; its page goes away.
        let _ = cx.global::<GlobalDevices>().0.read(cx).send(&id, command);
    }
}

fn dpi_buttons(id: &DeviceId, dpi: &DpiStages) -> impl IntoElement {
    h_flex()
        .flex_wrap()
        .gap_2()
        .children(dpi.values.iter().enumerate().map(|(index, value)| {
            Button::new(("dpi-stage", index))
                .label(value.to_string())
                .selected(index == dpi.active)
                .when(index == dpi.active, Button::primary)
                .on_click(on_click_send(id, Command::SelectDpiStage(index)))
        }))
}

fn polling_buttons(id: &DeviceId, polling: &PollingRates) -> impl IntoElement {
    h_flex()
        .flex_wrap()
        .gap_2()
        .children(polling.rates.iter().enumerate().map(|(index, rate)| {
            let label = if *rate == 0 {
                "Unknown".to_owned()
            } else {
                format!("{rate} Hz")
            };
            Button::new(("polling-rate", index))
                .label(label)
                .selected(index == polling.active)
                .when(index == polling.active, Button::primary)
                .on_click(on_click_send(id, Command::SelectPollingRate(index)))
        }))
}

fn profile_buttons(id: &DeviceId, profiles: Profiles) -> impl IntoElement {
    h_flex()
        .flex_wrap()
        .gap_2()
        .children((0..profiles.count).map(|profile| {
            Button::new(("profile", usize::from(profile)))
                .label(format!("{}", profile + 1))
                .selected(profile == profiles.active)
                .when(profile == profiles.active, Button::primary)
                .on_click(on_click_send(id, Command::SelectProfile(profile)))
        }))
}

fn keyboard_section(keyboard: &KeyboardState, cx: &App) -> impl IntoElement {
    section(
        "keyboard",
        "Keyboard",
        v_flex()
            .gap_1()
            .children(
                keyboard
                    .via_protocol
                    .map(|version| format!("VIA protocol {version}")),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("Keymap and lighting settings are not available yet."),
            ),
    )
}
