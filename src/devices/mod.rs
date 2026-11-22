//! Devices: HID worker threads that talk to each Keychron interface, and the app-level
//! [`store::DeviceStore`] that collects what they report.
//!
//! The [`manager`] thread enumerates interfaces every few seconds and starts a [`worker`]
//! thread per supported interface. Each worker owns a [`session::Session`], which speaks the
//! device protocol, and sends [`model::DeviceUpdate`]s through an [`model::UpdateSink`]; the
//! store applies them on the GPUI thread and emits [`store::DeviceStoreEvent`]s.

pub mod catalog;
pub mod keyboard;
pub mod manager;
pub mod model;
pub mod mouse;
pub mod session;
pub mod store;
pub mod worker;
