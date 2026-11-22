//! HID access: discovery of Keychron interfaces and report transport.
//!
//! [`Backend`] and [`Transport`] hide `hidapi` ([`NativeBackend`]), so the device workers
//! run against the scripted `fake` backend in tests.

#[cfg(test)]
pub mod fake;
mod native;

use std::time::Duration;

use thiserror::Error;

use crate::protocol::{mouse, via};

pub use native::NativeBackend;

/// USB vendor ID of Keychron devices.
pub const KEYCHRON_VENDOR_ID: u16 = 0x3434;

/// A HID interface of a Keychron device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterfaceInfo {
    /// Platform path, unique among the connected interfaces.
    pub path: String,
    /// USB vendor ID.
    pub vendor_id: u16,
    /// USB product ID.
    pub product_id: u16,
    /// HID usage page of the interface's top-level collection.
    pub usage_page: u16,
    /// HID usage of the interface's top-level collection.
    pub usage: u16,
    /// USB product string.
    pub product: Option<String>,
}

/// The protocol an interface speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    /// Keychron mouse vendor protocol, also spoken by the mouse receivers.
    Mouse,
    /// VIA raw HID, spoken by QMK keyboards.
    Via,
}

impl InterfaceInfo {
    /// The protocol this interface speaks; `None` for interfaces Openchron does not use.
    #[must_use]
    pub fn protocol(&self) -> Option<Protocol> {
        if self.vendor_id != KEYCHRON_VENDOR_ID {
            return None;
        }
        match (self.usage_page, self.usage) {
            (mouse::USAGE_PAGE, _) => Some(Protocol::Mouse),
            (via::USAGE_PAGE, via::USAGE) => Some(Protocol::Via),
            _ => None,
        }
    }
}

/// A failure reported by the platform HID stack.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{0}")]
pub struct HidError(pub String);

/// An open HID interface.
pub trait Transport: Send {
    /// Writes one report: the report ID, then the payload.
    ///
    /// # Errors
    /// Fails when the device is gone or rejects the report.
    fn write(&mut self, report: &[u8]) -> Result<(), HidError>;

    /// Reads one input report into `buf`, waiting at most `timeout`. Returns the report
    /// length, or 0 when no report arrived in time.
    ///
    /// # Errors
    /// Fails when the device is gone.
    fn read(&mut self, buf: &mut [u8], timeout: Duration) -> Result<usize, HidError>;
}

/// Lists and opens HID interfaces.
pub trait Backend {
    /// Lists the connected Keychron interfaces.
    ///
    /// # Errors
    /// Fails when the platform cannot enumerate HID devices.
    fn enumerate(&mut self) -> Result<Vec<InterfaceInfo>, HidError>;

    /// Opens an interface listed by [`Backend::enumerate`].
    ///
    /// # Errors
    /// Fails when the interface is gone or access is denied.
    fn open(&mut self, interface: &InterfaceInfo) -> Result<Box<dyn Transport>, HidError>;
}

/// Creates a backend on the thread that uses it: `hidapi` stays on one thread.
pub type BackendFactory = Box<dyn FnOnce() -> Result<Box<dyn Backend>, HidError> + Send>;

#[cfg(test)]
mod tests {
    use super::{InterfaceInfo, KEYCHRON_VENDOR_ID, Protocol};

    fn interface(vendor_id: u16, usage_page: u16, usage: u16) -> InterfaceInfo {
        InterfaceInfo {
            path: "path".into(),
            vendor_id,
            product_id: 0xD05A,
            usage_page,
            usage,
            product: None,
        }
    }

    #[test]
    fn protocol_follows_the_usage_page() {
        // Given Keychron interfaces on the mouse, raw HID and mouse input pages, and another vendor
        // When asking for their protocol
        // Then only the vendor pages of Keychron devices are used
        assert_eq!(
            interface(KEYCHRON_VENDOR_ID, 0xFFC1, 0x01).protocol(),
            Some(Protocol::Mouse)
        );
        assert_eq!(
            interface(KEYCHRON_VENDOR_ID, 0xFF60, 0x61).protocol(),
            Some(Protocol::Via)
        );
        assert_eq!(interface(KEYCHRON_VENDOR_ID, 0xFF60, 0x62).protocol(), None);
        assert_eq!(interface(KEYCHRON_VENDOR_ID, 0x0001, 0x02).protocol(), None);
        assert_eq!(interface(0x046D, 0xFFC1, 0x01).protocol(), None);
    }
}
