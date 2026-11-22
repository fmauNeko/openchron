//! Mouse session: the state of a mouse, or of the mouse a receiver relays, kept in sync
//! through the Keychron mouse protocol.

use std::collections::VecDeque;
use std::time::Duration;

use crate::hid::{InterfaceInfo, Transport};
use crate::protocol::mouse::{
    self, BaseInfo, DeviceInfo, DpiTable, Event, Features, PollingTable, Request, Status, WorkMode,
};

use super::catalog;
use super::model::{Command, Connection, Details, DeviceState, MouseState, Profiles};
use super::session::{SessionError, exchange};

/// A session with a mouse interface (usage page `0xFFC1`).
pub struct MouseSession {
    transport: Box<dyn Transport>,
    /// Whether the interface belongs to a receiver rather than to the mouse.
    receiver: bool,
    /// USB product string of the interface.
    usb_product: Option<String>,
    mode: WorkMode,
    protocol: u16,
    base: Option<BaseInfo>,
    dpi_table: Option<DpiTable>,
    polling_table: Option<PollingTable>,
    /// Last power state byte of a status event; a change means charging started or stopped.
    power: Option<u8>,
    /// Input reports received while waiting for an answer.
    pending: VecDeque<Vec<u8>>,
    state: DeviceState,
}

impl MouseSession {
    /// Starts a session; nothing is read until [`MouseSession::refresh`].
    #[must_use]
    pub fn new(transport: Box<dyn Transport>, interface: &InterfaceInfo) -> Self {
        let receiver = catalog::is_receiver(interface.product_id);
        let name = catalog::name(
            interface.product_id,
            interface.product.as_deref(),
            if receiver { "receiver" } else { "mouse" },
        );
        Self {
            transport,
            receiver,
            usb_product: interface.product.clone(),
            mode: if receiver {
                WorkMode::Receiver
            } else {
                WorkMode::Wired
            },
            protocol: 0,
            base: None,
            dpi_table: None,
            polling_table: None,
            power: None,
            pending: VecDeque::new(),
            state: DeviceState {
                connection: if receiver {
                    Connection::Receiver(name.clone())
                } else {
                    Connection::Wired
                },
                name,
                product_id: interface.product_id,
                firmware: None,
                online: false,
                details: Details::Mouse(MouseState::default()),
            },
        }
    }

    /// What is known about the mouse.
    #[must_use]
    pub const fn state(&self) -> &DeviceState {
        &self.state
    }

    /// Reads everything from the mouse. On failure the mouse is marked offline.
    ///
    /// # Errors
    /// See [`SessionError`].
    pub fn refresh(&mut self) -> Result<(), SessionError> {
        let result = self.read_all();
        if result.is_err() {
            self.state.online = false;
        }
        result
    }

