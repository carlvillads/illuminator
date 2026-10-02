use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use illuminator_core::cache::VolumeCache;
use illuminator_core::header::Header;
use illuminator_core::progress::Progress;

fn main() -> illuminator_core::Result<()> {
    let cache = VolumeCache::from_env()?;
    println!(
        "Cache: {} (budget {:.1} GiB)",
        cache.directory().display(),
        cache.budget() as f64 / (1u64 << 30) as f64
    );
    for arg in std::env::args().skip(1) {
        let header = Header::read(Path::new(&arg))?;
        if !header.is_gzipped() {
            println!("{arg}: not compressed, nothing to cache");
            continue;
        }
        let needed = header.data_offset + header.data_bytes as u64;
        let progress = Progress::new(Arc::new(AtomicBool::new(false)), |fraction, stage| {
            eprint!("\r{stage}: {:5.1}%   ", fraction * 100.0);
        });
        let started = Instant::now();
        let lease = cache.acquire(&header.data_path, needed, &progress)?;
        eprintln!();
        println!(
            "{arg}\n  -> {} ({:.2?})",
            lease.path().display(),
            started.elapsed()
        );
    }
    Ok(())
}
