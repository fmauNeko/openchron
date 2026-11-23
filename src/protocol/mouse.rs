//! Keychron mouse vendor protocol, on HID usage page `0xFFC1`.
//!
//! Keychron Launcher names it the "8k" protocol. A request is one output report: `0xB5`
//! carries a 20-byte payload and `0xB3` a 63-byte one. The mouse, or the receiver relaying
//! it, answers with input report `0xB6` or `0xB4`. Byte 0 of every payload is the opcode.
//! The mouse also reports changes on its own: `0xE2` when its battery, DPI stage or polling
//! rate changes, `0xE5` when its profile changes. `docs/keychron-hid.md` documents the
//! byte layouts.

use super::{Battery, ProtocolError, expect};

/// HID usage page of the vendor interface that speaks this protocol.
pub const USAGE_PAGE: u16 = 0xFFC1;

const DEVICE_INFO: u8 = 0x02;
const RECEIVER_DEVICES: u8 = 0x03;
const FIRMWARE_VERSION: u8 = 0x04;
const BASE_INFO: u8 = 0x06;
const SELECT_PROFILE: u8 = 0x0E;
const SET_DPI: u8 = 0x40;
const SET_POLLING: u8 = 0x41;
const SET_DPI_XY: u8 = 0x48;
const GET_DPI_XY: u8 = 0x49;
const SET_POLLING_PER_MODE: u8 = 0x4A;
const GET_POLLING_PER_MODE: u8 = 0x4B;
const STATUS_EVENT: u8 = 0xE2;
const ACK: u8 = 0xE4;
const PROFILE_EVENT: u8 = 0xE5;

/// DPI slots in the base info reply.
const BASE_DPI_SLOTS: usize = 5;
/// DPI slots per axis in the separate X/Y DPI table.
const XY_DPI_SLOTS: usize = 8;
/// Polling-rate slots per connection mode.
const POLLING_SLOTS: usize = 6;

/// The output report a request travels in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Channel {
    /// Report `0xB5`, answered in `0xB6`.
    Short,
    /// Report `0xB3`, answered in `0xB4`.
    Long,
}

impl Channel {
    const fn report_id(self) -> u8 {
        match self {
            Self::Short => 0xB5,
            Self::Long => 0xB3,
        }
    }

    const fn payload_len(self) -> usize {
        match self {
            Self::Short => 20,
            Self::Long => 63,
        }
    }
}

/// How the mouse answers a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Answer {
    /// A reply starting with the request's opcode.
    Reply,
    /// A `0xE4` acknowledgement naming the opcode, or a reply echoing it.
    Ack,
}

/// One request to the mouse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    channel: Channel,
    payload: Vec<u8>,
    answer: Answer,
}

impl Request {
    fn new(channel: Channel, payload: Vec<u8>, answer: Answer) -> Self {
        debug_assert!(!payload.is_empty() && payload.len() <= channel.payload_len());
        Self {
            channel,
            payload,
            answer,
        }
    }

    /// The opcode, byte 0 of the payload.
    #[must_use]
    pub fn opcode(&self) -> u8 {
        self.payload[0]
    }

    /// The bytes to write: the report ID, then the payload zero-padded to the report size.
    #[must_use]
    pub fn report(&self) -> Vec<u8> {
        let mut report = vec![0; self.channel.payload_len() + 1];
        report[0] = self.channel.report_id();
        report[1..=self.payload.len()].copy_from_slice(&self.payload);
        report
    }

    /// Whether `payload`, an input report without its report ID, answers this request.
    #[must_use]
    pub fn is_answered_by(&self, payload: &[u8]) -> bool {
        let opcode = self.opcode();
        let echoed = payload.first() == Some(&opcode);
        match self.answer {
            Answer::Reply => echoed,
            Answer::Ack => echoed || matches!(payload, [ACK, _, acked, ..] if *acked == opcode),
        }
    }
}

