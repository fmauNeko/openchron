//! Settings dialog: low-battery notifications and the DPI popup.

use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::group_box::{GroupBox, GroupBoxVariants as _};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{
    Disableable as _, Selectable as _, Sizable as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{App, Context, Entity, IntoElement, Render, TestSupportExt as _, Window, div, px};

use crate::settings::{GlobalSettings, Settings, SettingsStore};

/// Low-battery threshold choices, in percent.
const LOW_CHOICES: [u8; 5] = [10, 15, 20, 25, 30];
/// Critical threshold choices, in percent.
const CRITICAL_CHOICES: [u8; 3] = [5, 10, 15];
/// DPI popup duration choices, in milliseconds.
const DURATION_CHOICES: [u32; 4] = [1000, 1500, 2000, 3000];

struct SettingsView {
    store: Entity<SettingsStore>,
}

impl SettingsView {
    fn new(cx: &mut Context<Self>) -> Self {
        let store = cx.global::<GlobalSettings>().0.clone();
        cx.observe(&store, |_, _, cx| cx.notify()).detach();
        Self { store }
    }

    fn apply(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        change: impl FnOnce(&mut Settings),
    ) {
        if let Err(error) = self.store.update(cx, |store, cx| store.update(cx, change)) {
            window.push_notification(Notification::error(error.to_string()), cx);
        }
    }

    fn choices<T: Copy + PartialEq + 'static>(
        id: &'static str,
        label: &'static str,
        options: &[(T, String)],
        current: T,
        read_only: bool,
        set: fn(&mut Settings, T),
        cx: &Context<Self>,
    ) -> impl IntoElement {
        h_flex()
            .gap_3()
            .items_center()
            .child(div().w(px(96.)).text_sm().child(label))
            .child(h_flex().gap_1().children(options.iter().enumerate().map(
                |(index, (value, text))| {
                    let value = *value;
                    Button::new((id, index))
                        .small()
                        .label(text.clone())
                        .selected(value == current)
                        .when(value == current, Button::primary)
                        .disabled(read_only)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.apply(window, cx, |settings| set(settings, value));
                        }))
                },
            )))
    }
}

fn percent_choices(values: &[u8]) -> Vec<(u8, String)> {
    values
        .iter()
        .map(|&percent| (percent, format!("{percent}%")))
        .collect()
}

impl Render for SettingsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let store = self.store.read(cx);
        let read_only = store.is_read_only();
        let Settings {
            battery, dpi_popup, ..
        } = store.settings().clone();
        let durations: Vec<_> = DURATION_CHOICES
            .iter()
            .map(|&ms| (ms, format!("{} s", f64::from(ms) / 1000.)))
            .collect();
        v_flex()
            .gap_4()
            .when(read_only, |this| {
                this.child(div().id("settings-read-only").test_support().child(
                    Alert::warning(
                        "settings-read-only-alert",
                        "The settings file comes from a newer version of Openchron and is read-only.",
                    ),
                ))
            })
            .child(
                GroupBox::new().outline().title("Battery").child(
                    v_flex()
                        .gap_3()
                        .child(
                            Switch::new("settings-battery-notifications")
                                .label("Notify when a battery runs low")
                                .checked(battery.notifications)
                                .disabled(read_only)
                                .on_click(cx.listener(|this, checked: &bool, window, cx| {
                                    let checked = *checked;
                                    this.apply(window, cx, |settings| {
                                        settings.battery.notifications = checked;
                                    });
                                })),
                        )
                        .child(Self::choices(
                            "settings-battery-low",
                            "Low at",
                            &percent_choices(&LOW_CHOICES),
                            battery.low,
                            read_only,
                            |settings, low| settings.battery.low = low,
                            cx,
                        ))
                        .child(Self::choices(
                            "settings-battery-critical",
                            "Critical at",
                            &percent_choices(&CRITICAL_CHOICES),
                            battery.critical,
                            read_only,
                            |settings, critical| settings.battery.critical = critical,
                            cx,
                        )),
                ),
            )
            .child(
                GroupBox::new().outline().title("DPI popup").child(
                    v_flex()
                        .gap_3()
                        .child(
                            Switch::new("settings-dpi-popup")
                                .label("Show the DPI when a mouse switches stage")
                                .checked(dpi_popup.enabled)
                                .disabled(read_only)
                                .on_click(cx.listener(|this, checked: &bool, window, cx| {
                                    let checked = *checked;
                                    this.apply(window, cx, |settings| {
                                        settings.dpi_popup.enabled = checked;
                                    });
                                })),
                        )
                        .child(Self::choices(
                            "settings-dpi-duration",
                            "Show for",
                            &durations,
                            dpi_popup.duration_ms,
                            read_only,
                            |settings, duration_ms| settings.dpi_popup.duration_ms = duration_ms,
                            cx,
                        )),
                ),
            )
    }
}

