//! A value computed on a background thread, polled from the UI.

use std::time::Instant;

pub struct Task<T> {
    rx: Option<flume::Receiver<T>>,
    pub value: Option<T>,
    pub finished_at: Option<Instant>,
}

impl<T> Default for Task<T> {
    fn default() -> Self {
        Self {
            rx: None,
            value: None,
            finished_at: None,
        }
    }
}

impl<T: Send + 'static> Task<T> {
    /// Start (or restart) the computation. The previous value stays visible
    /// until the new one arrives.
    pub fn spawn(&mut self, ctx: &egui::Context, f: impl FnOnce() -> T + Send + 'static) {
        let (tx, rx) = flume::bounded(1);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(f());
            ctx.request_repaint();
        });
        self.rx = Some(rx);
    }

    /// Pick up a finished result. Returns true when a new value arrived.
    pub fn poll(&mut self) -> bool {
        if let Some(rx) = &self.rx {
            if let Ok(v) = rx.try_recv() {
                self.value = Some(v);
                self.rx = None;
                self.finished_at = Some(Instant::now());
                return true;
            }
            if rx.is_disconnected() {
                self.rx = None;
            }
        }
        false
    }

    pub fn running(&self) -> bool {
        self.rx.is_some()
    }

    /// Never run yet and not running: start it. Then poll.
    pub fn ensure(&mut self, ctx: &egui::Context, f: impl FnOnce() -> T + Send + 'static) {
        if self.value.is_none() && self.rx.is_none() {
            self.spawn(ctx, f);
        }
        self.poll();
    }

    /// Take the value out (for one-shot results like "command finished").
    pub fn take(&mut self) -> Option<T> {
        self.poll();
        self.value.take()
    }
}
