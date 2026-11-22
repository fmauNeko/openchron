//! A session with one HID interface, whatever protocol it speaks.

use std::time::{Duration, Instant};

use thiserror::Error;

use crate::hid::{HidError, InterfaceInfo, Protocol, Transport};
use crate::protocol::ProtocolError;

use super::keyboard::KeyboardSession;
use super::model::{Command, DeviceState};
use super::mouse::MouseSession;

/// Attempts per request, like Keychron Launcher.
const ATTEMPTS: usize = 3;
/// How long to wait for an answer before sending the request again.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(1);
/// Room for the longest input report and its report ID.
const BUFFER_LEN: usize = 65;

/// Why a session operation failed.
#[derive(Debug, Error)]
pub enum SessionError {
    /// The interface is gone; the session is over.
    #[error(transparent)]
    Hid(#[from] HidError),
    /// The device did not answer, for example because it is asleep.
    #[error("the device did not answer request 0x{0:02x}")]
    Timeout(u8),
    /// The device answered with a malformed reply.
    #[error(transparent)]
    Protocol(#[from] ProtocolError),
    /// The device refused the request.
    #[error("the device rejected request 0x{opcode:02x} (status {status})")]
    Rejected {
        /// The rejected request.
        opcode: u8,
        /// The status the device returned.
        status: u8,
    },
    /// The device does not support the requested setting or value.
    #[error("the device does not support this setting")]
    Unsupported,
    /// The device acknowledged the change but still reports the old value.
    #[error("the device did not apply the change")]
    NotApplied,
}

impl SessionError {
    /// Whether the interface is gone, which ends the session.
    #[must_use]
    pub const fn is_fatal(&self) -> bool {
        matches!(self, Self::Hid(_))
    }
}

/// A session with an interface, by protocol.
pub enum Session {
    /// A mouse, or a receiver relaying one.
    Mouse(MouseSession),
    /// A VIA keyboard.
    Keyboard(KeyboardSession),
}

impl Session {
    /// Starts a session on an open interface; nothing is read until [`Session::refresh`].
    #[must_use]
    pub fn open(
        protocol: Protocol,
        transport: Box<dyn Transport>,
        interface: &InterfaceInfo,
    ) -> Self {
        match protocol {
            Protocol::Mouse => Self::Mouse(MouseSession::new(transport, interface)),
            Protocol::Via => Self::Keyboard(KeyboardSession::new(transport, interface)),
        }
    }

    /// What is known about the device.
    #[must_use]
    pub const fn state(&self) -> &DeviceState {
        match self {
            Self::Mouse(session) => session.state(),
            Self::Keyboard(session) => session.state(),
        }
    }

    /// Reads the device state.
    ///
    /// # Errors
    /// See [`SessionError`]; only [`SessionError::Hid`] ends the session.
    pub fn refresh(&mut self) -> Result<(), SessionError> {
        match self {
            Self::Mouse(session) => session.refresh(),
            Self::Keyboard(session) => session.refresh(),
        }
    }

    /// Waits up to `timeout` for a report the device sends on its own and applies it.
    /// Returns whether the state changed.
    ///
    /// # Errors
    /// See [`SessionError`]; only [`SessionError::Hid`] ends the session.
    pub fn poll(&mut self, timeout: Duration) -> Result<bool, SessionError> {
        match self {
            Self::Mouse(session) => session.poll(timeout),
            Self::Keyboard(session) => session.poll(timeout),
        }
    }

    /// Applies a command, then reads the state back.
    ///
    /// # Errors
    /// See [`SessionError`]; only [`SessionError::Hid`] ends the session.
    pub fn execute(&mut self, command: Command) -> Result<(), SessionError> {
        match self {
            Self::Mouse(session) => session.execute(command),
            Self::Keyboard(session) => session.execute(command),
        }
    }
}

/// One request/answer exchange: writes `report` and waits for an input report accepted by
/// `answers`, sending the request again up to [`ATTEMPTS`] times. With `report_ids`, the
/// first byte of each input report is its report ID and is dropped. Input reports that do
/// not answer are handed to `unrelated`.
pub(super) fn exchange(
    transport: &mut dyn Transport,
    report: &[u8],
    report_ids: bool,
    answers: impl Fn(&[u8]) -> bool,
    mut unrelated: impl FnMut(&[u8]),
) -> Result<Vec<u8>, SessionError> {
    let mut buf = [0; BUFFER_LEN];
    for _ in 0..ATTEMPTS {
        transport.write(report)?;
        let deadline = Instant::now() + ANSWER_TIMEOUT;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            let len = transport.read(&mut buf, remaining)?;
            if len == 0 {
                break;
            }
            let payload = &buf[usize::from(report_ids).min(len)..len];
            if answers(payload) {
                return Ok(payload.to_vec());
            }
            unrelated(payload);
        }
    }
    Err(SessionError::Timeout(
        report.get(1).copied().unwrap_or_default(),
    ))
}

#[cfg(test)]
mod tests {
    use super::{SessionError, exchange};
    use crate::hid::fake::FakeDevice;

    #[test]
    fn exchange_skips_unrelated_reports_and_retries() {
        // Given a device that sends an unrelated report before answering
        let device = FakeDevice::default();
        device.reply(0x06, [vec![0xB4, 0x06, 0x01]]);
        device.push(vec![0xB6, 0xE2, 0x01]);
        let mut transport = device.transport();
        let mut unrelated = Vec::new();
        // When exchanging a request
        let answer = exchange(
            transport.as_mut(),
            &[0xB3, 0x06],
            true,
            |payload| payload.first() == Some(&0x06),
            |payload| unrelated.push(payload.to_vec()),
        )
        .unwrap();
        // Then the answer is returned without its report ID and the event is kept aside
        assert_eq!(answer, [0x06, 0x01]);
        assert_eq!(unrelated, [vec![0xE2, 0x01]]);
    }

    #[test]
    fn exchange_gives_up_after_three_attempts() {
        // Given a device that never answers
        let device = FakeDevice::default();
        let mut transport = device.transport();
        // When exchanging a request
        let result = exchange(transport.as_mut(), &[0xB3, 0x06], true, |_| true, |_| {});
        // Then it was sent three times before timing out
        assert!(matches!(result, Err(SessionError::Timeout(0x06))));
        assert_eq!(device.written().len(), 3);
    }

    #[test]
    fn unplugged_device_is_fatal() {
        // Given an unplugged device
        let device = FakeDevice::default();
        device.unplug();
        let mut transport = device.transport();
        // When exchanging a request
        let error =
            exchange(transport.as_mut(), &[0xB3, 0x06], true, |_| true, |_| {}).unwrap_err();
        // Then the error ends the session
        assert!(error.is_fatal());
        assert!(!SessionError::Timeout(6).is_fatal());
    }
}
