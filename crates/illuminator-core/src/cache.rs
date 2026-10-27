//! A cache of decompressed `.gz` volumes, so they can be memory-mapped like other files.
//!
//! When it would grow past its capacity, it first removes the oldest files that no volume uses.

use flate2::read::MultiGzDecoder;
use std::collections::{HashMap, HashSet};
use std::env::VarError;
use std::fs::{self, File};
use std::io::{BufReader, Read, Seek, SeekFrom, Write};
#[cfg(unix)]
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::error::{Error, Result};
use crate::progress::Progress;

const BYTES_PER_GIBIBYTE: f64 = 1024.0 * 1024.0 * 1024.0;

#[derive(Debug)]
pub(crate) struct CacheLease {
    path: PathBuf,
}

impl CacheLease {
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

/// Cache files in use by path
type Leases = HashMap<PathBuf, Weak<CacheLease>>;

#[derive(Default)]
struct State {
    leases: Leases,
    reserved_bytes: u64,
    paths_being_written: HashSet<PathBuf>,
}

pub struct VolumeCache {
    directory: PathBuf,
    capacity: u64,
    state: Mutex<State>,
    write_finished: Condvar,
}

impl VolumeCache {
    /// A cache in `directory` holding at most `capacity_bytes`.
    /// [`Self::from_environment`] picks both from the environment.
    pub fn new(directory: impl Into<PathBuf>, capacity_bytes: u64) -> Result<Self> {
        if capacity_bytes == 0 {
            return Err(Error::invalid("Cache capacity must be more than zero"));
        }
        let directory = directory.into();
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        builder.mode(0o700); // not possible or needed on windows.
        builder.create(&directory)?;

        Ok(Self {
            directory,
            capacity: capacity_bytes,
            state: Mutex::new(State::default()),
            write_finished: Condvar::new(),
        })
    }

