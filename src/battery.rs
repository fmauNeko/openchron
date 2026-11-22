//! Low-battery notifications.
//!
//! Each device is notified once when it drops to the low threshold and once more at the
//! critical threshold. Charging, or climbing [`HYSTERESIS`] points above the low threshold,
//! re-arms both, so a level hovering around a threshold does not notify repeatedly.

use std::collections::HashMap;

use gpui_kit::{App, Global, SystemNotification};

use crate::devices::model::{Battery, DeviceId};
use crate::devices::store::{DeviceStoreEvent, GlobalDevices};
use crate::{notifications, settings};

/// Points above the low threshold a battery must reach to be notified again.
pub const HYSTERESIS: u8 = 5;

/// How low a battery is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    /// At or under the low threshold.
    Low,
    /// At or under the critical threshold.
    Critical,
}

/// Notification thresholds, in percent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Thresholds {
    /// Low threshold.
    pub low: u8,
    /// Critical threshold, at most the low one.
    pub critical: u8,
}

impl Thresholds {
    /// Thresholds from settings, made consistent: at most 100, critical at most low.
    #[must_use]
    pub fn new(low: u8, critical: u8) -> Self {
        let low = low.min(100);
        Self {
            low,
            critical: critical.min(low),
        }
    }
}

/// The outcome of a battery reading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Assessment {
    /// The highest level notified so far, to remember until the next reading.
    pub notified: Option<Level>,
    /// The level to notify now.
    pub notify: Option<Level>,
}

/// Decides whether `battery` deserves a notification, given the level already notified.
#[must_use]
pub fn assess(battery: Battery, notified: Option<Level>, thresholds: Thresholds) -> Assessment {
    if battery.charging {
        return Assessment {
            notified: None,
            notify: None,
        };
    }
    let level = if battery.percent <= thresholds.critical {
        Some(Level::Critical)
    } else if battery.percent <= thresholds.low {
        Some(Level::Low)
    } else {
        None
    };
    match level {
        Some(level) if notified.is_none_or(|notified| level > notified) => Assessment {
            notified: Some(level),
            notify: Some(level),
        },
        None if battery.percent >= thresholds.low.saturating_add(HYSTERESIS) => Assessment {
            notified: None,
            notify: None,
        },
        Some(_) | None => Assessment {
            notified,
            notify: None,
        },
    }
}

/// The notification for a battery at `level`.
#[must_use]
pub fn notification(
    id: &DeviceId,
    name: &str,
    battery: Battery,
    level: Level,
) -> SystemNotification {
    SystemNotification {
        tag: format!("battery:{}", id.0).into(),
        title: match level {
            Level::Low => "Low battery",
            Level::Critical => "Battery critically low",
        }
        .into(),
        body: format!("{name} is at {}%. Connect it to charge.", battery.percent).into(),
        actions: Vec::new(),
    }
}

/// Levels already notified, per device.
#[derive(Default)]
struct Notified(HashMap<DeviceId, Level>);

impl Global for Notified {}

/// Notifies low batteries reported by the device store.
pub fn init(cx: &mut App) {
    cx.set_global(Notified::default());
    let store = cx.global::<GlobalDevices>().0.clone();
    cx.subscribe(&store, |_, event: &DeviceStoreEvent, cx| match event {
        DeviceStoreEvent::BatteryChanged { id, name, battery } => {
            on_battery(cx, id, name, *battery);
        }
        DeviceStoreEvent::Changed
        | DeviceStoreEvent::DpiChanged { .. }
        | DeviceStoreEvent::CommandFailed { .. } => {}
    })
    .detach();
}

