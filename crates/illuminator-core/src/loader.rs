//! loader.rs
//!
//! Loads volumes on background threads and tracks the progress of each load.
//!

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;

use crate::cache::VolumeCache;
use crate::error::{Error, Result};
use crate::progress::Progress;
use crate::volume::{Range, Volume};

const MAX_WORKERS: usize = 4;

pub struct LoadedVolume {
    pub load_id: u64,
    pub volume: Volume,
    pub range: Option<Range>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LoadState {
    Queued,
    Loading,
    Ready,
    Failed(String),
    Cancelled,
}

impl LoadState {
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Queued | Self::Loading)
    }
}

#[derive(Debug, Clone)]
pub struct LoadStatus {
    pub load_id: u64,
    pub path: PathBuf,
    pub state: LoadState,
    pub stage: String,
    pub progress: f64,
}

struct Job {
    status: LoadStatus,
    cancel: Arc<AtomicBool>,
}

#[derive(Default)]
struct Shared {
    queue: VecDeque<u64>,
    jobs: Vec<Job>,
    completed: Vec<LoadedVolume>,
    stop: bool,
    next_id: u64,
}

impl Shared {
    fn job_mut(&mut self, id: u64) -> Option<&mut Job> {
        self.jobs.iter_mut().find(|job| job.status.load_id == id)
    }
}

struct Inner {
    shared: Mutex<Shared>,
    wake: Condvar,
    cache: Arc<VolumeCache>,
    on_change: Box<dyn Fn() + Send + Sync>,
}

impl Inner {
    fn lock(&self) -> MutexGuard<'_, Shared> {
        self.shared.lock().expect("loader lock poisoned")
    }

    fn update(&self, id: u64, change: impl FnOnce(&mut LoadStatus)) {
        if let Some(job) = self.lock().job_mut(id) {
            change(&mut job.status);
        }
        (self.on_change)();
    }
}

pub struct VolumeLoader {
    inner: Arc<Inner>,
    workers: Vec<JoinHandle<()>>,
}

impl VolumeLoader {
    pub fn new(cache: Arc<VolumeCache>, on_change: impl Fn() + Send + Sync + 'static) -> Self {
        let inner = Arc::new(Inner {
            shared: Mutex::new(Shared {
                next_id: 1,
                ..Shared::default()
            }),
            wake: Condvar::new(),
            cache,
            on_change: Box::new(on_change),
        });
        let count = std::thread::available_parallelism()
            .map_or(2, |n| n.get())
            .clamp(1, MAX_WORKERS);
        let workers = (0..count)
            .map(|n| {
                let inner = Arc::clone(&inner);
                std::thread::Builder::new()
                    .name(format!("volume-loader-{n}"))
                    .spawn(move || run(&inner))
                    .expect("cannot start a volume loader thread")
            })
            .collect();
        Self { inner, workers }
    }

    /// Queues file for loading and returns its id
    pub fn enqueue(&self, path: impl Into<PathBuf>) -> u64 {
        let id = {
            let mut shared = self.inner.lock();
            let id = shared.next_id;
            shared.next_id += 1;
            shared.jobs.push(Job {
                status: LoadStatus {
                    load_id: id,
                    path: path.into(),
                    state: LoadState::Queued,
                    stage: "Queued".into(),
                    progress: 0.0,
                },
                cancel: Arc::new(AtomicBool::new(false)),
            });
            shared.queue.push_back(id);
            id
        };
        self.inner.wake.notify_one();
        (self.inner.on_change)();
        id
    }