/// Opens the settings dialog.
pub fn open(window: &mut Window, cx: &mut App) {
    let view = cx.new(SettingsView::new);
    window.open_dialog(cx, move |dialog, _, _| {
        dialog.title("Settings").width(px(480.)).child(view.clone())
    });
}

#[cfg(test)]
mod tests {
    use gpui_kit::prelude::*;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{AnyWindowHandle, Context, TestAppContext, Window, WindowOptions, div};

    use super::open;
    use crate::paths::ConfigPaths;
    use crate::settings::{GlobalSettings, Settings};

    struct Blank;

    impl Render for Blank {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full()
        }
    }

    fn start(cx: &mut TestAppContext, settings: &str) -> (tempfile::TempDir, AnyWindowHandle) {
        let dir = tempfile::tempdir().unwrap();
        let paths = ConfigPaths::at(dir.path().to_path_buf());
        std::fs::write(paths.settings_file(), settings).unwrap();
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_reduce_motion(true);
            crate::settings::init(cx, &paths);
        });
        let (window, _) = cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| Blank)).unwrap()
        });
        cx.update_window(window, |_, window, cx| {
            open(window, cx);
            window.render_frame(cx);
        })
        .unwrap();
        (dir, window)
    }

    fn click(cx: &mut TestAppContext, window: AnyWindowHandle, id: impl Into<gpui_kit::ElementId>) {
        let id = id.into();
        cx.update_window(window, |_, window, cx| {
            window.within("dialog").click(id, cx);
        })
        .unwrap();
        cx.run_until_parked();
    }

    fn saved(cx: &mut TestAppContext) -> Settings {
        cx.update(|cx| cx.global::<GlobalSettings>().0.read(cx).settings().clone())
    }

    #[gpui_kit::test]
    fn choosing_a_threshold_and_disabling_the_popup_persist(cx: &mut TestAppContext) {
        // Given the settings dialog with the defaults
        let (dir, window) = start(cx, "{}");
        // When choosing a 30% low threshold and turning the DPI popup off
        click(cx, window, ("settings-battery-low", 4usize));
        click(cx, window, "settings-dpi-popup");
        // Then the settings and the file hold both changes
        let settings = saved(cx);
        assert_eq!(settings.battery.low, 30);
        assert!(!settings.dpi_popup.enabled);
        let text = std::fs::read_to_string(dir.path().join("settings.json")).unwrap();
        assert!(text.contains("\"low\": 30"));
        assert!(text.contains("\"enabled\": false"));
    }

    #[gpui_kit::test]
    fn read_only_settings_disable_the_controls(cx: &mut TestAppContext) {
        // Given a settings file from a newer version
        let (_dir, window) = start(cx, r#"{"version": 99}"#);
        let before = saved(cx);
        // When trying to change the notifications and the duration
        click(cx, window, "settings-battery-notifications");
        click(cx, window, ("settings-dpi-duration", 3usize));
        // Then an explanation is shown and nothing changed
        assert_eq!(saved(cx), before);
        assert!(
            cx.update_window(window, |_, window, _| {
                window.try_find("settings-read-only").is_some()
            })
            .unwrap()
        );
    }
}
