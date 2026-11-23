//! Device store: the app-level list of devices, fed by the HID threads.

use std::sync::{Arc, mpsc};
use std::time::Duration;

use futures::StreamExt as _;
use gpui_kit::prelude::*;
use gpui_kit::{App, Context, Entity, EventEmitter, Global, Task};

use crate::hid::BackendFactory;

use super::manager::Manager;
use super::model::{Battery, Command, DeviceId, DeviceState, DeviceUpdate, DpiStages, UpdateSink};

/// How often the manager looks for plugged and unplugged devices.
const SCAN_INTERVAL: Duration = Duration::from_secs(2);

/// A device the HID threads reported.
#[derive(Debug)]
pub struct Device {
    /// Identity: the HID interface path.
    pub id: DeviceId,
    /// Display name.
    pub name: String,
    /// The last reported state; `None` until the worker reads the device.
    pub state: Option<DeviceState>,
    /// Why the device cannot be used, such as a permission error.
    pub error: Option<String>,
    commands: Option<mpsc::Sender<Command>>,
}

/// Change notification emitted by [`DeviceStore`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceStoreEvent {
    /// The device list or a device state changed.
    Changed,
    /// A device's active DPI stage changed, on the device or from Openchron.
    DpiChanged {
        /// The device.
        id: DeviceId,
        /// Its display name.
        name: String,
        /// Its stages, with the new active one.
        stages: DpiStages,
    },
    /// A device reported a new battery reading.
    BatteryChanged {
        /// The device.
        id: DeviceId,
        /// Its display name.
        name: String,
        /// The reading.
        battery: Battery,
    },
    /// A device could not apply a command.
    CommandFailed {
        /// The device.
        id: DeviceId,
        /// Its display name.
        name: String,
        /// What went wrong.
        error: String,
    },
}

/// GPUI entity holding the devices.
#[derive(Default)]
pub struct DeviceStore {
    devices: Vec<Device>,
    backend_error: Option<String>,
    _manager: Option<Manager>,
    _bridge: Option<Task<()>>,
}

impl EventEmitter<DeviceStoreEvent> for DeviceStore {}

/// GPUI global handle to the device store.
pub struct GlobalDevices(pub Entity<DeviceStore>);

impl Global for GlobalDevices {}

impl DeviceStore {
    /// The devices, in the order they were found.
    #[must_use]
    pub fn devices(&self) -> &[Device] {
        &self.devices
    }

    /// The device with this ID.
    #[must_use]
    pub fn device(&self, id: &DeviceId) -> Option<&Device> {
        self.devices.iter().find(|device| &device.id == id)
    }

    /// Why devices cannot be listed at all, if the HID stack failed.
    #[must_use]
    pub fn backend_error(&self) -> Option<&str> {
        self.backend_error.as_deref()
    }

    /// Sends a command to a device's worker; returns false when the device is gone.
    #[must_use]
    pub fn send(&self, id: &DeviceId, command: Command) -> bool {
        self.device(id)
            .and_then(|device| device.commands.as_ref())
            .is_some_and(|commands| commands.send(command).is_ok())
    }

    fn entry(&mut self, id: &DeviceId, name: &str) -> &mut Device {
        if let Some(index) = self.devices.iter().position(|device| &device.id == id) {
            return &mut self.devices[index];
        }
        self.devices.push(Device {
            id: id.clone(),
            name: name.to_owned(),
            state: None,
            error: None,
            commands: None,
        });
        let last = self.devices.len() - 1;
        &mut self.devices[last]
    }

    /// Applies an update from the HID threads and emits the matching events.
    pub fn apply(&mut self, update: DeviceUpdate, cx: &mut Context<Self>) {
        match update {
            DeviceUpdate::Attached { id, name, commands } => {
                let device = self.entry(&id, &name);
                device.commands = Some(commands);
                device.error = None;
            }
            DeviceUpdate::State { id, state } => {
                let device = self.entry(&id, &state.name);
                let previous = device.state.replace(state.clone());
                device.name.clone_from(&state.name);
                for event in transitions(&id, previous.as_ref(), &state) {
                    cx.emit(event);
                }
            }
            DeviceUpdate::CommandFailed { id, error } => {
                let name = self
                    .device(&id)
                    .map_or_else(|| id.0.clone(), |device| device.name.clone());
                cx.emit(DeviceStoreEvent::CommandFailed { id, name, error });
            }
            DeviceUpdate::Unavailable { id, name, error } => {
                let device = self.entry(&id, &name);
                device.error = Some(error);
                device.commands = None;
            }
            DeviceUpdate::Detached { id } => self.devices.retain(|device| device.id != id),
            DeviceUpdate::BackendFailed(error) => self.backend_error = Some(error),
        }
        cx.emit(DeviceStoreEvent::Changed);
        cx.notify();
    }
}

