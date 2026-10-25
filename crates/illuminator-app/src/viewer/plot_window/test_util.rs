//! A small volume loaded for tests.

use illuminator_core::cache::VolumeCache;
use illuminator_core::display::VolumeDisplay;
use illuminator_core::loader::VolumeLoader;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Removed when dropped
pub(in crate::viewer) struct TestDirectory(PathBuf);

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A 2x1x1 volume with 2 frames
pub(in crate::viewer) fn loaded_display() -> (TestDirectory, VolumeDisplay) {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let directory = TestDirectory(std::env::temp_dir().join(format!(
        "illuminator-plot-test-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )));
    std::fs::create_dir(&directory.0).unwrap();
    let path = directory.0.join("values.nrrd");
    let mut data = b"NRRD0004\ntype: uchar\ndimension: 4\nsizes: 2 1 1 2\nencoding: raw\nspace: RAS\nspace directions: (1,0,0) (0,1,0) (0,0,1) none\n\n".to_vec();
    data.extend_from_slice(&[1, 2, 3, 4]);
    std::fs::write(&path, data).unwrap();
    let (sender, receiver) = mpsc::channel();
    let cache = VolumeCache::new(directory.0.join("cache"), 1 << 20).unwrap();
    let loader = VolumeLoader::new(Arc::new(cache), move || {
        let _ = sender.send(());
    });
    loader.enqueue(path);
    let deadline = Instant::now() + Duration::from_secs(10);
    let loaded = loop {
        if let Some(loaded) = loader.take_completed().pop() {
            break loaded;
        }
        receiver
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .expect("test volume did not load");
    };
    (directory, VolumeDisplay::new(loaded).unwrap())
}