fn on_battery(cx: &mut App, id: &DeviceId, name: &str, battery: Battery) {
    let settings = settings::current(cx).battery;
    let thresholds = Thresholds::new(settings.low, settings.critical);
    let notified = cx.global::<Notified>().0.get(id).copied();
    let assessment = assess(battery, notified, thresholds);
    match assessment.notified {
        Some(level) => cx.global_mut::<Notified>().0.insert(id.clone(), level),
        None => cx.global_mut::<Notified>().0.remove(id),
    };
    if let Some(level) = assessment.notify
        && settings.notifications
    {
        notifications::show(cx, notification(id, name, battery, level));
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::TestAppContext;

    use super::{Assessment, Level, Thresholds, assess};
    use crate::devices::model::{DeviceId, DeviceUpdate};
    use crate::devices::store::tests::{apply, mouse};
    use crate::paths::ConfigPaths;
    use crate::protocol::Battery;

    const THRESHOLDS: Thresholds = Thresholds {
        low: 20,
        critical: 10,
    };

    fn discharging(percent: u8) -> Battery {
        Battery {
            percent,
            charging: false,
        }
    }

    /// Feeds readings through `assess` and returns the levels notified.
    fn notifications(readings: &[Battery]) -> Vec<Level> {
        let mut notified = None;
        let mut shown = Vec::new();
        for &battery in readings {
            let Assessment {
                notified: next,
                notify,
            } = assess(battery, notified, THRESHOLDS);
            notified = next;
            shown.extend(notify);
        }
        shown
    }

    #[test]
    fn draining_battery_notifies_low_then_critical_once() {
        // Given a battery draining from 25% to 5%
        let readings: Vec<_> = (5..=25).rev().map(discharging).collect();
        // When assessing each reading
        // Then it is notified once at the low threshold and once at the critical one
        assert_eq!(notifications(&readings), [Level::Low, Level::Critical]);
    }

    #[test]
    fn hovering_around_the_threshold_notifies_once() {
        // Given a battery oscillating between 19% and 22%
        let readings = [19, 22, 19, 21, 20].map(discharging);
        // When assessing each reading
        // Then only the first drop is notified
        assert_eq!(notifications(&readings), [Level::Low]);
    }

    #[test]
    fn charging_or_recovering_rearms_the_notification() {
        // Given a low battery that charges, then drains again; then one that recovers past
        // the hysteresis
        let charged = [
            discharging(18),
            Battery {
                percent: 18,
                charging: true,
            },
            discharging(17),
        ];
        let recovered = [discharging(18), discharging(25), discharging(19)];
        // When assessing each reading
        // Then each drop after re-arming is notified
        assert_eq!(notifications(&charged), [Level::Low, Level::Low]);
        assert_eq!(notifications(&recovered), [Level::Low, Level::Low]);
    }

    #[test]
    fn first_reading_under_the_threshold_notifies() {
        // Given a device found already at 8%
        // When assessing its first reading
        // Then the critical level is notified straight away
        assert_eq!(notifications(&[discharging(8)]), [Level::Critical]);
    }

    #[test]
    fn thresholds_are_made_consistent() {
        // Given settings with an out-of-range low threshold and a critical one above it
        // When building thresholds
        // Then both stay within 0..=100 and critical never exceeds low
        assert_eq!(
            Thresholds::new(150, 120),
            Thresholds {
                low: 100,
                critical: 100
            }
        );
        assert_eq!(
            Thresholds::new(15, 30),
            Thresholds {
                low: 15,
                critical: 15
            }
        );
    }

    #[gpui_kit::test]
    fn low_battery_reported_by_a_device_is_notified(cx: &mut TestAppContext) {
        // Given the device store, default settings and a recording notifier
        let directory = tempfile::tempdir().unwrap();
        let shown = cx.update(|cx| {
            crate::settings::init(cx, &ConfigPaths::at(directory.path().to_path_buf()));
            crate::devices::store::init(cx, None);
            let shown = crate::notifications::install_recorder(cx);
            super::init(cx);
            shown
        });
        let id = DeviceId("turbolink".into());
        // When the mouse reports 59%, then 15%, then 14%
        cx.update(|cx| {
            for percent in [59, 15, 14] {
                apply(
                    cx,
                    DeviceUpdate::State {
                        id: id.clone(),
                        state: mouse(percent, 2),
                    },
                );
            }
        });
        // Then exactly one low-battery notification names the mouse and its level
        let shown = shown.borrow();
        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0].tag.as_ref(), "battery:turbolink");
        assert!(shown[0].body.contains("Keychron G6 HE"));
        assert!(shown[0].body.contains("15%"));
    }

    #[gpui_kit::test]
    fn disabled_notifications_stay_silent(cx: &mut TestAppContext) {
        // Given notifications disabled in the settings
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("settings.json"),
            br#"{"version":1,"battery":{"notifications":false}}"#,
        )
        .unwrap();
        let shown = cx.update(|cx| {
            crate::settings::init(cx, &ConfigPaths::at(directory.path().to_path_buf()));
            crate::devices::store::init(cx, None);
            let shown = crate::notifications::install_recorder(cx);
            super::init(cx);
            shown
        });
        // When a mouse reports 5%
        cx.update(|cx| {
            apply(
                cx,
                DeviceUpdate::State {
                    id: DeviceId("turbolink".into()),
                    state: mouse(5, 2),
                },
            );
        });
        // Then nothing is shown
        assert!(shown.borrow().is_empty());
    }
}
