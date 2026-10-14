//! Read QR code in the overlay and the Edit window (Q, the Copy text
//! menu): the selection is exported and its codes read off the UI thread
//! ([`crate::qr_ui`]); no history entry, no other task.

use super::{Edit, FRect, Task};

impl Edit {
    /// Read the QR codes in the selection (all of the capture if none).
    pub(super) fn read_qr(&mut self) {
        if self.sel.is_none() && self.host.is_none() {
            self.sel = Some(FRect { x: 0.0, y: 0.0, w: self.shot.size.0 as f32, h: self.shot.size.1 as f32 });
        }
        self.tasks = vec![Task::Qr];
        self.done = true;
        self.cancelled = false;
    }
}