    pub fn from_environment() -> Result<Self> {
        let directory = match std::env::var_os("ILLUMINATOR_CACHE_DIR") {
            Some(directory) => PathBuf::from(directory),
            None => std::env::temp_dir().join("illuminator-cache-v1"),
        };
        let gib = match std::env::var("ILLUMINATOR_CACHE_GIB") {
            Ok(text) => text
                .parse::<f64>()
                .map_err(|_| Error::invalid("Invalid ILLUMINATOR_CACHE_GIB"))?,
            Err(VarError::NotPresent) => 64.0,
            Err(VarError::NotUnicode(_)) => {
                return Err(Error::invalid("Invalid ILLUMINATOR_CACHE_GIB"));
            }
        };
        if !gib.is_finite() || gib <= 0.0 || gib > 1_048_576.0 {
            return Err(Error::invalid(
                "ILLUMINATOR_CACHE_GIB must be positive and at most 1048576",
            ));
        }
        Self::new(directory, (gib * BYTES_PER_GIBIBYTE) as u64)
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    pub fn capacity(&self) -> u64 {
        self.capacity
    }

    #[cfg(test)]
    fn acquire(
        &self,
        source: &Path,
        minimum_bytes: u64,
        progress: &Progress,
    ) -> Result<Arc<CacheLease>> {
        self.acquire_from(source, 0, minimum_bytes, progress)
    }

    pub(crate) fn acquire_from(
        &self,
        source: &Path,
        start: u64,
        minimum_bytes: u64,
        progress: &Progress,
    ) -> Result<Arc<CacheLease>> {
        progress.check_cancelled()?;
        let source = fs::canonicalize(source)?;
        let identity = file_identity(&source)?;
        let mut key = cache_key(&source, identity);
        if start > 0 {
            key.push_str(&format!("\nfrom {start}"));
        }
        let target = self
            .directory
            .join(format!("{}.nii", cache_file_stem(&key)));
        let metadata_path = target.with_extension("nii.meta");

        let mut pending_write = {
            let mut state = self.lock();
            // Another load is decompressing this file
            while state.paths_being_written.contains(&target) {
                progress.report(0.0, "Waiting for another load of this file")?;
                state = self
                    .write_finished
                    .wait_timeout(state, Duration::from_millis(100))
                    .expect("cache lock poisoned")
                    .0;
            }

            if let Some(bytes) = cached_size(&target, &metadata_path, &key, minimum_bytes) {
                if bytes > self.capacity {
                    return Err(Error::invalid("Cached volume exceeds cache capacity"));
                }
                File::options()
                    .write(true)
                    .open(&target)?
                    .set_modified(SystemTime::now())?;
                let lease = obtain_lease(&mut state.leases, &target);
                self.reclaim_space(&mut state, 0)?;
                progress.report(1.0, "Using cached volume")?;
                return Ok(lease);
            }

            self.reclaim_space(&mut state, minimum_bytes)?;
            state.reserved_bytes += minimum_bytes;
            state.paths_being_written.insert(target.clone());
            PendingWrite {
                cache: self,
                target: target.clone(),
                reserved: minimum_bytes,
            }
        };
        // The lock is released here

        let mut output = tempfile::Builder::new()
            .prefix("work-")
            .tempfile_in(&self.directory)?;
        let total = self.decompress_into(
            &source,
            start,
            &mut output,
            &mut pending_write,
            minimum_bytes,
            progress,
        )?;
        if total < minimum_bytes {
            return Err(Error::invalid("Decompressed data is truncated"));
        }
        if file_identity(&source)? != identity {
            return Err(Error::invalid(
                "Source file changed while loading. Reopen the volume",
            ));
        }
        progress.check_cancelled()?;

        let mut metadata = tempfile::Builder::new()
            .prefix("work-")
            .tempfile_in(&self.directory)?;
        metadata.write_all(format!("{key}\n{total}").as_bytes())?;
        // Lease before the file appears so another thread's reclaim_space can't evict it
        let lease = obtain_lease(&mut self.lock().leases, &target);
        output.persist(&target).map_err(|error| error.error)?;
        if let Err(error) = metadata.persist(&metadata_path) {
            let _ = fs::remove_file(&target);
            return Err(error.error.into());
        }
        progress.report(1.0, "Decompression complete")?;
        Ok(lease)
        // `writing` is dropped here
    }

    fn decompress_into(
        &self,
        source: &Path,
        start: u64,
        output: &mut impl Write,
        writing: &mut PendingWrite<'_>,
        minimum_bytes: u64,
        progress: &Progress,
    ) -> Result<u64> {
        let mut file = File::open(source)?;
        file.seek(SeekFrom::Start(start))?;
        let mut input = MultiGzDecoder::new(BufReader::new(file));
        let mut buffer = vec![0u8; 1024 * 1024];
        let mut total = 0u64;
        loop {
            progress.check_cancelled()?;
            let read = input
                .read(&mut buffer)
                .map_err(|error| Error::invalid(format!("Gzip decompression failed: {error}")))?;
            if read == 0 {
                return Ok(total);
            }
            let bytes = read as u64;
            if bytes > self.capacity - total {
                return Err(Error::invalid("Decompressed volume exceeds cache capacity"));
            }
            writing.extend_reservation(total + bytes)?;
            output.write_all(&buffer[..read]).map_err(|error| {
                Error::invalid(format!(
                    "Cannot write decompressed volume; disk may be full ({error})"
                ))
            })?;
            total += bytes;
            let fraction = total as f64 / minimum_bytes.max(1) as f64;
            progress.report(fraction.min(0.99), "Decompressing")?;
        }
    }

    fn reclaim_space(&self, state: &mut State, required_bytes: u64) -> Result<()> {
        if required_bytes > self.capacity {
            return Err(Error::invalid("Volume exceeds cache capacity"));
        }
        let limit = self
            .capacity
            .checked_sub(state.reserved_bytes + required_bytes)
            .ok_or_else(|| Error::invalid("Cache is full with loads in progress"))?;
        let mut entries = Vec::new();
        let mut total = 0u64;
        for entry in fs::read_dir(&self.directory)? {
            let entry = entry?;
            let path = entry.path();
            let file_metadata = entry.metadata()?;
            if !file_metadata.is_file() || !is_cache_file(&path) {
                continue;
            }
            total = total
                .checked_add(file_metadata.len())
                .ok_or_else(|| Error::invalid("Cache size overflow"))?;
            entries.push((file_metadata.modified()?, file_metadata.len(), path));
        }
        entries.sort();

        for (_, bytes, path) in entries {
            if total <= limit {
                break;
            }
            let in_use = state
                .leases
                .get(&path)
                .is_some_and(|lease| lease.strong_count() > 0);
            if in_use {
                continue;
            }
            if fs::remove_file(&path).is_err() {
                // Windows will refuse to delete a file if anoter program has it open, so skip it if it can't delete
                continue;
            }
            let _ = fs::remove_file(path.with_extension("nii.meta"));
            state.leases.remove(&path);
            total -= bytes;
        }
        if total > limit {
            return Err(Error::invalid(
                "Cache is full with active volumes. Increase cache size",
            ));
        }
        Ok(())
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().expect("cache lock poisoned")
    }
}

struct PendingWrite<'a> {
    cache: &'a VolumeCache,
    target: PathBuf,
    reserved: u64,
}

impl PendingWrite<'_> {
    fn extend_reservation(&mut self, total: u64) -> Result<()> {
        if total <= self.reserved {
            return Ok(());
        }
        let additional_bytes = total - self.reserved;
        let mut state = self.cache.lock();
        self.cache.reclaim_space(&mut state, additional_bytes)?;
        state.reserved_bytes += additional_bytes;
        self.reserved = total;
        Ok(())
    }
}

