//! System notifications, behind a hook that tests replace with a recorder.

use std::rc::Rc;

use gpui_kit::{App, Global, SystemNotification};

/// Hook showing a notification.
pub type NotifyHook = Rc<dyn Fn(&mut App, SystemNotification)>;

/// Shows a notification. The default hook posts it to the operating system.
#[derive(Clone)]
pub struct Notifier(NotifyHook);

impl Global for Notifier {}

/// Installs the notifier that posts to the operating system's notification center.
pub fn install(cx: &mut App) {
    cx.set_global(Notifier(Rc::new(|cx, notification| {
        cx.show_system_notification(notification);
    })));
}

/// Shows a notification through the installed hook.
pub fn show(cx: &mut App, notification: SystemNotification) {
    let notifier = cx.global::<Notifier>().0.clone();
    notifier(cx, notification);
}

/// Installs a notifier that records notifications instead of showing them.
#[cfg(test)]
pub fn install_recorder(cx: &mut App) -> Rc<std::cell::RefCell<Vec<SystemNotification>>> {
    let shown = Rc::new(std::cell::RefCell::new(Vec::new()));
    let log = shown.clone();
    cx.set_global(Notifier(Rc::new(move |_, notification| {
        log.borrow_mut().push(notification);
    })));
    shown
}
