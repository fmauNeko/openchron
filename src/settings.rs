//! Settings model and store.

use std::io;
use std::path::PathBuf;
use std::time::Duration;

use gpui_kit::prelude::*;
use gpui_kit::{App, Context, Entity, EventEmitter, Global};
use serde::{Deserialize, Serialize};

use crate::paths::ConfigPaths;
use crate::persist::{self, LoadOutcome, Versioned};

/// Low-battery notifications.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BatterySettings {
    /// Whether to notify when a battery runs low.
    pub notifications: bool,
    /// Battery level, in percent, that triggers the low-battery notification.
    pub low: u8,
    /// Battery level, in percent, that triggers the critical notification.
    pub critical: u8,
}

impl Default for BatterySettings {
    fn default() -> Self {
        Self {
            notifications: true,
            low: 20,
            critical: 10,
        }
    }
}

/// The popup showing the DPI when a mouse switches DPI stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DpiPopupSettings {
    /// Whether to show the popup.
    pub enabled: bool,
    /// How long the popup stays, in milliseconds.
    pub duration_ms: u32,
}

impl DpiPopupSettings {
    /// The display duration, kept between half a second and ten seconds.
    #[must_use]
    pub fn duration(self) -> Duration {
        Duration::from_millis(u64::from(self.duration_ms.clamp(500, 10_000)))
    }
}

impl Default for DpiPopupSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            duration_ms: 1500,
        }
    }
}

/// User-configurable application settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub version: u32,
    pub battery: BatterySettings,
    pub dpi_popup: DpiPopupSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: Self::CURRENT_VERSION,
            battery: BatterySettings::default(),
            dpi_popup: DpiPopupSettings::default(),
        }
    }
}

impl Versioned for Settings {
    const CURRENT_VERSION: u32 = 1;
}

/// Change notification emitted by [`SettingsStore`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsEvent {
    Changed,
}

impl EventEmitter<SettingsEvent> for SettingsStore {}

/// Deferred information about a settings file that could not be used normally.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadNotice {
    Corrupt { backup: PathBuf },
    NewerVersion,
}

/// Failures returned when updating settings.
#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("settings are read-only because the file uses a newer version")]
    ReadOnly,
    #[error("could not save settings: {0}")]
    Io(#[from] io::Error),
}

/// GPUI entity holding settings and persistence state.
pub struct SettingsStore {
    file: PathBuf,
    settings: Settings,
    read_only: bool,
    load_notice: Option<LoadNotice>,
}

/// GPUI global handle to the settings store.
pub struct GlobalSettings(pub Entity<SettingsStore>);

impl Global for GlobalSettings {}

impl SettingsStore {
    /// Returns the current settings.
    #[must_use]
    pub const fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Returns whether a newer file version prevents writes.
    #[must_use]
    pub const fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// Takes the pending load notice, if any.
    #[must_use]
    pub fn take_load_notice(&mut self) -> Option<LoadNotice> {
        self.load_notice.take()
    }

    /// Updates settings, saves changes and emits a change event.
    ///
    /// # Errors
    /// Returns [`SettingsError::ReadOnly`] for files from a newer version, or
    /// [`SettingsError::Io`] when persistence fails.
    pub fn update(
        &mut self,
        cx: &mut Context<Self>,
        update: impl FnOnce(&mut Settings),
    ) -> Result<(), SettingsError> {
        if self.read_only {
            return Err(SettingsError::ReadOnly);
        }
        let previous = self.settings.clone();
        update(&mut self.settings);
        if previous == self.settings {
            return Ok(());
        }
        if let Err(error) = persist::save_json(&self.file, &self.settings) {
            self.settings = previous;
            return Err(error.into());
        }
        cx.emit(SettingsEvent::Changed);
        cx.notify();
        Ok(())
    }
}

/// The current settings.
#[must_use]
pub fn current(cx: &App) -> &Settings {
    cx.global::<GlobalSettings>().0.read(cx).settings()
}