/// Events for the change from `previous` to `current`.
fn transitions(
    id: &DeviceId,
    previous: Option<&DeviceState>,
    current: &DeviceState,
) -> Vec<DeviceStoreEvent> {
    let mut events = Vec::new();
    let Some(mouse) = current.mouse().filter(|_| current.online) else {
        return events;
    };
    let before = previous
        .filter(|previous| previous.online)
        .and_then(DeviceState::mouse);
    if let Some(battery) = mouse.battery
        && before.and_then(|before| before.battery) != Some(battery)
    {
        events.push(DeviceStoreEvent::BatteryChanged {
            id: id.clone(),
            name: current.name.clone(),
            battery,
        });
    }
    // The first reading is not a change: only a stage switch shows the DPI popup.
    if let (Some(stages), Some(before)) = (&mouse.dpi, before.and_then(|b| b.dpi.as_ref()))
        && stages.active != before.active
    {
        events.push(DeviceStoreEvent::DpiChanged {
            id: id.clone(),
            name: current.name.clone(),
            stages: stages.clone(),
        });
    }
    events
}

impl DeviceStore {
    /// A store fed by HID threads using `factory`, or by tests when there is none.
    fn start(factory: Option<BackendFactory>, cx: &mut Context<Self>) -> Self {
        let Some(factory) = factory else {
            return Self::default();
        };
        let (sender, mut receiver) = futures::channel::mpsc::unbounded();
        // The receiver only closes when the app shuts the store down.
        let sink: UpdateSink = Arc::new(move |update| drop(sender.unbounded_send(update)));
        let manager = match Manager::start(factory, sink, SCAN_INTERVAL) {
            Ok(manager) => manager,
            Err(error) => {
                return Self {
                    backend_error: Some(error.to_string()),
                    ..Self::default()
                };
            }
        };
        let bridge = cx.spawn(async move |this, cx| {
            while let Some(update) = receiver.next().await {
                if this
                    .update(cx, |store, cx| store.apply(update, cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            devices: Vec::new(),
            backend_error: None,
            _manager: Some(manager),
            _bridge: Some(bridge),
        }
    }
}

/// Installs the store as a GPUI global. With a backend, starts the HID threads and feeds
/// their updates to the store; tests pass `None` and apply updates themselves.
pub fn init(cx: &mut App, backend: Option<BackendFactory>) {
    let store = cx.new(|cx| DeviceStore::start(backend, cx));
    cx.set_global(GlobalDevices(store));
}

/// Stops the HID threads.
pub fn shutdown(cx: &mut App) {
    if let Some(GlobalDevices(store)) = cx.try_global::<GlobalDevices>() {
        let store = store.clone();
        store.update(cx, |store, _| *store = DeviceStore::default());
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::mpsc;

    use gpui_kit::{App, TestAppContext};

    use super::{DeviceStore, DeviceStoreEvent, GlobalDevices, init};
    use crate::devices::model::{
        Command, Connection, Details, DeviceId, DeviceState, DeviceUpdate, DpiStages, MouseState,
    };
    use crate::protocol::Battery;

    /// A mouse online through a receiver, at `percent` and DPI stage `stage`.
    pub(crate) fn mouse(percent: u8, stage: usize) -> DeviceState {
        DeviceState {
            name: "Keychron G6 HE".into(),
            product_id: 0xD09D,
            connection: Connection::Receiver("Keychron TurboLink 8K".into()),
            firmware: Some("1.0.6+0".into()),
            online: true,
            details: Details::Mouse(MouseState {
                battery: Some(Battery {
                    percent,
                    charging: false,
                }),
                dpi: Some(DpiStages {
                    values: vec![400, 800, 1600, 3200, 5000],
                    active: stage,
                }),
                polling: None,
                profiles: None,
            }),
        }
    }

    /// Applies an update to the global store.
    pub(crate) fn apply(cx: &mut App, update: DeviceUpdate) {
        let store = cx.global::<GlobalDevices>().0.clone();
        store.update(cx, |store, cx| store.apply(update, cx));
    }

    fn recorded(cx: &mut TestAppContext) -> std::rc::Rc<std::cell::RefCell<Vec<DeviceStoreEvent>>> {
        let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let log = events.clone();
        cx.update(|cx| {
            init(cx, None);
            let store = cx.global::<GlobalDevices>().0.clone();
            cx.subscribe(&store, move |_, event: &DeviceStoreEvent, _| {
                if *event != DeviceStoreEvent::Changed {
                    log.borrow_mut().push(event.clone());
                }
            })
            .detach();
        });
        events
    }

    fn id() -> DeviceId {
        DeviceId("turbolink".into())
    }

    #[gpui_kit::test]
    fn first_state_reports_the_battery_but_not_a_dpi_change(cx: &mut TestAppContext) {
        // Given an empty store
        let events = recorded(cx);
        // When the first state of a mouse arrives
        cx.update(|cx| {
            apply(
                cx,
                DeviceUpdate::State {
                    id: id(),
                    state: mouse(59, 2),
                },
            );
        });
        // Then the device is listed and its battery reported, without a DPI change
        assert_eq!(
            *events.borrow(),
            [DeviceStoreEvent::BatteryChanged {
                id: id(),
                name: "Keychron G6 HE".into(),
                battery: Battery {
                    percent: 59,
                    charging: false
                }
            }]
        );
        cx.update(|cx| {
            let store = cx.global::<GlobalDevices>().0.read(cx);
            assert_eq!(store.devices().len(), 1);
            assert_eq!(store.devices()[0].name, "Keychron G6 HE");
        });
    }

    #[gpui_kit::test]
    fn stage_switch_reports_a_dpi_change(cx: &mut TestAppContext) {
        // Given a known mouse at stage 2
        let events = recorded(cx);
        cx.update(|cx| {
            apply(
                cx,
                DeviceUpdate::State {
                    id: id(),
                    state: mouse(59, 2),
                },
            );
        });
        events.borrow_mut().clear();
        // When it switches to stage 3, then reports the same state again
        cx.update(|cx| {
            apply(
                cx,
                DeviceUpdate::State {
                    id: id(),
                    state: mouse(59, 3),
                },
            );
            apply(
                cx,
                DeviceUpdate::State {
                    id: id(),
                    state: mouse(59, 3),
                },
            );
        });
        // Then exactly one DPI change is reported, carrying the new stage
        assert_eq!(
            *events.borrow(),
            [DeviceStoreEvent::DpiChanged {
                id: id(),
                name: "Keychron G6 HE".into(),
                stages: DpiStages {
                    values: vec![400, 800, 1600, 3200, 5000],
                    active: 3
                }
            }]
        );
    }

    #[gpui_kit::test]
    fn waking_up_is_not_a_dpi_change(cx: &mut TestAppContext) {
        // Given a mouse that went offline at stage 2
        let events = recorded(cx);
        let mut asleep = mouse(59, 2);
        asleep.online = false;
        cx.update(|cx| {
            apply(
                cx,
                DeviceUpdate::State {
                    id: id(),
                    state: mouse(59, 2),
                },
            );
            apply(
                cx,
                DeviceUpdate::State {
                    id: id(),
                    state: asleep,
                },
            );
        });
        events.borrow_mut().clear();
        // When it comes back at stage 3
        cx.update(|cx| {
            apply(
                cx,
                DeviceUpdate::State {
                    id: id(),
                    state: mouse(59, 3),
                },
            );
        });
        // Then its battery is reported again, but no popup-worthy DPI change
        assert!(matches!(
            events.borrow().as_slice(),
            [DeviceStoreEvent::BatteryChanged { .. }]
        ));
    }

    #[gpui_kit::test]
    fn commands_reach_the_worker_until_detached(cx: &mut TestAppContext) {
        // Given an attached device
        recorded(cx);
        let (commands, worker) = mpsc::channel();
        cx.update(|cx| {
            apply(
                cx,
                DeviceUpdate::Attached {
                    id: id(),
                    name: "Keychron TurboLink 8K".into(),
                    commands,
                },
            );
        });
        // When sending a command, detaching the device, and sending again
        let (sent, after) = cx.update(|cx| {
            let sent = cx
                .global::<GlobalDevices>()
                .0
                .read(cx)
                .send(&id(), Command::SelectDpiStage(1));
            apply(cx, DeviceUpdate::Detached { id: id() });
            let store: &DeviceStore = cx.global::<GlobalDevices>().0.read(cx);
            (sent, store.send(&id(), Command::Refresh))
        });
        // Then only the first command reached the worker
        assert!(sent);
        assert!(!after);
        assert_eq!(worker.try_recv(), Ok(Command::SelectDpiStage(1)));
        assert!(worker.try_recv().is_err());
    }
}
