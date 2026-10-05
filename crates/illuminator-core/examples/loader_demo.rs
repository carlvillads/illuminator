use std::sync::Arc;
use std::time::{Duration, Instant};

use illuminator_core::cache::VolumeCache;
use illuminator_core::labels::LabelTable;
use illuminator_core::loader::{LoadState, VolumeLoader};
use illuminator_core::progress::Progress;

fn main() -> illuminator_core::Result<()> {
    let mut count_labels = false;
    let cache = Arc::new(VolumeCache::from_env()?);
    let loader = VolumeLoader::new(cache, || {});
    for arg in std::env::args().skip(1) {
        if arg == "--labels" {
            count_labels = true;
        } else {
            loader.enqueue(arg);
        }
    }

    loop {
        let statuses = loader.statuses();
        let line: Vec<String> = statuses
            .iter()
            .map(|s| format!("#{} {:3.0}% {}", s.id, s.progress * 100.0, s.stage))
            .collect();
        eprint!("\r{}   ", line.join(" | "));

        for loaded in loader.take_completed() {
            eprintln!();
            println!(
                "#{} ready: {} (range {:?})",
                loaded.id,
                loaded.volume.path().display(),
                loaded.range
            );
            if count_labels {
                let started = Instant::now();
                let mut table = LabelTable::default();
                match table.encode(&loaded.volume, 0, 1 << 20, &Progress::default()) {
                    Ok(_) => println!(
                        "  {} labels including background ({:.2?})",
                        table.labels().len(),
                        started.elapsed()
                    ),
                    Err(error) => println!("  not a segmentation: {error}"),
                }
            }
        }
        if !statuses.iter().any(|s| s.state.is_active()) {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    eprintln!();
    for status in loader.statuses() {
        if let LoadState::Failed(error) = &status.state {
            println!("#{} failed: {error}", status.id);
        }
    }
    Ok(())
}