/// Loads settings and installs the store as a GPUI global.
pub fn init(cx: &mut App, paths: &ConfigPaths) {
    let file = paths.settings_file();
    let (settings, read_only, load_notice) = match persist::load_json::<Settings>(&file) {
        Ok(LoadOutcome::Missing) | Err(_) => (Settings::default(), false, None),
        Ok(LoadOutcome::Loaded(settings)) => (settings, false, None),
        Ok(LoadOutcome::Corrupt { backup, .. }) => (
            Settings::default(),
            false,
            Some(LoadNotice::Corrupt { backup }),
        ),
        Ok(LoadOutcome::NewerVersion { .. }) => {
            (Settings::default(), true, Some(LoadNotice::NewerVersion))
        }
    };
    let entity = cx.new(|_| SettingsStore {
        file,
        settings,
        read_only,
        load_notice,
    });
    cx.set_global(GlobalSettings(entity));
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::Duration;

    use gpui_kit::TestAppContext;

    use crate::paths::ConfigPaths;

    use super::{DpiPopupSettings, GlobalSettings, LoadNotice, Settings, SettingsError, init};

    fn start(cx: &TestAppContext, file: Option<&[u8]>) -> tempfile::TempDir {
        let directory = tempfile::tempdir().expect("temporary directory is created");
        if let Some(bytes) = file {
            fs::write(directory.path().join("settings.json"), bytes).expect("file is written");
        }
        let paths = ConfigPaths::at(directory.path().to_path_buf());
        cx.update(|cx| init(cx, &paths));
        directory
    }

    fn read(cx: &TestAppContext) -> Settings {
        cx.update(|cx| super::current(cx).clone())
    }

    #[gpui_kit::test]
    fn defaults_when_file_is_missing(cx: &mut TestAppContext) {
        // Given a settings store with no existing file
        let directory = start(cx, None);
        // When reading its settings
        // Then defaults are available without creating the file
        assert_eq!(read(cx), Settings::default());
        assert!(!directory.path().join("settings.json").exists());
    }

    #[gpui_kit::test]
    fn update_persists_and_emits(cx: &mut TestAppContext) {
        // Given a missing settings file and an event subscriber
        let directory = start(cx, None);
        let changes = std::rc::Rc::new(std::cell::Cell::new(0));
        let count = changes.clone();
        let entity = cx.update(|cx| {
            let entity = cx.global::<GlobalSettings>().0.clone();
            cx.subscribe(&entity, move |_, _: &super::SettingsEvent, _| {
                count.set(count.get() + 1);
            })
            .detach();
            entity
        });
        // When lowering the low-battery threshold, then writing the same value again
        for _ in 0..2 {
            cx.update(|cx| entity.update(cx, |store, cx| store.update(cx, |s| s.battery.low = 15)))
                .expect("settings update succeeds");
        }
        // Then the document is saved once and one change is reported
        let saved = fs::read_to_string(directory.path().join("settings.json")).unwrap();
        assert!(saved.contains("\"low\": 15"));
        assert_eq!(changes.get(), 1);
    }

    #[gpui_kit::test]
    fn partial_file_keeps_the_other_defaults(cx: &mut TestAppContext) {
        // Given a file that only disables the DPI popup
        start(cx, Some(br#"{"version":1,"dpi_popup":{"enabled":false}}"#));
        // When reading the settings
        let settings = read(cx);
        // Then the rest uses the defaults
        assert!(!settings.dpi_popup.enabled);
        assert_eq!(settings.dpi_popup.duration_ms, 1500);
        assert_eq!(settings.battery, super::BatterySettings::default());
    }

    #[gpui_kit::test]
    fn corrupt_file_is_backed_up_and_noticed(cx: &mut TestAppContext) {
        // Given a corrupt settings document
        let _directory = start(cx, Some(b"{ invalid"));
        // When the load notice is taken
        let notice = cx.update(|cx| {
            let entity = cx.global::<GlobalSettings>().0.clone();
            entity.update(cx, |store, _| store.take_load_notice())
        });
        // Then defaults are available and the original bytes are preserved in a backup
        let Some(LoadNotice::Corrupt { backup }) = notice else {
            panic!("expected a corrupt-file notice");
        };
        assert_eq!(fs::read(backup).unwrap(), b"{ invalid");
        assert_eq!(read(cx), Settings::default());
    }

    #[gpui_kit::test]
    fn newer_version_is_read_only(cx: &mut TestAppContext) {
        // Given a settings document written by a newer version
        let original = br#"{"version":99,"future":true}"#;
        let directory = start(cx, Some(original));
        // When attempting an update
        let result = cx.update(|cx| {
            let entity = cx.global::<GlobalSettings>().0.clone();
            entity.update(cx, |store, cx| store.update(cx, |s| s.battery.low = 30))
        });
        // Then writes are rejected and the original bytes remain untouched
        assert!(matches!(result, Err(SettingsError::ReadOnly)));
        assert_eq!(
            fs::read(directory.path().join("settings.json")).unwrap(),
            original
        );
    }

    #[test]
    fn popup_duration_is_bounded() {
        // Given popup durations below, inside and above the allowed range
        let duration = |duration_ms| {
            DpiPopupSettings {
                enabled: true,
                duration_ms,
            }
            .duration()
        };
        // When converting them
        // Then they are kept between half a second and ten seconds
        assert_eq!(duration(0), Duration::from_millis(500));
        assert_eq!(duration(1500), Duration::from_millis(1500));
        assert_eq!(duration(60_000), Duration::from_secs(10));
    }
}
