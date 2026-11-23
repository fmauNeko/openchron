//! Keychron HID protocols, as pure request builders and reply parsers.

pub mod mouse;
pub mod via;

use thiserror::Error;

/// A battery reading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Battery {
    /// Charge level, 0 to 100.
    pub percent: u8,
    /// Whether the battery is charging.
    pub charging: bool,
}

/// A reply that does not have the expected shape.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProtocolError {
    /// The reply is shorter than its layout.
    #[error("reply 0x{opcode:02x} has {len} bytes, too few for its layout")]
    Truncated { opcode: u8, len: usize },
    /// The reply answers another request.
    #[error("expected reply 0x{expected:02x}, got 0x{found:02x}")]
    Unexpected { expected: u8, found: u8 },
}

/// Checks that `payload` answers `opcode` and holds at least `len` bytes.
fn expect(payload: &[u8], opcode: u8, len: usize) -> Result<(), ProtocolError> {
    match payload.first() {
        Some(&found) if found != opcode => Err(ProtocolError::Unexpected {
            expected: opcode,
            found,
        }),
        _ if payload.len() < len => Err(ProtocolError::Truncated {
            opcode,
            len: payload.len(),
        }),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::{ProtocolError, expect};

    #[test]
    fn expect_checks_opcode_then_length() {
        // Given replies of the wrong opcode, too short, and well formed
        // When checking them against opcode 0x06 and 4 bytes
        // Then each outcome is reported
        assert_eq!(
            expect(&[0x02, 0, 0, 0], 0x06, 4),
            Err(ProtocolError::Unexpected {
                expected: 0x06,
                found: 0x02
            })
        );
        assert_eq!(
            expect(&[0x06, 0], 0x06, 4),
            Err(ProtocolError::Truncated {
                opcode: 0x06,
                len: 2
            })
        );
        assert_eq!(
            expect(&[], 0x06, 4),
            Err(ProtocolError::Truncated {
                opcode: 0x06,
                len: 0
            })
        );
        assert_eq!(expect(&[0x06, 0, 0, 0], 0x06, 4), Ok(()));
    }
}
