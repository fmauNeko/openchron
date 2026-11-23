//! DPI popup: an on-screen display that shows the new DPI when a mouse switches stage.
//!
//! The popup is a borderless, always-on-top window that never takes focus, at the bottom
//! center of the primary display. It is drawn without GPUI Kit's `Root` so its window can
//! stay transparent around the rounded card. A new change while it shows updates it and
//! restarts its timer.

use std::time::Duration;

use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Bounds, Context, FontWeight, Global, Pixels, Render, SharedString, Size, Task,
    TestSupportExt as _, Window, WindowBackgroundAppearance, WindowBounds, WindowHandle,
    WindowKind, WindowOptions, div, point, px, size,
};

use crate::app::APP_ID;
use crate::devices::store::{DeviceStoreEvent, GlobalDevices};
use crate::settings;

/// Distance between the popup and the bottom of the display's usable area.
const BOTTOM_MARGIN: f32 = 96.;

/// What the popup shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DpiPopup {
    /// Device name.
    pub device: SharedString,
    /// DPI of the active stage.
    pub dpi: u16,
    /// Active stage, 0-based.
    pub stage: usize,
    /// Number of stages.
    pub stages: usize,
}

/// The popup window, while it shows, and the task that hides it.
#[derive(Default)]
struct Osd {
    window: Option<WindowHandle<PopupView>>,
    hide: Option<Task<()>>,
}

impl Global for Osd {}

/// Shows the popup when a mouse switches DPI stage, if enabled in the settings.
pub fn init(cx: &mut App) {
    cx.set_global(Osd::default());
    let store = cx.global::<GlobalDevices>().0.clone();
    cx.subscribe(&store, |_, event: &DeviceStoreEvent, cx| match event {
        DeviceStoreEvent::DpiChanged { name, stages, .. } => {
            let popup_settings = settings::current(cx).dpi_popup;
            if let (true, Some(dpi)) = (popup_settings.enabled, stages.active_value()) {
                let popup = DpiPopup {
                    device: name.clone().into(),
                    dpi,
                    stage: stages.active,
                    stages: stages.values.len(),
                };
                show(cx, popup, popup_settings.duration());
            }
        }
        DeviceStoreEvent::Changed
        | DeviceStoreEvent::BatteryChanged { .. }
        | DeviceStoreEvent::CommandFailed { .. } => {}
    })
    .detach();
}

/// Shows `popup` for `duration`, reusing the popup window if it is already showing.
pub fn show(cx: &mut App, popup: DpiPopup, duration: Duration) {
    let shown = cx.global::<Osd>().window.and_then(|handle| {
        let content = popup.clone();
        handle
            .update(cx, |view, _, cx| {
                view.popup = content;
                cx.notify();
            })
            .ok()
            .map(|()| handle)
    });
    let window = match shown {
        Some(handle) => handle,
        None => match open(cx, popup) {
            Ok(handle) => handle,
            Err(error) => {
                eprintln!("Openchron could not show the DPI popup: {error:#}");
                return;
            }
        },
    };
    let hide = cx.spawn(async move |cx| {
        cx.background_executor().timer(duration).await;
        cx.update(close);
    });
    let osd = cx.global_mut::<Osd>();
    osd.window = Some(window);
    // Replacing the task cancels the previous timer.
    osd.hide = Some(hide);
}

/// The popup currently shown, if any.
#[must_use]
pub fn current(cx: &App) -> Option<DpiPopup> {
    let handle = cx.global::<Osd>().window?;
    handle.read(cx).ok().map(|view| view.popup.clone())
}

fn close(cx: &mut App) {
    if let Some(handle) = cx.global_mut::<Osd>().window.take() {
        // The platform may already have destroyed the window.
        drop(handle.update(cx, |_, window, _| window.remove_window()));
    }
}

fn open(cx: &mut App, popup: DpiPopup) -> gpui_kit::Result<WindowHandle<PopupView>> {
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(popup_bounds(cx))),
        titlebar: None,
        focus: false,
        show: true,
        kind: WindowKind::PopUp,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        window_background: WindowBackgroundAppearance::Transparent,
        app_id: Some(APP_ID.to_owned()),
        ..WindowOptions::default()
    };
    cx.open_window(options, |_, cx| cx.new(|_| PopupView { popup }))
}

fn popup_size() -> Size<Pixels> {
    size(px(240.), px(128.))
}

/// Bottom center of the primary display's usable area.
fn popup_bounds(cx: &App) -> Bounds<Pixels> {
    let size = popup_size();
    cx.primary_display().map_or_else(
        || Bounds::centered(None, size, cx),
        |display| {
            let area = display.visible_bounds();
            Bounds::new(
                point(
                    area.center().x - size.width / 2.,
                    area.bottom() - size.height - px(BOTTOM_MARGIN),
                ),
                size,
            )
        },
    )
}

