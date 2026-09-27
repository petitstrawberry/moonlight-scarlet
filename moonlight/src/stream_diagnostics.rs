//! Low-frequency sampling of the video worker, including blocked operations.

use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug)]
pub enum Stage {
    Receive,
    Copy,
    Submit,
    Dequeue,
    Present,
    Complete,
}

struct Progress {
    stage: Stage,
    since: Instant,
    frames: u64,
    longest: [Duration; 6],
}

pub struct Monitor {
    progress: Arc<Mutex<Progress>>,
    stop: mpsc::Sender<()>,
    worker: Option<JoinHandle<()>>,
}

impl Monitor {
    pub fn start() -> Result<Self, String> {
        let progress = Arc::new(Mutex::new(Progress {
            stage: Stage::Receive,
            since: Instant::now(),
            frames: 0,
            longest: [Duration::ZERO; 6],
        }));
        let sampled = Arc::clone(&progress);
        let (stop, receiver) = mpsc::channel();
        let worker = thread::Builder::new()
            .name(String::from("video-progress"))
            .spawn(move || {
                let mut last_report = Instant::now();
                let mut last_stall = None;
                while matches!(
                    receiver.recv_timeout(Duration::from_millis(500)),
                    Err(mpsc::RecvTimeoutError::Timeout)
                ) {
                    let (stage, since, frames, longest) = {
                        let mut current = sampled.lock().unwrap_or_else(|e| e.into_inner());
                        let snapshot = (current.stage, current.since, current.frames, current.longest);
                        if last_report.elapsed() >= Duration::from_secs(10) {
                            current.longest = [Duration::ZERO; 6];
                        }
                        snapshot
                    };
                    // Log outside the lock: a slow console must not hold up video.
                    if since.elapsed() >= Duration::from_millis(500) && last_stall != Some(since) {
                        eprintln!("moonlight-video: stalled stage={stage:?} elapsed_ms={} frames={frames}", since.elapsed().as_millis());
                        last_stall = Some(since);
                    }
                    if last_report.elapsed() >= Duration::from_secs(10) {
                        eprintln!("moonlight-video: frames={frames} max_ms receive={} copy={} submit={} dequeue={} present={} complete={}",
                            longest[0].as_millis(), longest[1].as_millis(), longest[2].as_millis(),
                            longest[3].as_millis(), longest[4].as_millis(), longest[5].as_millis());
                        last_report = Instant::now();
                    }
                }
            })
            .map_err(|error| format!("failed to start video diagnostics: {error}"))?;
        Ok(Self {
            progress,
            stop,
            worker: Some(worker),
        })
    }

    pub fn enter(&self, stage: Stage) {
        let mut progress = self.progress.lock().unwrap_or_else(|e| e.into_inner());
        let index = progress.stage as usize;
        progress.longest[index] = progress.longest[index].max(progress.since.elapsed());
        if matches!(progress.stage, Stage::Complete) {
            progress.frames += 1;
        }
        progress.stage = stage;
        progress.since = Instant::now();
    }
}

impl Drop for Monitor {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
