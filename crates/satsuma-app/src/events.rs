//! UI events posted by views and handled by the controller on the next timer tick.
//! Queuing (instead of calling the controller directly) avoids re-entrancy while AppKit is
//! in the middle of a drag or a drawing pass.

use objc2_foundation::NSPoint;
use satsuma_core::{WheelItem, WheelMode};
use satsuma_engine::Operation;
use std::cell::RefCell;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub enum UiEvent {
    /// Files dropped on a wheel petal.
    WheelDrop { item: WheelItem, files: Vec<PathBuf>, at: NSPoint },
    /// Petal chosen with the keyboard or a click (no drag).
    WheelPick { item: WheelItem },
    WheelCancel,
    WheelSwitch(WheelMode),
    /// The × on the progress HUD.
    HudClose,
    /// Files dropped onto a Float shelf, identified by its window number.
    FloatAdd { window: isize, files: Vec<PathBuf> },
    FloatRemoveSelected { window: isize },
    /// A Float shelf or editor window closed.
    WindowClosed { window: isize },
    /// An editor window's Apply / Save button.
    EditorApply { op: Operation, files: Vec<PathBuf>, at: NSPoint },
    WindowResignedKey { window: isize },
}

thread_local! {
    static QUEUE: RefCell<Vec<UiEvent>> = const { RefCell::new(Vec::new()) };
}

pub fn post(event: UiEvent) {
    QUEUE.with(|q| q.borrow_mut().push(event));
}

pub fn drain() -> Vec<UiEvent> {
    QUEUE.with(|q| std::mem::take(&mut *q.borrow_mut()))
}
