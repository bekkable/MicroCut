//! Probing a file on a worker thread so the UI never waits for ffprobe.

use std::panic::{self, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

use eframe::egui;
use microcut_media::{MediaBackend, MediaInfo};

/// Result of a probe, with the error already turned into user-facing text.
pub type ProbeResult = Result<MediaInfo, String>;

/// A running probe. Dropping it abandons the result (the worker finishes on its own).
pub struct ProbeJob {
    path: PathBuf,
    rx: Receiver<ProbeResult>,
}

impl ProbeJob {
    /// Starts probing `path`; requests a repaint of `ctx` when done.
    pub fn start(backend: Arc<dyn MediaBackend>, path: PathBuf, ctx: egui::Context) -> ProbeJob {
        let (tx, rx) = mpsc::channel();
        let worker_tx = tx.clone();
        let worker_path = path.clone();
        let spawned = thread::Builder::new()
            .name("microcut-probe".to_owned())
            .spawn(move || {
                let result =
                    match panic::catch_unwind(AssertUnwindSafe(|| backend.probe(&worker_path))) {
                        Ok(probed) => probed.map_err(|err| err.to_string()),
                        Err(_) => Err("internal error while reading the file".to_owned()),
                    };
                // The receiver is gone if the user opened another file meanwhile.
                let _ = worker_tx.send(result);
                ctx.request_repaint();
            });
        if let Err(err) = spawned {
            let _ = tx.send(Err(format!("could not start reading the file: {err}")));
        }
        ProbeJob { path, rx }
    }

    /// The file being probed.
    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    /// The result once the worker is done.
    pub fn poll(&self) -> Option<ProbeResult> {
        match self.rx.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                Some(Err("reading the file stopped unexpectedly".to_owned()))
            }
        }
    }
}
