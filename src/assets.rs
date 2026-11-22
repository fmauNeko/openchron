//! Application asset sources: the Openchron icon set layered over the GPUI Kit defaults.

use std::borrow::Cow;

use gpui_kit::assets::Assets;
use gpui_kit::{AssetSource, Result, SharedString};

gpui_kit::assets::icon_assets!(pub OpenchronIcons, [Bluetooth, Keyboard, Mouse, Radio, Usb]);

/// Application asset source: the extra Openchron icons first, then the GPUI Kit defaults.
pub struct AppAssets;

impl AssetSource for AppAssets {
    /// Loads `path` from the extra Openchron icons, then from the default bundle.
    ///
    /// # Errors
    ///
    /// Returns an error only when an underlying source fails for a reason other
    /// than a missing asset; an unknown path resolves to `Ok(None)`.
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(bytes) = OpenchronIcons.load(path)? {
            return Ok(Some(bytes));
        }
        // The default bundle reports a missing asset as an error; this merged
        // source reports it as `Ok(None)`, like `OpenchronIcons` does.
        Ok(Assets.load(path).ok().flatten())
    }

    /// Lists the assets under `path` from both sources, sorted and deduplicated.
    ///
    /// # Errors
    ///
    /// Returns an error when one of the underlying sources fails to list.
    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = Assets.list(path)?;
        paths.extend(OpenchronIcons.list(path)?);
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::assets::IconName;
    use gpui_kit::{AssetSource as _, SharedString};

    use super::AppAssets;

    #[test]
    fn application_icons_resolve() {
        // Given every icon used by the application's views
        let icons = [
            IconName::Battery,
            IconName::BatteryCharging,
            IconName::BatteryWarning,
            IconName::Bluetooth,
            IconName::CircleAlert,
            IconName::Info,
            IconName::Keyboard,
            IconName::Mouse,
            IconName::Radio,
            IconName::Settings,
            IconName::Usb,
        ];
        // When loading the paths through the production asset source
        let missing: Vec<_> = icons
            .into_iter()
            .map(IconName::path)
            .filter(|path| AppAssets.load(path).unwrap().is_none())
            .collect();
        // Then no view renders a missing glyph
        assert_eq!(missing, Vec::<SharedString>::new());
    }

    #[test]
    fn unknown_path_is_none() {
        // Given the application asset source
        // When an unknown icon path is loaded
        let unknown = AppAssets.load("icons/does-not-exist.svg");
        // Then nothing is found
        assert!(matches!(unknown, Ok(None)));
    }

    #[test]
    fn list_contains_both_sources_once() {
        // Given the application asset source
        // When the icons directory is listed
        let paths = AppAssets.list("icons").expect("the asset list is readable");
        // Then an extra icon and a default icon each appear exactly once
        let count = |name: &str| paths.iter().filter(|path| path.as_str() == name).count();
        assert_eq!(count("icons/mouse.svg"), 1);
        assert_eq!(count("icons/settings.svg"), 1);
    }
}