impl Drop for PendingWrite<'_> {
    fn drop(&mut self) {
        let mut state = self
            .cache
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        state.reserved_bytes -= self.reserved;
        state.paths_being_written.remove(&self.target);
        drop(state);
        self.cache.write_finished.notify_all();
    }
}

fn obtain_lease(leases: &mut Leases, path: &Path) -> Arc<CacheLease> {
    if let Some(lease) = leases.get(path).and_then(Weak::upgrade) {
        return lease;
    }
    let lease = Arc::new(CacheLease {
        path: path.to_path_buf(),
    });
    leases.insert(path.to_path_buf(), Arc::downgrade(&lease));
    lease
}

fn cached_size(target: &Path, metadata_path: &Path, key: &str, minimum_bytes: u64) -> Option<u64> {
    let bytes = fs::metadata(target)
        .ok()
        .filter(|file_metadata| file_metadata.is_file())?
        .len();
    let recorded = fs::read_to_string(metadata_path).ok()?;
    (bytes >= minimum_bytes && recorded == format!("{key}\n{bytes}")).then_some(bytes)
}

fn file_identity(path: &Path) -> Result<(u64, SystemTime)> {
    let file_metadata = fs::metadata(path)?;
    Ok((file_metadata.len(), file_metadata.modified()?))
}

fn cache_key(source: &Path, (size, modified): (u64, SystemTime)) -> String {
    let since_epoch = modified.duration_since(UNIX_EPOCH).unwrap_or_default();
    format!(
        "illuminator-cache-v1\n{}\n{size}\n{}:{}",
        source.display(),
        since_epoch.as_secs(),
        since_epoch.subsec_nanos()
    )
}

fn cache_file_stem(key: &str) -> String {
    let mut a: u64 = 14695981039346656037;
    let mut b: u64 = 7809847782465536322;
    for byte in key.bytes() {
        a = (a ^ u64::from(byte)).wrapping_mul(1099511628211);
        b = (b ^ u64::from(byte)).wrapping_mul(14029467366897019727);
    }
    format!("volume-{a:016x}{b:016x}")
}

