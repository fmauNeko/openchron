//! Devices: sessions that speak each Keychron protocol, and the state they report.
//!
//! A [`session::Session`] owns an open HID interface; [`mouse`] and [`keyboard`] keep the
//! device's [`model::DeviceState`] in sync with it.

pub mod catalog;
pub mod keyboard;
pub mod model;
pub mod mouse;
pub mod session;
