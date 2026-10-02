use std::path::Path;
use std::time::Instant;

use illuminator_core::cache::VolumeCache;
use illuminator_core::progress::Progress;
use illuminator_core::volume::NiiVolume;

fn main() -> illuminator_core::Result<()> {
    let cache = VolumeCache::from_env()?;
    for arg in std::env::args().skip(1) {
        let started = Instant::now();
        let volume = NiiVolume::load(Path::new(&arg), &cache, &Progress::default())?;
        let loaded = started.elapsed();
        let range = volume.frame_range(0, &Progress::default())?;
        let measured = started.elapsed() - loaded;

        let h = volume.header();
        // The C++ viewer starts with the cursor on this voxel.
        let center = [
            (h.width() - 1) / 2,
            (h.height() - 1) / 2,
            (h.depth() - 1) / 2,
        ];
        println!("{arg}");
        println!(
            "  {} x {} x {} x {} | {}",
            h.width(),
            h.height(),
            h.depth(),
            h.timepoints(),
            h.datatype.name()
        );
        println!("  loaded in {loaded:.2?}, range measured in {measured:.2?}");
        println!("  frame 0 range: {range:?}");
        println!("  voxel {center:?} = {:?}", volume.sample_value(center, 0));
    }
    Ok(())
}