/// The popup content: device, DPI and a dot per stage.
pub struct PopupView {
    popup: DpiPopup,
}

impl Render for PopupView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let DpiPopup {
            device,
            dpi,
            stage,
            stages,
        } = self.popup.clone();
        // The card fills the window: Windows 11 rounds and outlines popup windows itself,
        // with the same 8 px radius.
        div()
            .id("dpi-popup")
            .test_support()
            .size_full()
            .font_family(theme.font_family.clone())
            .child(
                v_flex()
                    .size_full()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.popover)
                    .text_color(theme.popover_foreground)
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(device),
                    )
                    .child(
                        h_flex()
                            .items_baseline()
                            .gap_1()
                            .child(
                                div()
                                    .text_3xl()
                                    .font_weight(FontWeight::BOLD)
                                    .child(dpi.to_string()),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(theme.muted_foreground)
                                    .child("DPI"),
                            ),
                    )
                    .child(h_flex().gap_1p5().children((0..stages).map(|index| {
                        div().size(px(8.)).rounded_full().bg(if index == stage {
                            theme.primary
                        } else {
                            theme.muted
                        })
                    }))),
            )
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use gpui_kit::TestAppContext;

    use super::{DpiPopup, current};
    use crate::devices::model::{DeviceId, DeviceUpdate};
    use crate::devices::store::tests::{apply, mouse};
    use crate::paths::ConfigPaths;

    fn start(cx: &mut TestAppContext, settings: Option<&[u8]>) -> tempfile::TempDir {
        let directory = tempfile::tempdir().unwrap();
        if let Some(bytes) = settings {
            std::fs::write(directory.path().join("settings.json"), bytes).unwrap();
        }
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::settings::init(cx, &ConfigPaths::at(directory.path().to_path_buf()));
            crate::devices::store::init(cx, None);
            super::init(cx);
        });
        directory
    }

    fn report_stage(cx: &mut TestAppContext, stage: usize) {
        cx.update(|cx| {
            apply(
                cx,
                DeviceUpdate::State {
                    id: DeviceId("turbolink".into()),
                    state: mouse(59, stage),
                },
            );
        });
    }

    fn wait(cx: &mut TestAppContext, millis: u64) {
        cx.executor().advance_clock(Duration::from_millis(millis));
        cx.run_until_parked();
    }

    #[gpui_kit::test]
    fn stage_switch_shows_the_popup_then_hides_it(cx: &mut TestAppContext) {
        // Given a mouse known at stage 2 and the default 1.5 s popup
        let _directory = start(cx, None);
        report_stage(cx, 2);
        assert!(cx.update(|cx| current(cx)).is_none());
        // When the mouse switches to stage 3
        report_stage(cx, 3);
        // Then the popup shows the new DPI in its own window
        assert_eq!(
            cx.update(|cx| current(cx)),
            Some(DpiPopup {
                device: "Keychron G6 HE".into(),
                dpi: 3200,
                stage: 3,
                stages: 5
            })
        );
        assert_eq!(cx.windows().len(), 1);
        // And it closes once its time is up
        wait(cx, 1600);
        assert!(cx.update(|cx| current(cx)).is_none());
        assert_eq!(cx.windows().len(), 0);
    }

    #[gpui_kit::test]
    fn new_switch_reuses_the_popup_and_restarts_its_timer(cx: &mut TestAppContext) {
        // Given a popup shown for a switch to stage 3
        let _directory = start(cx, None);
        report_stage(cx, 2);
        report_stage(cx, 3);
        wait(cx, 1000);
        // When the mouse switches to stage 4 before the popup hides
        report_stage(cx, 4);
        wait(cx, 1000);
        // Then the same window shows the new DPI, still visible after the first deadline
        assert_eq!(cx.windows().len(), 1);
        assert_eq!(
            cx.update(|cx| current(cx)).map(|popup| popup.dpi),
            Some(5000)
        );
        wait(cx, 600);
        assert_eq!(cx.windows().len(), 0);
    }

    #[gpui_kit::test]
    fn disabled_popup_stays_hidden(cx: &mut TestAppContext) {
        // Given the popup disabled in the settings
        let _directory = start(cx, Some(br#"{"version":1,"dpi_popup":{"enabled":false}}"#));
        report_stage(cx, 2);
        // When the mouse switches stage
        report_stage(cx, 3);
        // Then no window opens
        assert_eq!(cx.windows().len(), 0);
    }
}