/// How the mouse is connected, as the protocol numbers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkMode {
    /// USB cable.
    Wired,
    /// 2.4 GHz receiver.
    Receiver,
    /// Bluetooth.
    Bluetooth,
}

impl WorkMode {
    fn from_byte(byte: u8) -> Option<Self> {
        match byte & 0x07 {
            0 => Some(Self::Wired),
            1 => Some(Self::Receiver),
            2 => Some(Self::Bluetooth),
            _ => None,
        }
    }

    const fn byte(self) -> u8 {
        match self {
            Self::Wired => 0,
            Self::Receiver => 1,
            Self::Bluetooth => 2,
        }
    }

    const fn index(self) -> usize {
        self.byte() as usize
    }
}

/// Optional capabilities the mouse reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Features([u8; 4]);

impl Features {
    /// Whether the mouse stores several profiles.
    #[must_use]
    pub const fn profiles(self) -> bool {
        self.0[0] & 0x20 != 0
    }

    /// Whether DPI stages have separate X and Y values (table `0x49`, setter `0x48`).
    #[must_use]
    pub const fn separate_xy_dpi(self) -> bool {
        self.0[2] & 0x01 != 0
    }

    /// Whether each connection mode has its own polling rates (table `0x4B`, setter `0x4A`).
    #[must_use]
    pub const fn polling_per_mode(self) -> bool {
        self.0[2] & 0x02 != 0
    }
}

/// DPI stages and the active one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DpiStages {
    /// DPI of each stage.
    pub values: Vec<u16>,
    /// Index of the active stage.
    pub active: usize,
}

impl DpiStages {
    /// DPI of the active stage.
    #[must_use]
    pub fn active_value(&self) -> Option<u16> {
        self.values.get(self.active).copied()
    }
}

/// Selectable polling rates and the active one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PollingRates {
    /// Rates in Hz; 0 for a rate code this version does not know.
    pub rates: Vec<u16>,
    /// Index of the active rate.
    pub active: usize,
}

/// Asks for the protocol version, identity, firmware version and features.
#[must_use]
pub fn device_info(mode: WorkMode) -> Request {
    Request::new(
        Channel::Short,
        vec![DEVICE_INFO, mode.byte()],
        Answer::Reply,
    )
}

/// Asks a receiver for the devices paired with it.
#[must_use]
pub fn receiver_devices() -> Request {
    Request::new(Channel::Short, vec![RECEIVER_DEVICES], Answer::Reply)
}

/// Asks for the full firmware version string.
#[must_use]
pub fn firmware_version(mode: WorkMode) -> Request {
    Request::new(
        Channel::Long,
        vec![FIRMWARE_VERSION, mode.byte()],
        Answer::Reply,
    )
}

/// Asks for the current profile, DPI, polling rate and battery.
#[must_use]
pub fn base_info() -> Request {
    Request::new(Channel::Long, vec![BASE_INFO], Answer::Reply)
}

/// Asks for the separate X/Y DPI table.
#[must_use]
pub fn dpi_table() -> Request {
    Request::new(Channel::Long, vec![GET_DPI_XY], Answer::Reply)
}

/// Asks for the polling rates of each connection mode.
#[must_use]
pub fn polling_table() -> Request {
    Request::new(Channel::Short, vec![GET_POLLING_PER_MODE], Answer::Reply)
}

/// Makes `profile` (0-based) the active profile; `count` must be the current profile count,
/// since the same command also sets it.
#[must_use]
pub fn select_profile(profile: u8, count: u8) -> Request {
    Request::new(
        Channel::Short,
        vec![SELECT_PROFILE, profile, count],
        Answer::Ack,
    )
}

