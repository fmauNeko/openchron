//! Scripted HID backend and devices for tests.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use super::{Backend, BackendFactory, HidError, InterfaceInfo, Transport};

/// A scripted device. Writing a report queues the next scripted reply for its opcode
/// (report byte 1); the last reply of a script repeats.
#[derive(Clone, Default)]
pub struct FakeDevice(Arc<Mutex<DeviceScript>>);

#[derive(Default)]
struct DeviceScript {
    replies: HashMap<u8, VecDeque<Vec<u8>>>,
    inbox: VecDeque<Vec<u8>>,
    written: Vec<Vec<u8>>,
    unplugged: bool,
}

impl FakeDevice {
    fn script(&self) -> std::sync::MutexGuard<'_, DeviceScript> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Answers requests with `opcode` by the input reports `replies`, in order.
    pub fn reply(&self, opcode: u8, replies: impl IntoIterator<Item = Vec<u8>>) {
        self.script()
            .replies
            .insert(opcode, replies.into_iter().collect());
    }

    /// Queues an input report the device sends on its own.
    pub fn push(&self, report: Vec<u8>) {
        self.script().inbox.push_back(report);
    }

    /// Reports written so far.
    #[must_use]
    pub fn written(&self) -> Vec<Vec<u8>> {
        self.script().written.clone()
    }

    /// Makes every later read and write fail, like a device that was unplugged.
    pub fn unplug(&self) {
        self.script().unplugged = true;
    }

    /// An open transport to this device.
    #[must_use]
    pub fn transport(&self) -> Box<dyn Transport> {
        Box::new(self.clone())
    }
}

impl Transport for FakeDevice {
    fn write(&mut self, report: &[u8]) -> Result<(), HidError> {
        let mut script = self.script();
        if script.unplugged {
            return Err(HidError("unplugged".into()));
        }
        script.written.push(report.to_vec());
        let opcode = report.get(1).copied().unwrap_or_default();
        let reply = script.replies.get_mut(&opcode).and_then(|replies| {
            if replies.len() > 1 {
                replies.pop_front()
            } else {
                replies.front().cloned()
            }
        });
        script.inbox.extend(reply);
        Ok(())
    }

    fn read(&mut self, buf: &mut [u8], _timeout: Duration) -> Result<usize, HidError> {
        let mut script = self.script();
        if script.unplugged {
            return Err(HidError("unplugged".into()));
        }
        let Some(report) = script.inbox.pop_front() else {
            return Ok(0);
        };
        let len = report.len().min(buf.len());
        buf[..len].copy_from_slice(&report[..len]);
        Ok(len)
    }
}

/// A scripted backend whose interfaces tests plug and unplug.
#[derive(Clone, Default)]
pub struct FakeBackend(Arc<Mutex<Vec<(InterfaceInfo, FakeDevice)>>>);

impl FakeBackend {
    fn interfaces(&self) -> std::sync::MutexGuard<'_, Vec<(InterfaceInfo, FakeDevice)>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Connects `device` through `interface`.
    pub fn plug(&self, interface: InterfaceInfo, device: FakeDevice) {
        self.interfaces().push((interface, device));
    }

    /// Disconnects the interface at `path`.
    pub fn unplug(&self, path: &str) {
        self.interfaces().retain(|(interface, device)| {
            if interface.path == path {
                device.unplug();
            }
            interface.path != path
        });
    }

    /// A factory handing out this backend.
    pub fn factory(&self) -> BackendFactory {
        let backend = self.clone();
        Box::new(move || Ok(Box::new(backend) as Box<dyn Backend>))
    }
}

impl Backend for FakeBackend {
    fn enumerate(&mut self) -> Result<Vec<InterfaceInfo>, HidError> {
        Ok(self
            .interfaces()
            .iter()
            .map(|(interface, _)| interface.clone())
            .collect())
    }

    fn open(&mut self, interface: &InterfaceInfo) -> Result<Box<dyn Transport>, HidError> {
        self.interfaces()
            .iter()
            .find(|(candidate, _)| candidate.path == interface.path)
            .map(|(_, device)| device.transport())
            .ok_or_else(|| HidError("no such interface".into()))
    }
}
