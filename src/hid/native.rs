//! [`Backend`] on `hidapi`.

use std::ffi::CString;
use std::time::Duration;

use hidapi::{HidApi, HidDevice};

use super::{Backend, BackendFactory, HidError, InterfaceInfo, KEYCHRON_VENDOR_ID, Transport};

/// The platform HID stack, through `hidapi`.
pub struct NativeBackend {
    api: HidApi,
}

impl NativeBackend {
    /// Initializes `hidapi`.
    ///
    /// # Errors
    /// Fails when the platform HID stack is unavailable.
    pub fn new() -> Result<Self, HidError> {
        // Each scan lists Keychron devices only, see `enumerate`.
        HidApi::disable_device_discovery();
        let api = HidApi::new().map_err(error)?;
        Ok(Self { api })
    }

    /// A factory creating this backend on the thread that calls it.
    pub fn factory() -> BackendFactory {
        Box::new(|| Self::new().map(|backend| Box::new(backend) as Box<dyn Backend>))
    }
}

impl Backend for NativeBackend {
    fn enumerate(&mut self) -> Result<Vec<InterfaceInfo>, HidError> {
        self.api.reset_devices().map_err(error)?;
        self.api.add_devices(KEYCHRON_VENDOR_ID, 0).map_err(error)?;
        Ok(self
            .api
            .device_list()
            .map(|device| InterfaceInfo {
                path: device.path().to_string_lossy().into_owned(),
                vendor_id: device.vendor_id(),
                product_id: device.product_id(),
                usage_page: device.usage_page(),
                usage: device.usage(),
                product: device.product_string().map(str::to_owned),
            })
            .collect())
    }

    fn open(&mut self, interface: &InterfaceInfo) -> Result<Box<dyn Transport>, HidError> {
        let path =
            CString::new(interface.path.as_str()).map_err(|error| HidError(error.to_string()))?;
        let device = self.api.open_path(&path).map_err(error)?;
        Ok(Box::new(NativeTransport(device)))
    }
}

struct NativeTransport(HidDevice);

impl Transport for NativeTransport {
    fn write(&mut self, report: &[u8]) -> Result<(), HidError> {
        self.0.write(report).map(drop).map_err(error)
    }

    fn read(&mut self, buf: &mut [u8], timeout: Duration) -> Result<usize, HidError> {
        let millis = i32::try_from(timeout.as_millis()).unwrap_or(i32::MAX);
        self.0.read_timeout(buf, millis).map_err(error)
    }
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "used as a map_err adapter, which passes the error by value"
)]
fn error(error: hidapi::HidError) -> HidError {
    HidError(error.to_string())
}
