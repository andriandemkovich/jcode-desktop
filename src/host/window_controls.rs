//! Host-owned window actions shared by every reloadable tab bar.
use std::{cell::RefCell, rc::Rc};

use gpui::{AnyWindowHandle, App, actions};

use super::reload::ReloadManager;

actions!(jcode_desktop_host, [CloseWindow, MinimizeWindow]);

pub fn install(
    cx: &mut App,
    manager: Rc<RefCell<ReloadManager>>,
    current: Rc<RefCell<Option<AnyWindowHandle>>>,
) {
    // The titlebar is transparent, so the platform's own minimize button never
    // appears. Without a host action the tab bar can only close the window,
    // which discards the visible workspace instead of setting it aside.
    cx.on_action({
        let current = current.clone();
        move |_: &MinimizeWindow, cx| {
            let current = current.clone();
            cx.defer(move |cx| {
                let Some(handle) = *current.borrow() else {
                    return;
                };
                let _ = handle.update(cx, |_, window, _| window.minimize_window());
            });
        }
    });

    cx.on_action(move |_: &CloseWindow, cx| {
        let manager = manager.clone();
        let current = current.clone();
        // Action dispatch holds the window update. Release it before entering
        // the host's snapshot path, just as native close notifications do.
        cx.defer(move |cx| {
            let Some(handle) = *current.borrow() else {
                return;
            };
            let _ = handle.update(cx, |_, window, cx| {
                if let Err(error) = manager.borrow_mut().suspend(window, cx) {
                    eprintln!("failed to suspend desktop workspace: {error:#}");
                    return;
                }
                *current.borrow_mut() = None;
                window.remove_window();
            });
        });
    });
}