fn is_cache_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    name.len() == 43
        && name.starts_with("volume-")
        && name.ends_with(".nii")
        && name[7..39]
            .bytes()
            .all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f'))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    use flate2::Compression;
    use flate2::write::GzEncoder;

    use super::*;

    fn write_gz(path: &Path, bytes: &[u8]) {
        let mut encoder = GzEncoder::new(File::create(path).unwrap(), Compression::default());
        encoder.write_all(bytes).unwrap();
        encoder.finish().unwrap();
    }

    /// `len` bytes counting up from `start`.
    fn data(len: usize, start: u8) -> Vec<u8> {
        (0..len).map(|i| start.wrapping_add(i as u8)).collect()
    }

    fn count_files(directory: &Path, matches: impl Fn(&Path) -> bool) -> usize {
        fs::read_dir(directory)
            .unwrap()
            .filter(|entry| matches(&entry.as_ref().unwrap().path()))
            .count()
    }

    fn cache_entries(directory: &Path) -> usize {
        count_files(directory, is_cache_file)
    }

    fn work_files(directory: &Path) -> usize {
        count_files(directory, |path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("work-")
        })
    }

    fn recorder() -> (Progress, Arc<Mutex<Vec<String>>>) {
        let stages = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&stages);
        let progress = Progress::new(Arc::new(AtomicBool::new(false)), move |_, stage| {
            seen.lock().unwrap().push(stage.to_string());
        });
        (progress, stages)
    }

    #[test]
    fn names_are_recognized() {
        let name = cache_file_stem("some key");
        assert_eq!(name.len(), 39);
        assert!(is_cache_file(Path::new(&format!("{name}.nii"))));
        assert!(!is_cache_file(Path::new(&format!("{name}.nii.meta"))));
        assert!(!is_cache_file(Path::new("work-abc123")));
        assert_ne!(cache_file_stem("some key"), cache_file_stem("other key"));
    }

    #[test]
    fn decompresses_once_and_reuses() {
        let directory = tempfile::tempdir().unwrap();
        let cache = VolumeCache::new(directory.path().join("cache"), 1 << 20).unwrap();
        let source = directory.path().join("a.nii.gz");
        let bytes = data(5000, 0);
        write_gz(&source, &bytes);

        let lease = cache.acquire(&source, 5000, &Progress::default()).unwrap();
        assert_eq!(fs::read(lease.path()).unwrap(), bytes);

        let (progress, stages) = recorder();
        let again = cache.acquire(&source, 5000, &progress).unwrap();
        assert_eq!(again.path(), lease.path());
        assert!(
            stages
                .lock()
                .unwrap()
                .iter()
                .any(|s| s == "Using cached volume")
        );
        assert_eq!(cache_entries(cache.directory()), 1);
    }

    #[test]
    fn decompresses_from_partway_into_a_file() {
        let directory = tempfile::tempdir().unwrap();
        let cache = VolumeCache::new(directory.path().join("cache"), 1 << 20).unwrap();
        let source = directory.path().join("a.nrrd");
        let bytes = data(5000, 0);
        write_gz(&source, &bytes);
        let gzip = fs::read(&source).unwrap();
        fs::write(&source, [b"NRRD0004\n\n".as_slice(), &gzip].concat()).unwrap();

        let none = Progress::default();
        let lease = cache.acquire_from(&source, 10, 5000, &none).unwrap();
        assert_eq!(fs::read(lease.path()).unwrap(), bytes);
        assert!(cache.acquire(&source, 5000, &none).is_err());
    }

    #[test]
    fn changed_source_gets_a_new_entry() {
        let directory = tempfile::tempdir().unwrap();
        let cache = VolumeCache::new(directory.path().join("cache"), 1 << 20).unwrap();
        let source = directory.path().join("a.nii.gz");
        write_gz(&source, &data(5000, 0));
        let old = cache.acquire(&source, 5000, &Progress::default()).unwrap();

        write_gz(&source, &data(5000, 100));
        File::options()
            .write(true)
            .open(&source)
            .unwrap()
            .set_modified(SystemTime::now() + Duration::from_secs(1))
            .unwrap();
        let new = cache.acquire(&source, 5000, &Progress::default()).unwrap();

        assert_ne!(old.path(), new.path());
        assert_eq!(fs::read(new.path()).unwrap()[0], 100);
        // The old file is still leased, so it is untouched.
        assert_eq!(fs::read(old.path()).unwrap()[0], 0);
    }

    #[test]
    fn bad_gzip_leaves_nothing_behind() {
        let directory = tempfile::tempdir().unwrap();
        let cache = VolumeCache::new(directory.path().join("cache"), 1 << 20).unwrap();
        let good = directory.path().join("good.nii.gz");
        write_gz(&good, &data(5000, 0));
        let gzip = fs::read(&good).unwrap();

        let mut corrupt = gzip.clone();
        let checksum = corrupt.len() - 8;
        corrupt[checksum] ^= 0xFF;
        let truncated = gzip[..gzip.len() - 8].to_vec();

        for (name, bytes) in [("corrupt.nii.gz", corrupt), ("short.nii.gz", truncated)] {
            let path = directory.path().join(name);
            fs::write(&path, bytes).unwrap();
            assert!(
                cache.acquire(&path, 5000, &Progress::default()).is_err(),
                "{name}"
            );
        }
        assert_eq!(work_files(cache.directory()), 0);
        assert_eq!(cache_entries(cache.directory()), 0);
    }

    #[test]
    fn cancellation_stops_and_cleans_up() {
        let directory = tempfile::tempdir().unwrap();
        let cache = VolumeCache::new(directory.path().join("cache"), 1 << 20).unwrap();
        let source = directory.path().join("a.nii.gz");
        write_gz(&source, &data(5000, 0));

        let cancel = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&cancel);
        let progress = Progress::new(cancel, move |_, _| flag.store(true, Ordering::Relaxed));
        let result = cache.acquire(&source, 5000, &progress);

        assert!(matches!(result, Err(Error::Cancelled)));
        assert_eq!(work_files(cache.directory()), 0);
        assert_eq!(cache_entries(cache.directory()), 0);
    }

    #[test]
    fn evicts_oldest_unused_entries() {
        let directory = tempfile::tempdir().unwrap();
        let cache = VolumeCache::new(directory.path().join("tiny"), 900).unwrap();
        let [a, b, c] = ["a", "b", "c"].map(|name| {
            let path = directory.path().join(format!("{name}.nii.gz"));
            write_gz(&path, &data(400, 0));
            path
        });
        let none = Progress::default();

        let lease_a = cache.acquire(&a, 400, &none).unwrap();
        let lease_b = cache.acquire(&b, 400, &none).unwrap();
        assert!(cache.acquire(&c, 400, &none).is_err());

        drop(lease_a);
        let lease_c = cache.acquire(&c, 400, &none).unwrap();
        assert_eq!(cache_entries(cache.directory()), 2);
        assert!(lease_b.path().exists() && lease_c.path().exists());

        let reduced = VolumeCache::new(cache.directory(), 300).unwrap();
        assert!(reduced.acquire(&b, 400, &none).is_err());
    }

    #[test]
    fn rejects_volumes_larger_than_the_capacity() {
        let directory = tempfile::tempdir().unwrap();
        let cache = VolumeCache::new(directory.path().join("cache"), 100).unwrap();
        let source = directory.path().join("a.nii.gz");
        write_gz(&source, &data(5000, 0));
        assert!(cache.acquire(&source, 5000, &Progress::default()).is_err());
        assert!(VolumeCache::new(directory.path().join("zero"), 0).is_err());
    }

    #[test]
    fn parallel_loads_of_one_file_share_the_work() {
        let directory = tempfile::tempdir().unwrap();
        let cache = VolumeCache::new(directory.path().join("cache"), 1 << 24).unwrap();
        let source = directory.path().join("a.nii.gz");
        let bytes = data(1 << 20, 0);
        write_gz(&source, &bytes);

        let leases: Vec<_> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..4)
                .map(|_| {
                    scope.spawn(|| {
                        cache
                            .acquire(&source, 1 << 20, &Progress::default())
                            .unwrap()
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });

        assert!(leases.iter().all(|lease| lease.path() == leases[0].path()));
        assert_eq!(fs::read(leases[0].path()).unwrap(), bytes);
        assert_eq!(cache_entries(cache.directory()), 1);
        assert_eq!(work_files(cache.directory()), 0);
    }

    #[test]
    fn failed_loads_release_their_reservation() {
        let directory = tempfile::tempdir().unwrap();
        let cache = VolumeCache::new(directory.path().join("cache"), 6000).unwrap();
        let bad = directory.path().join("bad.nii.gz");
        fs::write(&bad, b"not gzip").unwrap();
        assert!(cache.acquire(&bad, 5000, &Progress::default()).is_err());

        let good = directory.path().join("good.nii.gz");
        write_gz(&good, &data(5000, 0));
        assert!(cache.acquire(&good, 5000, &Progress::default()).is_ok());
    }
}