/// Identity and capabilities, the reply to [`device_info`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    /// Protocol version; layouts change from version 6.
    pub protocol: u16,
    /// USB vendor ID of the mouse.
    pub vendor_id: u16,
    /// USB product ID of the mouse, also when it is reached through a receiver.
    pub product_id: u16,
    /// Firmware version, `major.minor.patch`.
    pub firmware: String,
    /// Current connection mode.
    pub work_mode: Option<WorkMode>,
    /// Feature flags, reported here from protocol 6 on.
    pub features: Option<Features>,
}

impl DeviceInfo {
    /// Parses the reply to [`device_info`].
    ///
    /// # Errors
    /// Returns [`ProtocolError`] when `payload` is not a complete device info reply.
    pub fn parse(payload: &[u8]) -> Result<Self, ProtocolError> {
        expect(payload, DEVICE_INFO, 16)?;
        let protocol = u16_le(payload, 1);
        Ok(Self {
            protocol,
            vendor_id: u16_le(payload, 3),
            product_id: u16_le(payload, 5),
            firmware: format!("{}.{}.{}", payload[8], payload[7] >> 4, payload[7] & 0x0F),
            work_mode: WorkMode::from_byte(payload[9]),
            features: (protocol >= 6)
                .then(|| Features([payload[11], payload[12], payload[13], payload[15]])),
        })
    }
}

/// A device paired with a receiver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PairedDevice {
    /// USB vendor ID.
    pub vendor_id: u16,
    /// USB product ID.
    pub product_id: u16,
    /// Whether the device is connected to the receiver right now.
    pub active: bool,
}

/// Parses the reply to [`receiver_devices`].
///
/// # Errors
/// Returns [`ProtocolError`] when `payload` is not a receiver devices reply.
pub fn parse_receiver_devices(payload: &[u8]) -> Result<Vec<PairedDevice>, ProtocolError> {
    expect(payload, RECEIVER_DEVICES, 2)?;
    Ok(payload[2..]
        .as_chunks::<5>()
        .0
        .iter()
        .take(usize::from(payload[1]))
        .map(|record| PairedDevice {
            vendor_id: u16_le(record, 0),
            product_id: u16_le(record, 2),
            active: record[4] == 1,
        })
        .collect())
}

/// Parses the reply to [`firmware_version`].
///
/// # Errors
/// Returns [`ProtocolError`] when `payload` is not a complete firmware version reply.
pub fn parse_firmware_version(payload: &[u8]) -> Result<String, ProtocolError> {
    expect(payload, FIRMWARE_VERSION, 2)?;
    let text = payload
        .get(2..2 + usize::from(payload[1]))
        .ok_or(ProtocolError::Truncated {
            opcode: FIRMWARE_VERSION,
            len: payload.len(),
        })?;
    Ok(String::from_utf8_lossy(text)
        .trim_end_matches('\0')
        .to_owned())
}

/// Current settings and battery, the reply to [`base_info`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseInfo {
    /// Active profile, 0-based.
    pub profile: u8,
    /// Number of profiles.
    pub profile_count: u8,
    /// Battery reading.
    pub battery: Battery,
    levels: [u8; 3],
    dpi_values: [u16; BASE_DPI_SLOTS],
    dpi_count: u8,
    polling_codes: [u8; POLLING_SLOTS],
    polling_count: u8,
    features: Features,
}

impl BaseInfo {
    /// Parses the reply to [`base_info`].
    ///
    /// # Errors
    /// Returns [`ProtocolError`] when `payload` is not a complete base info reply.
    pub fn parse(payload: &[u8]) -> Result<Self, ProtocolError> {
        expect(payload, BASE_INFO, 61)?;
        let mut dpi_values = [0; BASE_DPI_SLOTS];
        for (slot, value) in dpi_values.iter_mut().enumerate() {
            *value = u16_le(payload, 5 + 2 * slot);
        }
        let mut polling_codes = [0; POLLING_SLOTS];
        polling_codes.copy_from_slice(&payload[43..43 + POLLING_SLOTS]);
        Ok(Self {
            profile: payload[1],
            profile_count: payload[50],
            battery: Battery {
                percent: (payload[19] & 0x7F).min(100),
                charging: payload[19] & 0x80 != 0,
            },
            levels: [payload[2], payload[3], payload[4]],
            dpi_values,
            dpi_count: payload[16],
            polling_codes,
            polling_count: match payload[49] {
                0 => 6,
                count => count,
            },
            features: Features([payload[26], payload[53], payload[60], 0]),
        })
    }