    fn read_all(&mut self) -> Result<(), SessionError> {
        // A receiver answers this itself, but relays the other requests to the mouse, which
        // does not answer while it sleeps.
        if self.receiver {
            let paired = mouse::parse_receiver_devices(&self.query(&mouse::receiver_devices())?)?;
            let mouse = paired
                .iter()
                .find(|device| device.active)
                .or_else(|| paired.first());
            if let Some(mouse) = mouse {
                self.state.product_id = mouse.product_id;
                self.state.name = catalog::name(mouse.product_id, None, "mouse");
            }
            if !mouse.is_some_and(|mouse| mouse.active) {
                self.state.online = false;
                return Ok(());
            }
        }
        let info = DeviceInfo::parse(&self.query(&mouse::device_info(self.mode))?)?;
        self.protocol = info.protocol;
        if !self.receiver {
            self.mode = info.work_mode.unwrap_or(self.mode);
            self.state.connection = match self.mode {
                WorkMode::Bluetooth => Connection::Bluetooth,
                WorkMode::Wired | WorkMode::Receiver => Connection::Wired,
            };
            self.state.product_id = info.product_id;
            self.state.name = catalog::name(info.product_id, self.usb_product.as_deref(), "mouse");
        }
        self.state.firmware = Some(match self.query(&mouse::firmware_version(self.mode)) {
            Ok(payload) => mouse::parse_firmware_version(&payload)
                .ok()
                .filter(|version| !version.is_empty())
                .unwrap_or(info.firmware),
            Err(error) if error.is_fatal() => return Err(error),
            Err(_) => info.firmware,
        });

        let base = BaseInfo::parse(&self.query(&mouse::base_info())?)?;
        let features: Features = info.features.unwrap_or_else(|| base.features());
        let dpi = if features.separate_xy_dpi() {
            let table = DpiTable::parse(&self.query(&mouse::dpi_table())?, self.protocol)?;
            let stages = table.stages(self.mode);
            self.dpi_table = Some(table);
            stages
        } else {
            self.dpi_table = None;
            base.dpi(self.mode)
        };
        let polling = if features.polling_per_mode() {
            let table = PollingTable::parse(&self.query(&mouse::polling_table())?)?;
            let rates = table.rates(self.mode);
            self.polling_table = Some(table);
            rates
        } else {
            self.polling_table = None;
            Some(base.polling(self.mode))
        };
        self.state.details = Details::Mouse(MouseState {
            battery: Some(base.battery),
            dpi: Some(dpi),
            polling,
            profiles: features.profiles().then_some(Profiles {
                active: base.profile,
                count: base.profile_count,
            }),
        });
        self.state.online = true;
        self.base = Some(base);
        Ok(())
    }

    /// Waits up to `timeout` for a report the mouse sends on its own and applies it.
    /// Returns whether the state changed.
    ///
    /// # Errors
    /// See [`SessionError`].
    pub fn poll(&mut self, timeout: Duration) -> Result<bool, SessionError> {
        let payload = if let Some(payload) = self.pending.pop_front() {
            payload
        } else {
            let mut buf = [0; 65];
            let len = self.transport.read(&mut buf, timeout)?;
            if len == 0 {
                return Ok(false);
            }
            buf[1..len].to_vec()
        };
        let before = self.state.clone();
        match Event::parse(&payload) {
            Some(Event::Status(status)) => self.apply_status(status)?,
            Some(Event::ProfileChanged(_)) => self.refresh()?,
            None => {}
        }
        Ok(self.state != before)
    }

    fn apply_status(&mut self, status: Status) -> Result<(), SessionError> {
        let lost =
            !status.connected || (self.receiver && status.work_mode != Some(WorkMode::Receiver));
        if lost {
            self.state.online = false;
            return Ok(());
        }
        if status.dpi_stage_count == 0 {
            return self.refresh();
        }
        let power_changed = self
            .power
            .replace(status.power)
            .is_some_and(|previous| previous != status.power);
        if !self.state.online || power_changed {
            return self.refresh();
        }
        if let Details::Mouse(mouse) = &mut self.state.details {
            if let Some(battery) = &mut mouse.battery {
                battery.percent = status.battery;
            }
            if let Some(dpi) = &mut mouse.dpi
                && usize::from(status.dpi_stage) < dpi.values.len()
            {
                dpi.active = usize::from(status.dpi_stage);
            }
            if let Some(polling) = &mut mouse.polling
                && usize::from(status.polling_level) < polling.rates.len()
            {
                polling.active = usize::from(status.polling_level);
            }
        }
        Ok(())
    }

    /// Applies a command, then reads the state back to check it.
    ///
    /// # Errors
    /// See [`SessionError`].
    pub fn execute(&mut self, command: Command) -> Result<(), SessionError> {
        let request = match command {
            Command::Refresh => return self.refresh(),
            Command::SelectDpiStage(stage) => self.dpi_request(stage)?,
            Command::SelectPollingRate(level) => self.polling_request(level)?,
            Command::SelectProfile(profile) => self.profile_request(profile)?,
        };
        // Some commands are applied without an acknowledgement: the read-back decides.
        match self.query(&request) {
            Ok(_) | Err(SessionError::Timeout(_)) => {}
            Err(error) => return Err(error),
        }
        self.refresh()?;
        if self.applied(command) {
            Ok(())
        } else {
            Err(SessionError::NotApplied)
        }
    }

