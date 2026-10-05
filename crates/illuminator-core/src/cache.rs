use flate2::read::MultiGzDecoder;
use std::collections::HashMap;
use std::env::VarError;
use std::fs::{self, File};
use std::io::{BufReader, Read, Write};
#[cfg(unix)]
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::{Error, Result};
use crate::progress::Progress;

const GIB: f64 = 1024.0 * 1024.0 * 1024.0;

#[derive(Debug)]
pub struct CacheLease {
    path: PathBuf,
}

impl CacheLease {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Cache files in use by path
type Leases = HashMap<PathBuf, Weak<CacheLease>>;

pub struct VolumeCache {
    directory: PathBuf,
    budget: u64,
    leases: Mutex<Leases>,
}

impl VolumeCache {
    pub fn new(directory: impl Into<PathBuf>, budget_bytes: u64) -> Result<Self> {
        if budget_bytes == 0 {
            return Err(Error::invalid("Cache budget must be more than zero"));
        }
        let directory = directory.into();
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        builder.mode(0o700); // not possible or needed on windows.
        builder.create(&directory)?;

        Ok(Self {
            directory,
            budget: budget_bytes,
            leases: Mutex::new(HashMap::new()),
        })
    }

    pub fn from_env() -> Result<Self> {
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
        Self::new(directory, (gib * GIB) as u64)
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    pub fn budget(&self) -> u64 {
        self.budget
    }

    pub fn acquire(
        &self,
        source: &Path,
        min_bytes: u64,
        progress: &Progress,
    ) -> Result<Arc<CacheLease>> {
        let mut leases = self.leases.lock().expect("cache lock poisoned");
        progress.check()?;
        let source = fs::canonicalize(source)?;
        let stamp = file_stamp(&source)?;
        let key = cache_key(&source, stamp);
        let target = self.directory.join(format!("{}.nii", cache_name(&key)));
        let meta = target.with_extension("nii.meta");

        if let Some(bytes) = cached_size(&target, &meta, &key, min_bytes) {
            if bytes > self.budget {
                return Err(Error::invalid("Cached volume exceeds cache budget"));
            }
            File::options()
                .write(true)
                .open(&target)?
                .set_modified(SystemTime::now())?;
            let lease = lease_for(&mut leases, &target);
            self.make_room(&mut leases, 0)?;
            progress.report(1.0, "Using cached volume")?;
            return Ok(lease);
        }

        self.make_room(&mut leases, min_bytes)?;

        let mut output = tempfile::Builder::new()
            .prefix("work-")
            .tempfile_in(&self.directory)?;
        let total = self.decompress_into(&mut leases, &source, &mut output, min_bytes, progress)?;
        if total < min_bytes {
            return Err(Error::invalid("Decompressed NIfTI payload is truncated"));
        }
        if file_stamp(&source)? != stamp {
            return Err(Error::invalid(
                "Source file changed while loading; reopen the volume",
            ));
        }
        progress.check()?;
        self.make_room(&mut leases, total)?;

        let mut metadata = tempfile::Builder::new()
            .prefix("work-")
            .tempfile_in(&self.directory)?;
        metadata.write_all(format!("{key}\n{total}").as_bytes())?;
        output.persist(&target).map_err(|error| error.error)?;
        if let Err(error) = metadata.persist(&meta) {
            let _ = fs::remove_file(&target);
            return Err(error.error.into());
        }
        let lease = lease_for(&mut leases, &target);
        progress.report(1.0, "Decompression complete")?;
        Ok(lease)
    }

    fn decompress_into(
        &self,
        leases: &mut Leases,
        source: &Path,
        output: &mut impl Write,
        min_bytes: u64,
        progress: &Progress,
    ) -> Result<u64> {
        let mut input = MultiGzDecoder::new(BufReader::new(File::open(source)?));
        let mut buffer = vec![0u8; 1024 * 1024];
        let mut total = 0u64;
        let mut reserved = min_bytes;
        loop {
            progress.check()?;
            let read = input
                .read(&mut buffer)
                .map_err(|error| Error::invalid(format!("Gzip decompression failed: {error}")))?;
            if read == 0 {
                return Ok(total);
            }
            let bytes = read as u64;
            if bytes > self.budget - total {
                return Err(Error::invalid("Decompressed volume exceeds cache budget"));
            }
            if total + bytes > reserved {
                reserved = total + bytes;
                self.make_room(leases, reserved)?;
            }
            output.write_all(&buffer[..read]).map_err(|error| {
                Error::invalid(format!(
                    "Cannot write decompressed volume; disk may be full ({error})"
                ))
            })?;
            total += bytes;
            let fraction = total as f64 / min_bytes.max(1) as f64;
            progress.report(fraction.min(0.99), "Decompressing")?;
        }
    }

    fn make_room(&self, leases: &mut Leases, incoming: u64) -> Result<()> {
        if incoming > self.budget {
            return Err(Error::invalid(
                "Volume exceeds cache budget; increase cache size or open an uncompressed file",
            ));
        }
        let mut entries = Vec::new();
        let mut total = 0u64;
        for entry in fs::read_dir(&self.directory)? {
            let entry = entry?;
            let path = entry.path();
            let info = entry.metadata()?;
            if !info.is_file() || !is_cache_file(&path) {
                continue;
            }
            total = total
                .checked_add(info.len())
                .ok_or_else(|| Error::invalid("Cache size overflow"))?;
            entries.push((info.modified()?, info.len(), path));
        }
        entries.sort();

        let limit = self.budget - incoming;
        for (_, bytes, path) in entries {
            if total <= limit {
                break;
            }
            let in_use = leases
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
            leases.remove(&path);
            total -= bytes;
        }
        if total > limit {
            return Err(Error::invalid(
                "Cache is full with active volumes. Increase cache size",
            ));
        }
        Ok(())
    }
}

fn lease_for(leases: &mut Leases, path: &Path) -> Arc<CacheLease> {
    if let Some(lease) = leases.get(path).and_then(Weak::upgrade) {
        return lease;
    }
    let lease = Arc::new(CacheLease {
        path: path.to_path_buf(),
    });
    leases.insert(path.to_path_buf(), Arc::downgrade(&lease));
    lease
}

fn cached_size(target: &Path, meta: &Path, key: &str, min_bytes: u64) -> Option<u64> {
    let bytes = fs::metadata(target)
        .ok()
        .filter(|info| info.is_file())?
        .len();
    let recorded = fs::read_to_string(meta).ok()?;
    (bytes >= min_bytes && recorded == format!("{key}\n{bytes}")).then_some(bytes)
}

fn file_stamp(path: &Path) -> Result<(u64, SystemTime)> {
    let info = fs::metadata(path)?;
    Ok((info.len(), info.modified()?))
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

fn cache_name(key: &str) -> String {
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
        let name = cache_name("some key");
        assert_eq!(name.len(), 39);
        assert!(is_cache_file(Path::new(&format!("{name}.nii"))));
        assert!(!is_cache_file(Path::new(&format!("{name}.nii.meta"))));
        assert!(!is_cache_file(Path::new("work-abc123")));
        assert_ne!(cache_name("some key"), cache_name("other key"));
    }

    #[test]
    fn decompresses_once_and_reuses() {
        let dir = tempfile::tempdir().unwrap();
        let cache = VolumeCache::new(dir.path().join("cache"), 1 << 20).unwrap();
        let source = dir.path().join("a.nii.gz");
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
    fn changed_source_gets_a_new_entry() {
        let dir = tempfile::tempdir().unwrap();
        let cache = VolumeCache::new(dir.path().join("cache"), 1 << 20).unwrap();
        let source = dir.path().join("a.nii.gz");
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
        let dir = tempfile::tempdir().unwrap();
        let cache = VolumeCache::new(dir.path().join("cache"), 1 << 20).unwrap();
        let good = dir.path().join("good.nii.gz");
        write_gz(&good, &data(5000, 0));
        let gzip = fs::read(&good).unwrap();

        let mut corrupt = gzip.clone();
        let checksum = corrupt.len() - 8;
        corrupt[checksum] ^= 0xFF;
        let truncated = gzip[..gzip.len() - 8].to_vec();

        for (name, bytes) in [("corrupt.nii.gz", corrupt), ("short.nii.gz", truncated)] {
            let path = dir.path().join(name);
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
        let dir = tempfile::tempdir().unwrap();
        let cache = VolumeCache::new(dir.path().join("cache"), 1 << 20).unwrap();
        let source = dir.path().join("a.nii.gz");
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
        let dir = tempfile::tempdir().unwrap();
        let cache = VolumeCache::new(dir.path().join("tiny"), 900).unwrap();
        let [a, b, c] = ["a", "b", "c"].map(|name| {
            let path = dir.path().join(format!("{name}.nii.gz"));
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
    fn rejects_volumes_larger_than_the_budget() {
        let dir = tempfile::tempdir().unwrap();
        let cache = VolumeCache::new(dir.path().join("cache"), 100).unwrap();
        let source = dir.path().join("a.nii.gz");
        write_gz(&source, &data(5000, 0));
        assert!(cache.acquire(&source, 5000, &Progress::default()).is_err());
        assert!(VolumeCache::new(dir.path().join("zero"), 0).is_err());
    }
}