    /// Feature flags, as reported here before protocol 6.
    #[must_use]
    pub const fn features(&self) -> Features {
        self.features
    }

    /// DPI stages of mice without separate X/Y DPI.
    #[must_use]
    pub fn dpi(&self, mode: WorkMode) -> DpiStages {
        let count = usize::from(self.dpi_count).min(BASE_DPI_SLOTS);
        DpiStages {
            values: self.dpi_values[..count].to_vec(),
            active: usize::from(self.levels[mode.index()] & 0x0F),
        }
    }

    /// Polling rates of mice without per-mode polling rates.
    #[must_use]
    pub fn polling(&self, mode: WorkMode) -> PollingRates {
        let count = usize::from(self.polling_count).min(POLLING_SLOTS);
        PollingRates {
            rates: self.polling_codes[..count]
                .iter()
                .map(|&code| polling_rate_hz(code))
                .collect(),
            active: usize::from(self.levels[mode.index()] >> 4),
        }
    }

    /// Makes `stage` the active DPI stage of a mouse without separate X/Y DPI.
    #[must_use]
    pub fn select_dpi_stage(&self, stage: u8) -> Request {
        let mut payload = vec![SET_DPI, stage, stage, stage];
        for value in self.dpi_values {
            payload.extend_from_slice(&value.to_le_bytes());
        }
        payload.push(self.dpi_count);
        Request::new(Channel::Short, payload, Answer::Ack)
    }

    /// Makes `level` the active polling rate of a mouse without per-mode polling rates.
    #[must_use]
    pub fn select_polling_level(&self, level: u8) -> Request {
        let mut payload = vec![SET_POLLING, level, level];
        payload.extend_from_slice(&self.polling_codes);
        payload.push(self.polling_count);
        Request::new(Channel::Short, payload, Answer::Ack)
    }
}

/// Separate X/Y DPI table, the reply to [`dpi_table`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DpiTable {
    payload: Vec<u8>,
    /// Before protocol 6, each mode's level byte packs the Y stage over the X stage.
    packed_levels: bool,
}

impl DpiTable {
    /// Parses the reply to [`dpi_table`] for a mouse speaking `protocol`.
    ///
    /// # Errors
    /// Returns [`ProtocolError`] when `payload` is not a complete DPI table reply.
    pub fn parse(payload: &[u8], protocol: u16) -> Result<Self, ProtocolError> {
        let packed_levels = protocol < 6;
        let len = if packed_levels { 45 } else { 38 };
        expect(payload, GET_DPI_XY, len)?;
        Ok(Self {
            payload: payload[..len].to_vec(),
            packed_levels,
        })
    }

    /// DPI stages (X values) and the stage active in `mode`.
    #[must_use]
    pub fn stages(&self, mode: WorkMode) -> DpiStages {
        let count = usize::from(self.payload[4]).min(XY_DPI_SLOTS);
        let level = self.payload[1 + mode.index()];
        DpiStages {
            values: (0..count)
                .map(|slot| u16_le(&self.payload, 5 + 2 * slot))
                .collect(),
            active: usize::from(if self.packed_levels {
                level & 0x0F
            } else {
                level
            }),
        }
    }

    /// Makes `stage` the active DPI stage in every connection mode, keeping the table.
    #[must_use]
    pub fn select(&self, stage: u8) -> Request {
        let mut payload = self.payload.clone();
        payload[0] = SET_DPI_XY;
        let level = if self.packed_levels {
            (stage << 4) | stage
        } else {
            stage
        };
        payload[1..4].fill(level);
        Request::new(Channel::Long, payload, Answer::Ack)
    }
}