    fn mouse_state(&self) -> Option<&MouseState> {
        self.state.mouse().filter(|_| self.state.online)
    }

    fn dpi_request(&self, stage: usize) -> Result<Request, SessionError> {
        let count = self
            .mouse_state()
            .and_then(|mouse| mouse.dpi.as_ref())
            .map_or(0, |dpi| dpi.values.len());
        let stage = index(stage, count)?;
        match (&self.dpi_table, &self.base) {
            (Some(table), _) => Ok(table.select(stage)),
            (None, Some(base)) => Ok(base.select_dpi_stage(stage)),
            (None, None) => Err(SessionError::Unsupported),
        }
    }

    fn polling_request(&self, level: usize) -> Result<Request, SessionError> {
        let count = self
            .mouse_state()
            .and_then(|mouse| mouse.polling.as_ref())
            .map_or(0, |polling| polling.rates.len());
        let level = index(level, count)?;
        match (&self.polling_table, &self.base) {
            (Some(table), _) => table
                .select(self.mode, level)
                .ok_or(SessionError::Unsupported),
            (None, Some(base)) => Ok(base.select_polling_level(level)),
            (None, None) => Err(SessionError::Unsupported),
        }
    }

    fn profile_request(&self, profile: u8) -> Result<Request, SessionError> {
        let profiles = self
            .mouse_state()
            .and_then(|mouse| mouse.profiles)
            .ok_or(SessionError::Unsupported)?;
        if profile >= profiles.count {
            return Err(SessionError::Unsupported);
        }
        Ok(mouse::select_profile(profile, profiles.count))
    }

    fn applied(&self, command: Command) -> bool {
        let Some(mouse) = self.mouse_state() else {
            return false;
        };
        match command {
            Command::SelectDpiStage(stage) => {
                mouse.dpi.as_ref().is_some_and(|dpi| dpi.active == stage)
            }
            Command::SelectPollingRate(level) => mouse
                .polling
                .as_ref()
                .is_some_and(|polling| polling.active == level),
            Command::SelectProfile(profile) => mouse
                .profiles
                .is_some_and(|profiles| profiles.active == profile),
            Command::Refresh => true,
        }
    }

    fn query(&mut self, request: &Request) -> Result<Vec<u8>, SessionError> {
        let pending = &mut self.pending;
        let payload = exchange(
            self.transport.as_mut(),
            &request.report(),
            true,
            |payload| request.is_answered_by(payload),
            |payload| pending.push_back(payload.to_vec()),
        )?;
        match mouse::ack_status(&payload) {
            Some(status) if status != 0 => Err(SessionError::Rejected {
                opcode: request.opcode(),
                status,
            }),
            _ => Ok(payload),
        }
    }
}

