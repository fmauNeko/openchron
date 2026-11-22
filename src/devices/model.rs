//! Device state, as the workers report it to the app.

use std::sync::{Arc, mpsc};

pub use crate::protocol::Battery;
pub use crate::protocol::mouse::{DpiStages, PollingRates};

/// Identifies a device by the path of the HID interface it is reached through.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DeviceId(pub String);

/// How a device is connected to the computer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Connection {
    /// USB cable.
    Wired,
    /// 2.4 GHz receiver, named.
    Receiver(String),
    /// Bluetooth.
    Bluetooth,
}

/// The active profile and the number of profiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Profiles {
    /// Active profile, 0-based.
    pub active: u8,
    /// Number of profiles.
    pub count: u8,
}

/// Mouse settings and battery; `None` for what the mouse does not report.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MouseState {
    /// Battery reading.
    pub battery: Option<Battery>,
    /// DPI stages.
    pub dpi: Option<DpiStages>,
    /// Polling rates of the current connection.
    pub polling: Option<PollingRates>,
    /// Onboard profiles.
    pub profiles: Option<Profiles>,
}

/// Keyboard information.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct KeyboardState {
    /// VIA protocol version.
    pub via_protocol: Option<u16>,
}

/// The kind of device, with its kind-specific state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Details {
    /// A mouse.
    Mouse(MouseState),
    /// A keyboard.
    Keyboard(KeyboardState),
}

/// Everything known about a device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceState {
    /// Display name.
    pub name: String,
    /// USB product ID of the device itself, also when it is reached through a receiver.
    pub product_id: u16,
    /// How it is connected.
    pub connection: Connection,
    /// Firmware version.
    pub firmware: Option<String>,
    /// False while the device cannot be reached, such as a mouse asleep or out of its
    /// receiver's range.
    pub online: bool,
    /// Kind-specific state.
    pub details: Details,
}

impl DeviceState {
    /// The mouse state of a mouse.
    #[must_use]
    pub const fn mouse(&self) -> Option<&MouseState> {
        match &self.details {
            Details::Mouse(mouse) => Some(mouse),
            Details::Keyboard(_) => None,
        }
    }
}

/// A request to a device worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Makes this DPI stage (0-based) the active one.
    SelectDpiStage(usize),
    /// Makes this polling rate (0-based index) the active one.
    SelectPollingRate(usize),
    /// Makes this profile (0-based) the active one.
    SelectProfile(u8),
    /// Reads the device state again.
    Refresh,
}

/// A message from the HID threads to the app.
#[derive(Debug)]
pub enum DeviceUpdate {
    /// A worker started on an interface; `commands` reaches it.
    Attached {
        /// The device.
        id: DeviceId,
        /// Display name until the device reports its state.
        name: String,
        /// Channel to the worker.
        commands: mpsc::Sender<Command>,
    },
    /// The device state changed.
    State {
        /// The device.
        id: DeviceId,
        /// Its new state.
        state: DeviceState,
    },
    /// A command could not be applied.
    CommandFailed {
        /// The device.
        id: DeviceId,
        /// What went wrong.
        error: String,
    },
    /// The interface exists but could not be opened, for example for lack of permission.
    Unavailable {
        /// The device.
        id: DeviceId,
        /// Display name.
        name: String,
        /// What went wrong.
        error: String,
    },
    /// The interface is gone.
    Detached {
        /// The device.
        id: DeviceId,
    },
    /// The platform HID stack is unavailable.
    BackendFailed(String),
}

/// Where the HID threads send their updates.
pub type UpdateSink = Arc<dyn Fn(DeviceUpdate) + Send + Sync>;
