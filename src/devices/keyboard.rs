//! Keyboard session: a QMK keyboard reached through VIA raw HID.
//!
//! Only identification is implemented: the keymap, lighting and Keychron extensions come
//! later (see `AGENTS.md`).

use std::time::Duration;

use crate::hid::{InterfaceInfo, Transport};
use crate::protocol::via;

use super::catalog;
use super::model::{Command, Connection, Details, DeviceState, KeyboardState};
use super::session::{SessionError, exchange};

/// A session with a VIA raw HID interface (usage page `0xFF60`).
pub struct KeyboardSession {
    transport: Box<dyn Transport>,
    state: DeviceState,
}

impl KeyboardSession {
    /// Starts a session; nothing is read until [`KeyboardSession::refresh`].
    #[must_use]
    pub fn new(transport: Box<dyn Transport>, interface: &InterfaceInfo) -> Self {
        let name = catalog::name(
            interface.product_id,
            interface.product.as_deref(),
            "keyboard",
        );
        let connection = if catalog::is_receiver(interface.product_id) {
            Connection::Receiver(name.clone())
        } else {
            Connection::Wired
        };
        Self {
            transport,
            state: DeviceState {
                name,
                product_id: interface.product_id,
                connection,
                firmware: None,
                online: false,
                details: Details::Keyboard(KeyboardState::default()),
            },
        }
    }

    /// What is known about the keyboard.
    #[must_use]
    pub const fn state(&self) -> &DeviceState {
        &self.state
    }

    /// Reads the VIA protocol version.
    ///
    /// # Errors
    /// See [`SessionError`].
    pub fn refresh(&mut self) -> Result<(), SessionError> {
        let result = exchange(
            self.transport.as_mut(),
            &via::protocol_version_request(),
            false,
            |reply| via::parse_protocol_version(reply).is_ok(),
            |_| {},
        )
        .and_then(|reply| Ok(via::parse_protocol_version(&reply)?));
        self.state.online = result.is_ok();
        let version = result?;
        self.state.details = Details::Keyboard(KeyboardState {
            via_protocol: Some(version),
        });
        Ok(())
    }

    /// Waits up to `timeout` for a report; keyboards send nothing Openchron uses yet.
    ///
    /// # Errors
    /// Fails with [`SessionError::Hid`] when the keyboard is gone.
    pub fn poll(&mut self, timeout: Duration) -> Result<bool, SessionError> {
        let mut buf = [0; 33];
        self.transport.read(&mut buf, timeout)?;
        Ok(false)
    }

    /// Applies a command; keyboards only support [`Command::Refresh`] so far.
    ///
    /// # Errors
    /// [`SessionError::Unsupported`] for mouse settings, else see [`KeyboardSession::refresh`].
    pub fn execute(&mut self, command: Command) -> Result<(), SessionError> {
        match command {
            Command::Refresh => self.refresh(),
            Command::SelectDpiStage(_)
            | Command::SelectPollingRate(_)
            | Command::SelectProfile(_) => Err(SessionError::Unsupported),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::KeyboardSession;
    use crate::devices::model::{Command, Details, KeyboardState};
    use crate::devices::session::SessionError;
    use crate::hid::InterfaceInfo;
    use crate::hid::fake::FakeDevice;

    fn keyboard() -> (InterfaceInfo, FakeDevice) {
        let device = FakeDevice::default();
        let mut reply = vec![0; 32];
        reply[..3].copy_from_slice(&[0x01, 0x00, 0x0C]);
        device.reply(0x01, [reply]);
        let interface = InterfaceInfo {
            path: "keyboard".into(),
            vendor_id: 0x3434,
            product_id: 0x0A30,
            usage_page: 0xFF60,
            usage: 0x61,
            product: Some("Keychron V3 Max".into()),
        };
        (interface, device)
    }

    #[test]
    fn refresh_reads_the_via_protocol_version() {
        // Given a VIA keyboard
        let (interface, device) = keyboard();
        let mut session = KeyboardSession::new(device.transport(), &interface);
        // When refreshing
        session.refresh().unwrap();
        // Then it is online, named after its USB product string, with its VIA version
        let state = session.state();
        assert!(state.online);
        assert_eq!(state.name, "Keychron V3 Max");
        assert_eq!(
            state.details,
            Details::Keyboard(KeyboardState {
                via_protocol: Some(12)
            })
        );
        assert_eq!(&device.written()[0][..2], &[0x00, 0x01]);
    }

    #[test]
    fn mouse_commands_are_unsupported() {
        // Given a keyboard session
        let (interface, device) = keyboard();
        let mut session = KeyboardSession::new(device.transport(), &interface);
        // When asking for a DPI stage
        // Then it is refused without writing anything
        assert!(matches!(
            session.execute(Command::SelectDpiStage(1)),
            Err(SessionError::Unsupported)
        ));
        assert_eq!(device.written(), Vec::<Vec<u8>>::new());
    }
}