/// Converts a 0-based index for a list of `count` items to the byte the protocol uses.
fn index(index: usize, count: usize) -> Result<u8, SessionError> {
    u8::try_from(index)
        .ok()
        .filter(|_| index < count)
        .ok_or(SessionError::Unsupported)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::MouseSession;
    use crate::devices::model::{Command, Connection, Details, DeviceState, Profiles};
    use crate::devices::session::SessionError;
    use crate::hid::InterfaceInfo;
    use crate::hid::fake::FakeDevice;
    use crate::protocol::Battery;
    use crate::protocol::mouse::tests::{
        BASE_INFO_REPLY, DEVICE_INFO_REPLY, DPI_TABLE_REPLY, FIRMWARE_VERSION_REPLY,
        POLLING_TABLE_REPLY, RECEIVER_DEVICES_REPLY, STATUS_EVENT_REPORT, frame,
    };
    use crate::protocol::mouse::{DpiStages, PollingRates};

    fn input(report_id: u8, hex: &str) -> Vec<u8> {
        let mut report = vec![report_id];
        report.extend(frame(hex, 63));
        report
    }

    /// A TurboLink 8K receiver relaying the G6 HE captured in `protocol::mouse::tests`.
    pub(crate) fn turbolink() -> (InterfaceInfo, FakeDevice) {
        let device = FakeDevice::default();
        device.reply(0x02, [input(0xB6, DEVICE_INFO_REPLY)]);
        device.reply(0x03, [input(0xB6, RECEIVER_DEVICES_REPLY)]);
        device.reply(0x04, [input(0xB4, FIRMWARE_VERSION_REPLY)]);
        device.reply(0x06, [input(0xB4, BASE_INFO_REPLY)]);
        device.reply(0x49, [input(0xB4, DPI_TABLE_REPLY)]);
        device.reply(0x4B, [input(0xB6, POLLING_TABLE_REPLY)]);
        let interface = InterfaceInfo {
            path: "turbolink".into(),
            vendor_id: 0x3434,
            product_id: 0xD05A,
            usage_page: 0xFFC1,
            usage: 0x01,
            product: Some("Keychron TurboLink 8K".into()),
        };
        (interface, device)
    }

    fn refreshed() -> (MouseSession, FakeDevice) {
        let (interface, device) = turbolink();
        let mut session = MouseSession::new(device.transport(), &interface);
        session.refresh().unwrap();
        (session, device)
    }

    fn dpi(session: &MouseSession) -> DpiStages {
        session.state().mouse().unwrap().dpi.clone().unwrap()
    }

    #[test]
    fn refresh_reads_the_mouse_behind_the_receiver() {
        // Given the receiver relaying the captured mouse
        // When refreshing the session
        let (session, _) = refreshed();
        // Then the mouse, its battery, DPI stages, polling rates and profiles are known
        let state: &DeviceState = session.state();
        assert_eq!(state.name, "Keychron G6 HE");
        assert_eq!(state.product_id, 0xD09D);
        assert_eq!(
            state.connection,
            Connection::Receiver("Keychron TurboLink 8K".into())
        );
        assert_eq!(state.firmware.as_deref(), Some("1.0.6+0"));
        assert!(state.online);
        let Details::Mouse(mouse) = &state.details else {
            panic!("a mouse");
        };
        assert_eq!(
            mouse.battery,
            Some(Battery {
                percent: 59,
                charging: false
            })
        );
        assert_eq!(
            mouse.dpi,
            Some(DpiStages {
                values: vec![400, 800, 1600, 3200, 5000],
                active: 2
            })
        );
        assert_eq!(
            mouse.polling,
            Some(PollingRates {
                rates: vec![125, 500, 1000, 2000, 4000, 8000],
                active: 2
            })
        );
        assert_eq!(
            mouse.profiles,
            Some(Profiles {
                active: 0,
                count: 5
            })
        );
    }

    #[test]
    fn receiver_without_active_mouse_is_offline() {
        // Given a receiver whose paired mouse is asleep: it lists the mouse as inactive and
        // leaves relayed requests such as device info unanswered
        let (interface, device) = turbolink();
        device.reply(0x03, [input(0xB6, "03 01 34 34 9D D0 00")]);
        device.reply(0x02, []);
        let mut session = MouseSession::new(device.transport(), &interface);
        // When refreshing
        session.refresh().unwrap();
        // Then the mouse is named but offline, and only the receiver was asked
        assert_eq!(session.state().name, "Keychron G6 HE");
        assert!(!session.state().online);
        assert!(device.written().iter().all(|report| report[1] == 0x03));
    }

    #[test]
    fn status_event_updates_dpi_and_battery() {
        // Given a refreshed session and a first status event recording the power state
        let (mut session, device) = refreshed();
        device.push(input(0xB6, STATUS_EVENT_REPORT));
        session.poll(std::time::Duration::ZERO).unwrap();
        // When the mouse reports DPI stage 3 at 58%
        device.push(input(0xB6, "E2 01 01 00 3A 03 02 05 06"));
        let changed = session.poll(std::time::Duration::ZERO).unwrap();
        // Then the state follows without reading the mouse again
        assert!(changed);
        assert_eq!(dpi(&session).active, 3);
        let battery = session.state().mouse().unwrap().battery.unwrap();
        assert_eq!(battery.percent, 58);
        assert_eq!(
            device
                .written()
                .iter()
                .filter(|report| report[1] == 0x06)
                .count(),
            1
        );
    }

    #[test]
    fn power_change_reads_the_charging_state() {
        // Given a refreshed session that saw the mouse on battery
        let (mut session, device) = refreshed();
        device.push(input(0xB6, STATUS_EVENT_REPORT));
        session.poll(std::time::Duration::ZERO).unwrap();
        let mut charging = frame(BASE_INFO_REPLY, 63);
        charging[19] = 0x80 | 0x3B;
        let mut report = vec![0xB4];
        report.extend(charging);
        device.reply(0x06, [report]);
        // When the power state changes
        device.push(input(0xB6, "E2 01 01 01 3B 02 02 05 06"));
        session.poll(std::time::Duration::ZERO).unwrap();
        // Then the base info is read again and shows the mouse charging
        assert!(session.state().mouse().unwrap().battery.unwrap().charging);
    }

    #[test]
    fn disconnection_event_marks_the_mouse_offline() {
        // Given a refreshed session
        let (mut session, device) = refreshed();
        // When the receiver reports that the mouse left
        device.push(input(0xB6, "E2 01 00 00 3B 02 02 05 06"));
        // Then the mouse is offline
        assert!(session.poll(std::time::Duration::ZERO).unwrap());
        assert!(!session.state().online);
    }

    #[test]
    fn reconnection_status_reads_the_mouse_again() {
        // Given a refreshed session at 59% and DPI stage 2
        let (mut session, device) = refreshed();
        // When the mouse reconnects and sends the status without settings captured on wake
        device.push(input(0xB6, "E2 01 01 00 00 00 00 00"));
        session.poll(std::time::Duration::ZERO).unwrap();
        // Then it is read again instead of taking the zeros as 0% and the first stage
        let battery = session.state().mouse().unwrap().battery.unwrap();
        assert_eq!(battery.percent, 59);
        assert_eq!(dpi(&session).active, 2);
        let base_reads = device
            .written()
            .iter()
            .filter(|report| report[1] == 0x06)
            .count();
        assert_eq!(base_reads, 2);
    }

    #[test]
    fn selecting_a_dpi_stage_writes_the_table_and_reads_it_back() {
        // Given a refreshed session whose mouse applies stage 1
        let (mut session, device) = refreshed();
        let mut after = input(0xB4, DPI_TABLE_REPLY);
        after[2..5].fill(1);
        device.reply(0x49, [after]);
        device.reply(0x48, [input(0xB4, "E4 00 48")]);
        // When selecting stage 1
        session.execute(Command::SelectDpiStage(1)).unwrap();
        // Then the full table was written with stage 1 and the new stage is read back
        let write = device
            .written()
            .into_iter()
            .find(|report| report[1] == 0x48)
            .unwrap();
        assert_eq!(&write[..6], &[0xB3, 0x48, 1, 1, 1, 5]);
        assert_eq!(dpi(&session).active, 1);
    }

    #[test]
    fn rejected_and_out_of_range_commands_fail() {
        // Given a refreshed session whose mouse rejects DPI changes
        let (mut session, device) = refreshed();
        device.reply(0x48, [input(0xB4, "E4 05 48")]);
        // When selecting a stage, then a stage that does not exist
        // Then the rejection and the bad index are reported, and nothing bad is written
        assert!(matches!(
            session.execute(Command::SelectDpiStage(0)),
            Err(SessionError::Rejected {
                opcode: 0x48,
                status: 5
            })
        ));
        let writes = device.written().len();
        assert!(matches!(
            session.execute(Command::SelectDpiStage(9)),
            Err(SessionError::Unsupported)
        ));
        assert_eq!(device.written().len(), writes);
    }

    #[test]
    fn unapplied_change_is_reported() {
        // Given a refreshed session whose mouse acknowledges but keeps its polling rate
        let (mut session, device) = refreshed();
        device.reply(0x4A, [input(0xB6, "E4 00 4A")]);
        // When selecting 8000 Hz
        let result = session.execute(Command::SelectPollingRate(5));
        // Then the read-back shows the change was not applied
        assert!(matches!(result, Err(SessionError::NotApplied)));
    }
}