    pub fn cancel(&self, id: u64) {
        if let Some(job) = self.inner.lock().job_mut(id) {
            job.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// Status of every non-dismised load, sorted by oldest first
    pub fn statuses(&self) -> Vec<LoadStatus> {
        let shared = self.inner.lock();
        shared.jobs.iter().map(|job| job.status.clone()).collect()
    }

    pub fn take_completed(&self) -> Vec<LoadedVolume> {
        std::mem::take(&mut self.inner.lock().completed)
    }

    pub fn dismiss_finished(&self) {
        self.inner
            .lock()
            .jobs
            .retain(|job| job.status.state.is_active());
    }
}

impl Drop for VolumeLoader {
    /// Cancel all work and wait for the worker thread to finish
    fn drop(&mut self) {
        {
            let mut shared = self.inner.lock();
            shared.stop = true;
            for job in &shared.jobs {
                job.cancel.store(true, Ordering::Relaxed);
            }
        }
        self.inner.wake.notify_all();
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

/// Worker thread
fn run(inner: &Arc<Inner>) {
    loop {
        let (id, path, cancel) = {
            let shared = inner.lock();
            let mut shared = inner
                .wake
                .wait_while(shared, |shared| !shared.stop && shared.queue.is_empty())
                .expect("loader lock poisoned");
            if shared.stop {
                return;
            }
            let id = shared.queue.pop_front().expect("queue is not empty");
            let Some(job) = shared.job_mut(id) else {
                continue;
            };
            job.status.state = LoadState::Loading;
            job.status.stage = "Reading header".into();
            (id, job.status.path.clone(), Arc::clone(&job.cancel))
        };
        (inner.on_change)();

        let reporter = Arc::clone(inner);
        let progress = Progress::new(Arc::clone(&cancel), move |fraction, stage| {
            reporter.update(id, |status| {
                status.progress = fraction;
                status.stage = stage.to_string();
            });
        });
        let result = load(&path, &inner.cache, &progress);

        {
            let mut shared = inner.lock();
            // a cancel that arrives after the work is done will still win. the user has asked to cancel this
            // volume opening, so it must not show up anyway after they've cancelled.
            let cancelled = cancel.load(Ordering::Relaxed);
            let (state, stage) = match result {
                Ok(_) if cancelled => (LoadState::Cancelled, "Cancelled"),
                Ok((volume, range)) => {
                    shared.completed.push(LoadedVolume {
                        load_id: id,
                        volume,
                        range,
                    });
                    (LoadState::Ready, "Ready")
                }
                Err(Error::Cancelled) => (LoadState::Cancelled, "Cancelled"),
                Err(error) => (LoadState::Failed(error.to_string()), "Failed"),
            };
            if let Some(job) = shared.job_mut(id) {
                if state == LoadState::Ready {
                    job.status.progress = 1.0;
                }
                job.status.state = state;
                job.status.stage = stage.into();
            }
        }
        (inner.on_change)();
    }
}

fn load(path: &Path, cache: &VolumeCache, progress: &Progress) -> Result<(Volume, Option<Range>)> {
    let volume = Volume::load(path, cache, &progress.part(0.0, 0.8))?;
    let range = volume.frame_range(0, &progress.part(0.8, 0.2))?;
    Ok((volume, range))
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use super::*;
    use crate::test_util::Fixture;

    fn wait_until_idle(loader: &VolumeLoader, changed: &mpsc::Receiver<()>) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while loader
            .statuses()
            .iter()
            .any(|status| status.state.is_active())
        {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                changed.recv_timeout(remaining).is_ok(),
                "loader did not finish"
            );
        }
    }

    #[test]
    fn isolates_failures_and_cancellation() {
        let dir = tempfile::tempdir().unwrap();
        let good = dir.path().join("worker.nii.gz");
        Fixture::default().write(&good);
        let cache = Arc::new(VolumeCache::new(dir.path().join("cache"), 1 << 24).unwrap());
        let (sender, changed) = mpsc::channel();
        let loader = VolumeLoader::new(cache, move || {
            let _ = sender.send(());
        });

        loader.enqueue(dir.path().join("does-not-exist.nii"));
        let ok = loader.enqueue(&good);
        let cancelled = loader.enqueue(&good);
        loader.cancel(cancelled);
        wait_until_idle(&loader, &changed);

        let completed = loader.take_completed();
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].load_id, ok);
        assert_eq!(completed[0].volume.sample_value([3, 2, 3], 0), Some(47.0));
        assert_eq!(
            completed[0].range,
            Some(Range {
                min: 0.0,
                max: 47.0
            })
        );
        assert!(loader.take_completed().is_empty());

        let statuses = loader.statuses();
        assert!(matches!(statuses[0].state, LoadState::Failed(_)));
        assert_eq!(statuses[1].state, LoadState::Ready);
        assert_eq!(statuses[1].progress, 1.0);
        assert_eq!(statuses[2].state, LoadState::Cancelled);

        loader.dismiss_finished();
        assert!(loader.statuses().is_empty());
    }

    #[test]
    fn dropping_stops_the_worker() {
        let dir = tempfile::tempdir().unwrap();
        let good = dir.path().join("a.nii.gz");
        Fixture::default().write(&good);
        let cache = Arc::new(VolumeCache::new(dir.path().join("cache"), 1 << 24).unwrap());
        let loader = VolumeLoader::new(cache, || {});
        for _ in 0..20 {
            loader.enqueue(&good);
        }
        drop(loader);
    }
}
