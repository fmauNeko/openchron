//! Device manager: the thread that enumerates Keychron interfaces and runs a worker for
//! each supported one. `hidapi` has no hotplug notifications, so it rescans periodically.

use std::collections::{HashMap, HashSet};
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::hid::{Backend, BackendFactory};

use super::catalog;
use super::model::{DeviceId, DeviceUpdate, UpdateSink};
use super::session::Session;
use super::worker;

/// Handle to the manager thread; dropping it stops the manager and its workers.
pub struct Manager {
    stop: Option<mpsc::Sender<()>>,
    _thread: JoinHandle<()>,
}

impl Manager {
    /// Starts the manager thread, which creates its backend with `factory` and scans every
    /// `interval`.
    ///
    /// # Errors
    /// Fails when the thread cannot be created.
    pub fn start(
        factory: BackendFactory,
        sink: UpdateSink,
        interval: Duration,
    ) -> io::Result<Self> {
        let (stop, stopped) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("openchron-devices".into())
            .spawn(move || run(factory, &sink, &stopped, interval))?;
        Ok(Self {
            stop: Some(stop),
            _thread: thread,
        })
    }
}

impl Drop for Manager {
    fn drop(&mut self) {
        // Closing the channel wakes the manager, which stops its workers on the way out.
        drop(self.stop.take());
    }
}

/// A worker thread; dropping it asks the worker to stop.
struct Worker {
    stop: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn run(
    factory: BackendFactory,
    sink: &UpdateSink,
    stopped: &mpsc::Receiver<()>,
    interval: Duration,
) {
    let mut backend = match factory() {
        Ok(backend) => backend,
        Err(error) => {
            sink(DeviceUpdate::BackendFailed(error.to_string()));
            return;
        }
    };
    let mut workers = HashMap::new();
    let mut unavailable = HashSet::new();
    loop {
        scan(backend.as_mut(), &mut workers, &mut unavailable, sink);
        match stopped.recv_timeout(interval) {
            Err(RecvTimeoutError::Timeout) => {}
            Ok(()) | Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}

/// One scan: stops the workers of vanished interfaces and starts workers for new ones.
fn scan(
    backend: &mut dyn Backend,
    workers: &mut HashMap<String, Worker>,
    unavailable: &mut HashSet<String>,
    sink: &UpdateSink,
) {
    let Ok(interfaces) = backend.enumerate() else {
        return;
    };
    let present: HashSet<&str> = interfaces
        .iter()
        .map(|interface| interface.path.as_str())
        .collect();
    for (path, worker) in workers.iter() {
        if !present.contains(path.as_str()) {
            worker.stop.store(true, Ordering::Relaxed);
        }
    }
    // A worker leaves the map only once its thread has ended, after its `Detached`: a
    // replugged interface never gets a new worker before the old one has detached.
    workers.retain(|_, worker| !worker.thread.is_finished());
    unavailable.retain(|path| present.contains(path.as_str()));
    for interface in &interfaces {
        let Some(protocol) = interface.protocol() else {
            continue;
        };
        if workers.contains_key(&interface.path) || unavailable.contains(&interface.path) {
            continue;
        }
        let id = DeviceId(interface.path.clone());
        let name = catalog::name(interface.product_id, interface.product.as_deref(), "device");
        let transport = match backend.open(interface) {
            Ok(transport) => transport,
            Err(error) => {
                unavailable.insert(interface.path.clone());
                sink(DeviceUpdate::Unavailable {
                    id,
                    name,
                    error: error.to_string(),
                });
                continue;
            }
        };
        let (commands, receiver) = mpsc::channel();
        sink(DeviceUpdate::Attached {
            id: id.clone(),
            name,
            commands,
        });
        let stop = Arc::new(AtomicBool::new(false));
        let session = Session::open(protocol, transport, interface);
        match worker::spawn(session, id.clone(), receiver, stop.clone(), sink.clone()) {
            Ok(thread) => {
                workers.insert(interface.path.clone(), Worker { stop, thread });
            }
            Err(error) => sink(DeviceUpdate::CommandFailed {
                id,
                error: error.to_string(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, mpsc};
    use std::time::Duration;

    use super::Manager;
    use crate::devices::model::{DeviceId, DeviceUpdate, UpdateSink};
    use crate::devices::mouse::tests::turbolink;
    use crate::hid::fake::FakeBackend;
    use crate::hid::{BackendFactory, HidError, InterfaceInfo};

    /// Bound on every wait for a manager event.
    const WAIT: Duration = Duration::from_secs(10);

    fn start(factory: BackendFactory) -> (Manager, mpsc::Receiver<DeviceUpdate>) {
        let (sender, receiver) = mpsc::channel();
        // Updates sent after the test returned have nobody to receive them.
        let sink: UpdateSink = Arc::new(move |update| drop(sender.send(update)));
        let manager = Manager::start(factory, sink, Duration::from_millis(5)).unwrap();
        (manager, receiver)
    }

    fn next(receiver: &mpsc::Receiver<DeviceUpdate>) -> DeviceUpdate {
        receiver.recv_timeout(WAIT).expect("a device update")
    }

    #[test]
    fn plugged_interfaces_get_a_worker_until_unplugged() {
        // Given a backend with the receiver and an interface Openchron does not use
        let backend = FakeBackend::default();
        let (interface, device) = turbolink();
        backend.plug(interface.clone(), device);
        backend.plug(
            InterfaceInfo {
                path: "keyboard-input".into(),
                usage_page: 0x0001,
                usage: 0x06,
                ..interface
            },
            crate::hid::fake::FakeDevice::default(),
        );
        // When the manager starts
        let (_manager, receiver) = start(backend.factory());
        // Then only the receiver is attached and reports its state
        let id = DeviceId("turbolink".into());
        assert!(
            matches!(next(&receiver), DeviceUpdate::Attached { id: attached, .. } if attached == id)
        );
        assert!(matches!(next(&receiver), DeviceUpdate::State { state, .. } if state.online));
        // When the receiver is unplugged
        backend.unplug("turbolink");
        // Then its worker detaches
        assert!(
            matches!(next(&receiver), DeviceUpdate::Detached { id: detached } if detached == id)
        );
    }

    #[test]
    fn backend_failure_is_reported() {
        // Given a HID stack that cannot start
        let factory: BackendFactory = Box::new(|| Err(HidError("no HID".into())));
        // When the manager starts
        let (_manager, receiver) = start(factory);
        // Then the failure is reported
        assert!(matches!(next(&receiver), DeviceUpdate::BackendFailed(error) if error == "no HID"));
    }
}
