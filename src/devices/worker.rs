//! Device worker: the thread that runs one session and reports its state.

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use super::model::{Command, DeviceId, DeviceState, DeviceUpdate, UpdateSink};
use super::session::Session;

/// How long one wait for an unsolicited report lasts; commands wait at most this long.
const POLL_TIMEOUT: Duration = Duration::from_millis(100);
/// How often the whole state is read again, in case an event was missed.
const REFRESH_INTERVAL: Duration = Duration::from_secs(60);

/// Starts a worker thread for `session`; it runs until `stop` is set or the device is gone.
///
/// # Errors
/// Fails when the thread cannot be created.
pub fn spawn(
    session: Session,
    id: DeviceId,
    commands: Receiver<Command>,
    stop: Arc<AtomicBool>,
    sink: UpdateSink,
) -> io::Result<JoinHandle<()>> {
    thread::Builder::new()
        .name("openchron-device".into())
        .spawn(move || run(session, &id, &commands, &stop, &*sink))
}

/// The worker loop: reads the device, then applies commands and unsolicited reports,
/// publishing the state whenever it changes. Ends with [`DeviceUpdate::Detached`].
pub fn run(
    mut session: Session,
    id: &DeviceId,
    commands: &Receiver<Command>,
    stop: &AtomicBool,
    sink: &(dyn Fn(DeviceUpdate) + Send + Sync),
) {
    let mut published: Option<DeviceState> = None;
    let mut publish = |session: &Session| {
        if published.as_ref() != Some(session.state()) {
            published = Some(session.state().clone());
            sink(DeviceUpdate::State {
                id: id.clone(),
                state: session.state().clone(),
            });
        }
    };
    let mut alive = !session.refresh().is_err_and(|error| error.is_fatal());
    publish(&session);
    let mut refreshed = Instant::now();
    while alive && !stop.load(Ordering::Relaxed) {
        let result = match commands.try_recv() {
            Ok(command) => match session.execute(command) {
                Err(error) if !error.is_fatal() => {
                    sink(DeviceUpdate::CommandFailed {
                        id: id.clone(),
                        error: error.to_string(),
                    });
                    Ok(())
                }
                result => result,
            },
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => {
                if refreshed.elapsed() >= REFRESH_INTERVAL {
                    refreshed = Instant::now();
                    session.refresh()
                } else {
                    session.poll(POLL_TIMEOUT).map(drop)
                }
            }
        };
        alive = !result.is_err_and(|error| error.is_fatal());
        publish(&session);
    }
    sink(DeviceUpdate::Detached { id: id.clone() });
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;
    use std::sync::{Mutex, mpsc};

    use super::run;
    use crate::devices::model::{Command, DeviceId, DeviceUpdate};
    use crate::devices::mouse::tests::turbolink;
    use crate::devices::session::Session;
    use crate::hid::Protocol;

    #[test]
    fn worker_publishes_state_then_detaches_when_unplugged() {
        // Given a session on the receiver, a queued command, and a device unplugged after it
        let (interface, device) = turbolink();
        let session = Session::open(Protocol::Mouse, device.transport(), &interface);
        let (commands, receiver) = mpsc::channel();
        commands.send(Command::SelectProfile(9)).unwrap();
        let updates = Mutex::new(Vec::new());
        let sink = |update: DeviceUpdate| {
            if matches!(update, DeviceUpdate::CommandFailed { .. }) {
                device.unplug();
            }
            updates.lock().unwrap().push(update);
        };
        // When running the worker
        run(
            session,
            &DeviceId("turbolink".into()),
            &receiver,
            &AtomicBool::new(false),
            &sink,
        );
        // Then it published the state, reported the failed command, and detached
        let updates = updates.into_inner().unwrap();
        assert!(matches!(&updates[0], DeviceUpdate::State { state, .. } if state.online));
        assert!(matches!(&updates[1], DeviceUpdate::CommandFailed { .. }));
        assert!(matches!(
            updates.last(),
            Some(DeviceUpdate::Detached { .. })
        ));
        assert_eq!(updates.len(), 3);
    }
}
