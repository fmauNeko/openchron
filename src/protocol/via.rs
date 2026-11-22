//! VIA raw HID protocol, spoken by Keychron's QMK keyboards on usage page `0xFF60`.
//!
//! Reports are 32 bytes and carry no report ID: a write is prefixed with report ID 0 and a
//! read returns the 32 payload bytes. Byte 0 is the command, which the reply echoes.

use super::{ProtocolError, expect};

/// HID usage page of the raw HID interface.
pub const USAGE_PAGE: u16 = 0xFF60;
/// HID usage of the raw HID interface.
pub const USAGE: u16 = 0x61;

const REPORT_LEN: usize = 32;
const GET_PROTOCOL_VERSION: u8 = 0x01;

/// The report asking for the VIA protocol version.
#[must_use]
pub fn protocol_version_request() -> Vec<u8> {
    let mut report = vec![0; REPORT_LEN + 1];
    report[1] = GET_PROTOCOL_VERSION;
    report
}

/// Parses the reply to [`protocol_version_request`].
///
/// # Errors
/// Returns [`ProtocolError`] when `reply` is not a protocol version reply.
pub fn parse_protocol_version(reply: &[u8]) -> Result<u16, ProtocolError> {
    expect(reply, GET_PROTOCOL_VERSION, 3)?;
    Ok(u16::from_be_bytes([reply[1], reply[2]]))
}

#[cfg(test)]
mod tests {
    use super::{parse_protocol_version, protocol_version_request};

    #[test]
    fn protocol_version_round_trip() {
        // Given the protocol version request and a reply announcing VIA protocol 12
        let request = protocol_version_request();
        let mut reply = [0u8; 32];
        reply[..3].copy_from_slice(&[0x01, 0x00, 0x0C]);
        // When building and parsing them
        // Then the request is report 0 with command 1, and the version is big-endian
        assert_eq!(request.len(), 33);
        assert_eq!(&request[..2], &[0x00, 0x01]);
        assert_eq!(parse_protocol_version(&reply), Ok(12));
        assert!(parse_protocol_version(&[0x02, 0x00, 0x0C]).is_err());
    }
}