/// Polling rates of the wired and receiver modes, the reply to [`polling_table`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PollingTable([u8; 17]);

impl PollingTable {
    /// Parses the reply to [`polling_table`].
    ///
    /// # Errors
    /// Returns [`ProtocolError`] when `payload` is not a complete polling table reply.
    pub fn parse(payload: &[u8]) -> Result<Self, ProtocolError> {
        expect(payload, GET_POLLING_PER_MODE, 17)?;
        let mut table = [0; 17];
        table.copy_from_slice(&payload[..17]);
        Ok(Self(table))
    }

    const fn slot(mode: WorkMode) -> Option<usize> {
        match mode {
            WorkMode::Wired => Some(0),
            WorkMode::Receiver => Some(1),
            WorkMode::Bluetooth => None,
        }
    }

    /// Polling rates of `mode`; `None` for Bluetooth, whose rate is fixed.
    #[must_use]
    pub fn rates(&self, mode: WorkMode) -> Option<PollingRates> {
        let slot = Self::slot(mode)?;
        let count = usize::from(self.0[3 + slot]).min(POLLING_SLOTS);
        let codes = &self.0[5 + slot * POLLING_SLOTS..][..count];
        Some(PollingRates {
            rates: codes.iter().map(|&code| polling_rate_hz(code)).collect(),
            active: usize::from(self.0[1 + slot]),
        })
    }

    /// Makes `level` the active polling rate of `mode`, keeping the other mode's.
    #[must_use]
    pub fn select(&self, mode: WorkMode, level: u8) -> Option<Request> {
        let slot = Self::slot(mode)?;
        let mut payload = self.0.to_vec();
        payload[0] = SET_POLLING_PER_MODE;
        payload[1 + slot] = level;
        Some(Request::new(Channel::Short, payload, Answer::Ack))
    }
}

/// Status the mouse reports on its own when it changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Status {
    /// Connection mode.
    pub work_mode: Option<WorkMode>,
    /// Whether the mouse is connected, false when a receiver lost it.
    pub connected: bool,
    /// Power state byte; it changes when the mouse starts or stops charging.
    pub power: u8,
    /// Battery level, 0 to 100.
    pub battery: u8,
    /// Active DPI stage.
    pub dpi_stage: u8,
    /// Active polling level.
    pub polling_level: u8,
    /// Number of DPI stages; 0 when the status carries no settings, as when the mouse
    /// reconnects to its receiver.
    pub dpi_stage_count: u8,
}

/// A report the mouse sends on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// Battery, DPI stage, polling rate or connection changed (`0xE2`).
    Status(Status),
    /// The active profile changed, for example with a button on the mouse (`0xE5`).
    ProfileChanged(u8),
}

impl Event {
    /// Parses an input report payload; `None` when it is not an event.
    #[must_use]
    pub fn parse(payload: &[u8]) -> Option<Self> {
        match *payload {
            [
                STATUS_EVENT,
                work_mode,
                connected,
                power,
                battery,
                dpi_stage,
                polling_level,
                dpi_stage_count,
                ..,
            ] => Some(Self::Status(Status {
                work_mode: WorkMode::from_byte(work_mode),
                connected: connected == 1,
                power,
                battery: (battery & 0x7F).min(100),
                dpi_stage,
                polling_level,
                dpi_stage_count,
            })),
            [PROFILE_EVENT, profile, ..] => Some(Self::ProfileChanged(profile)),
            _ => None,
        }
    }
}

/// The status byte of an acknowledgement (0 for success); `None` when `payload` is not one.
#[must_use]
pub fn ack_status(payload: &[u8]) -> Option<u8> {
    match *payload {
        [ACK, status, ..] => Some(status),
        _ => None,
    }
}

/// Polling rate in Hz of a rate code; 0 for an unknown code.
const fn polling_rate_hz(code: u8) -> u16 {
    match code {
        0 => 125,
        6 => 250,
        1 => 500,
        2 => 1000,
        3 => 2000,
        4 => 4000,
        5 => 8000,
        _ => 0,
    }
}

fn u16_le(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Decodes space-separated hex bytes, zero-padded to at least `len` bytes.
    pub(crate) fn frame(hex: &str, len: usize) -> Vec<u8> {
        let mut bytes: Vec<u8> = hex
            .split_whitespace()
            .map(|byte| u8::from_str_radix(byte, 16).unwrap())
            .collect();
        bytes.resize(len.max(bytes.len()), 0);
        bytes
    }

    // Reports captured from a Keychron G6 HE paired with a TurboLink 8K receiver, without
    // their report ID.
    pub(crate) const DEVICE_INFO_REPLY: &str = "02 06 00 34 34 9D D0 06 01 59 00 BB 9F 9B 00 38 01";
    pub(crate) const RECEIVER_DEVICES_REPLY: &str = "03 01 34 34 9D D0 01";
    pub(crate) const FIRMWARE_VERSION_REPLY: &str = "04 07 31 2E 30 2E 36 2B 30";
    pub(crate) const BASE_INFO_REPLY: &str = "06 00 22 22 02 90 01 20 03 40 06 80 0C 00 00 01 \
        05 04 0A 3B 00 00 00 00 00 00 BB 00 00 01 04 04 04 04 04 04 04 04 04 04 50 C3 01 00 01 \
        02 03 04 05 06 05 FF 00 9F 01 00 00 04 00 00 9B B4";
    pub(crate) const DPI_TABLE_REPLY: &str = "49 02 02 02 05 90 01 20 03 40 06 80 0C 88 13 00 \
        00 00 00 00 00 90 01 20 03 40 06 80 0C 88 13 00 00 00 00 00 00 1F";
    pub(crate) const POLLING_TABLE_REPLY: &str =
        "4B 02 02 06 06 00 01 02 03 04 05 00 01 02 03 04 05";
    pub(crate) const STATUS_EVENT_REPORT: &str = "E2 01 01 00 3B 02 02 05 06";

    #[test]
    fn requests_are_zero_padded_reports() {
        // Given a short and a long request
        // When rendering them as reports
        let short = device_info(WorkMode::Receiver).report();
        let long = base_info().report();
        // Then each starts with its report ID and fills its report size
        assert_eq!(short.len(), 21);
        assert_eq!(&short[..3], &[0xB5, 0x02, 0x01]);
        assert!(short[3..].iter().all(|&byte| byte == 0));
        assert_eq!(long.len(), 64);
        assert_eq!(&long[..2], &[0xB3, 0x06]);
        assert_eq!(
            select_profile(2, 5).report(),
            frame("B5 0E 02 05", 21),
            "the profile count is sent back unchanged"
        );
    }

    #[test]
    fn device_info_reply_parses() {
        // Given the captured device info reply
        // When parsing it
        let info = DeviceInfo::parse(&frame(DEVICE_INFO_REPLY, 63)).unwrap();
        // Then the identity, firmware, mode and features are decoded
        assert_eq!(info.protocol, 6);
        assert_eq!((info.vendor_id, info.product_id), (0x3434, 0xD09D));
        assert_eq!(info.firmware, "1.0.6");
        assert_eq!(info.work_mode, Some(WorkMode::Receiver));
        let features = info.features.unwrap();
        assert!(features.profiles());
        assert!(features.separate_xy_dpi());
        assert!(features.polling_per_mode());
    }

    #[test]
    fn receiver_devices_reply_lists_the_active_mouse() {
        // Given the captured receiver reply
        // When parsing it
        let devices = parse_receiver_devices(&frame(RECEIVER_DEVICES_REPLY, 63)).unwrap();
        // Then the one paired mouse is active
        assert_eq!(
            devices,
            [PairedDevice {
                vendor_id: 0x3434,
                product_id: 0xD09D,
                active: true
            }]
        );
    }

    #[test]
    fn firmware_version_reply_parses() {
        // Given the captured firmware version reply, padded with zeros
        // When parsing it
        let version = parse_firmware_version(&frame(FIRMWARE_VERSION_REPLY, 63)).unwrap();
        // Then the length-prefixed string is returned
        assert_eq!(version, "1.0.6+0");
        assert!(parse_firmware_version(&frame("04 09 31", 3)).is_err());
    }

    #[test]
    fn base_info_reply_parses() {
        // Given the captured base info reply
        // When parsing it
        let base = BaseInfo::parse(&frame(BASE_INFO_REPLY, 63)).unwrap();
        // Then profile, battery, DPI and polling rate are decoded
        assert_eq!((base.profile, base.profile_count), (0, 5));
        assert_eq!(
            base.battery,
            Battery {
                percent: 59,
                charging: false
            }
        );
        assert_eq!(base.dpi(WorkMode::Receiver).active, 2);
        assert_eq!(
            base.polling(WorkMode::Receiver),
            PollingRates {
                rates: vec![125, 500, 1000, 2000, 4000, 8000],
                active: 2
            }
        );
        let device = DeviceInfo::parse(&frame(DEVICE_INFO_REPLY, 63)).unwrap();
        assert_eq!(
            Some(base.features()),
            device
                .features
                .map(|f| Features([f.0[0], f.0[1], f.0[2], 0]))
        );
    }

    #[test]
    fn charging_bit_is_separate_from_the_level() {
        // Given a base info reply whose battery byte has the charging bit set
        let mut payload = frame(BASE_INFO_REPLY, 63);
        payload[19] = 0x80 | 0x2A;
        // When parsing it
        let battery = BaseInfo::parse(&payload).unwrap().battery;
        // Then the level and the charging state are both read
        assert_eq!(
            battery,
            Battery {
                percent: 42,
                charging: true
            }
        );
    }

    #[test]
    fn dpi_table_reply_parses_and_selects_a_stage() {
        // Given the captured DPI table reply of a protocol 6 mouse
        let table = DpiTable::parse(&frame(DPI_TABLE_REPLY, 63), 6).unwrap();
        // When reading it and selecting stage 1
        let stages = table.stages(WorkMode::Receiver);
        let request = table.select(1);
        // Then the stages are listed and the request echoes the table with the new stage
        assert_eq!(stages.values, [400, 800, 1600, 3200, 5000]);
        assert_eq!(stages.active, 2);
        assert_eq!(stages.active_value(), Some(1600));
        let report = request.report();
        assert_eq!(&report[..6], &[0xB3, 0x48, 1, 1, 1, 5]);
        assert_eq!(&report[6..39], &frame(DPI_TABLE_REPLY, 63)[5..38]);
        assert!(report[39..].iter().all(|&byte| byte == 0));
        assert!(request.is_answered_by(&frame("E4 00 48", 63)));
        assert!(!request.is_answered_by(&frame("E4 00 0E", 63)));
    }

    #[test]
    fn packed_dpi_levels_before_protocol_6() {
        // Given a protocol 4 DPI table whose level bytes pack Y over X
        let mut payload = frame(DPI_TABLE_REPLY, 63);
        payload[1..4].fill(0x33);
        let table = DpiTable::parse(&payload, 4).unwrap();
        // When reading the stage and selecting another
        // Then the X nibble is the stage and both nibbles are written
        assert_eq!(table.stages(WorkMode::Wired).active, 3);
        assert_eq!(&table.select(1).report()[2..5], &[0x11, 0x11, 0x11]);
    }

    #[test]
    fn polling_table_reply_parses_and_selects_a_level() {
        // Given the captured polling table reply
        let table = PollingTable::parse(&frame(POLLING_TABLE_REPLY, 63)).unwrap();
        // When reading the receiver rates and selecting 8000 Hz on the receiver
        let rates = table.rates(WorkMode::Receiver).unwrap();
        let request = table.select(WorkMode::Receiver, 5).unwrap();
        // Then only the receiver level changes
        assert_eq!(rates.rates, [125, 500, 1000, 2000, 4000, 8000]);
        assert_eq!(rates.active, 2);
        assert_eq!(
            request.report(),
            frame("B5 4A 02 05 06 06 00 01 02 03 04 05 00 01 02 03 04 05", 21)
        );
        assert_eq!(table.rates(WorkMode::Bluetooth), None);
        assert_eq!(table.select(WorkMode::Bluetooth, 0), None);
    }

    #[test]
    fn legacy_setters_match_the_launcher_layout() {
        // Given the captured base info reply
        let base = BaseInfo::parse(&frame(BASE_INFO_REPLY, 63)).unwrap();
        // When selecting a DPI stage and a polling level with the legacy commands
        // Then the requests carry the whole table, like Keychron Launcher's
        assert_eq!(
            base.select_dpi_stage(2).report(),
            frame("B5 40 02 02 02 90 01 20 03 40 06 80 0C 00 00 05", 21)
        );
        assert_eq!(
            base.select_polling_level(3).report(),
            frame("B5 41 03 03 00 01 02 03 04 05 06", 21)
        );
    }

    #[test]
    fn status_and_profile_events_parse() {
        // Given the captured status event and a profile event
        // When parsing them
        // Then their fields are decoded, and replies are not events
        assert_eq!(
            Event::parse(&frame(STATUS_EVENT_REPORT, 63)),
            Some(Event::Status(Status {
                work_mode: Some(WorkMode::Receiver),
                connected: true,
                power: 0,
                battery: 59,
                dpi_stage: 2,
                polling_level: 2,
                dpi_stage_count: 5,
            }))
        );
        assert_eq!(
            Event::parse(&frame("E5 03", 63)),
            Some(Event::ProfileChanged(3))
        );
        assert_eq!(Event::parse(&frame(BASE_INFO_REPLY, 63)), None);
    }

    #[test]
    fn replies_and_acknowledgements_answer_their_request() {
        // Given a query and a command
        let query = base_info();
        let command = select_profile(1, 5);
        // When matching input reports against them
        // Then a query only takes its reply, a command its acknowledgement or echo
        assert!(query.is_answered_by(&frame(BASE_INFO_REPLY, 63)));
        assert!(!query.is_answered_by(&frame(STATUS_EVENT_REPORT, 63)));
        assert!(!query.is_answered_by(&frame("E4 00 06", 63)));
        assert!(command.is_answered_by(&frame("E4 00 0E", 63)));
        assert!(command.is_answered_by(&frame("0E 01", 63)));
        assert_eq!(ack_status(&frame("E4 03 0E", 63)), Some(3));
        assert_eq!(ack_status(&frame("0E 01", 63)), None);
    }

    #[test]
    fn short_replies_are_rejected() {
        // Given replies cut short
        // When parsing them
        // Then each parser reports the truncation instead of reading past the end
        assert!(DeviceInfo::parse(&frame("02 06 00", 3)).is_err());
        assert!(BaseInfo::parse(&frame("06 00", 20)).is_err());
        assert!(DpiTable::parse(&frame("49 02", 20), 6).is_err());
        assert!(PollingTable::parse(&frame("4B 02", 10)).is_err());
        assert_eq!(
            parse_receiver_devices(&frame("03 03 34 34 9D D0 01", 7))
                .unwrap()
                .len(),
            1,
            "records past the end of the reply are dropped"
        );
    }
}
