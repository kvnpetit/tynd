//! Content-addressed cache of the zstd-compressed Bun runtime.
//!
//! Compressing the ~110 MB Bun binary at level 19 takes over a minute and the
//! result only depends on the binary, the zstd level and the zstd library, so
//! `tynd build` reuses it across builds. The cache lives in the per-user cache
//! directory rather than the project's `.tynd/cache`: one Bun binary serves
//! every project on the machine, the entry is ~40 MB (one copy per project
//! would multiply that), and `tynd clean` wipes `.tynd/cache`.
//!
//! An entry is trusted only after it decodes back to a binary of the expected
//! length and BLAKE3 hash; anything else is deleted and recompressed. Cache
//! failures never fail a build.

use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

use tracing::debug;

/// Bump when the entry layout or the key derivation changes.
const FORMAT_VERSION: u32 = 1;

/// Per-user cache directory for compressed Bun runtimes, or `None` when the
/// platform exposes no usable location (caching is then skipped).
pub(super) fn default_dir() -> Option<PathBuf> {
    let absolute = |value: std::ffi::OsString| {
        let path = PathBuf::from(value);
        path.is_absolute().then_some(path)
    };
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").and_then(absolute)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME")
            .and_then(absolute)
            .map(|home| home.join("Library/Caches"))
    } else {
        std::env::var_os("XDG_CACHE_HOME")
            .and_then(absolute)
            .or_else(|| {
                std::env::var_os("HOME")
                    .and_then(absolute)
                    .map(|home| home.join(".cache"))
            })
    }?;
    Some(base.join("tynd").join("bun-runtime"))
}

/// Cache file for a Bun binary with BLAKE3 `bun_hash` and `raw_len` bytes,
/// compressed at `level`.
pub(super) fn entry_path(dir: &Path, bun_hash: &blake3::Hash, raw_len: u64, level: i32) -> PathBuf {
    let key = format!(
        "tynd-bun-zst/v{FORMAT_VERSION}/{}/len={raw_len}/zstd={}/level={level}",
        bun_hash.to_hex(),
        zstd::zstd_safe::version_number(),
    );
    dir.join(format!("{}.zst", blake3::hash(key.as_bytes()).to_hex()))
}

/// Returns the cached compressed runtime when it decodes to the expected
/// binary; deletes an entry that does not.
pub(super) fn load(path: &Path, bun_hash: &blake3::Hash, raw_len: u64) -> Option<Vec<u8>> {
    let data = fs::read(path).ok()?;
    match verify(&data, bun_hash, raw_len) {
        Ok(()) => Some(data),
        Err(error) => {
            debug!(
                "discarding invalid bun runtime cache {}: {error}",
                path.display()
            );
            let _ = fs::remove_file(path);
            None
        },
    }
}

fn verify(data: &[u8], bun_hash: &blake3::Hash, raw_len: u64) -> io::Result<()> {
    let mut sink = HashingSink {
        hasher: blake3::Hasher::new(),
        len: 0,
        cap: raw_len,
    };
    zstd::stream::copy_decode(data, &mut sink)?;
    if sink.len != raw_len || sink.hasher.finalize() != *bun_hash {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "decoded runtime does not match the source binary",
        ));
    }
    Ok(())
}

/// Writes `data` to `path` through a temporary file in the same directory so a
/// concurrent reader never sees a partial entry.
pub(super) fn store(path: &Path, data: &[u8]) {
    let result = (|| -> io::Result<()> {
        let dir = path
            .parent()
            .ok_or_else(|| io::Error::other("cache path has no parent"))?;
        fs::create_dir_all(dir)?;
        let mut temporary = tempfile::Builder::new()
            .prefix(".bun-runtime-")
            .suffix(".tmp")
            .tempfile_in(dir)?;
        temporary.write_all(data)?;
        temporary.as_file().sync_all()?;
        temporary.persist(path).map_err(|error| error.error)?;
        Ok(())
    })();
    match result {
        Ok(()) => debug!("cached compressed bun runtime at {}", path.display()),
        Err(error) => debug!("could not cache compressed bun runtime: {error}"),
    }
}

/// Hashes decoded bytes without buffering them; refuses output beyond `cap`
/// so a corrupt entry cannot expand without bound.
struct HashingSink {
    hasher: blake3::Hasher,
    len: u64,
    cap: u64,
}

impl Write for HashingSink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.len += buf.len() as u64;
        if self.len > self.cap {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "decoded runtime is larger than the source binary",
            ));
        }
        self.hasher.update(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEVEL: i32 = 3;

    fn fixture() -> (Vec<u8>, blake3::Hash, Vec<u8>) {
        let raw: Vec<u8> = (0..64 * 1024).map(|index| (index % 251) as u8).collect();
        let hash = blake3::hash(&raw);
        let zst = zstd::encode_all(raw.as_slice(), LEVEL).unwrap();
        (raw, hash, zst)
    }

    #[test]
    fn stored_entry_is_reused() {
        let dir = tempfile::tempdir().unwrap();
        let (raw, hash, zst) = fixture();
        let path = entry_path(dir.path(), &hash, raw.len() as u64, LEVEL);
        assert!(load(&path, &hash, raw.len() as u64).is_none());
        store(&path, &zst);
        assert_eq!(load(&path, &hash, raw.len() as u64), Some(zst));
    }

    #[test]
    fn key_depends_on_binary_and_level() {
        let dir = Path::new("cache");
        let (raw, hash, _) = fixture();
        let len = raw.len() as u64;
        let base = entry_path(dir, &hash, len, LEVEL);
        assert_ne!(base, entry_path(dir, &hash, len, LEVEL + 1));
        assert_ne!(base, entry_path(dir, &blake3::hash(b"other"), len, LEVEL));
        assert_ne!(base, entry_path(dir, &hash, len + 1, LEVEL));
    }

    #[test]
    fn corrupt_or_mismatched_entry_is_discarded() {
        let dir = tempfile::tempdir().unwrap();
        let (raw, hash, zst) = fixture();
        let len = raw.len() as u64;

        let corrupt = entry_path(dir.path(), &hash, len, LEVEL);
        store(&corrupt, b"not zstd");
        assert!(load(&corrupt, &hash, len).is_none());
        assert!(!corrupt.exists());

        // Valid zstd that decodes to a different binary.
        let other = zstd::encode_all(&b"different runtime"[..], LEVEL).unwrap();
        store(&corrupt, &other);
        assert!(load(&corrupt, &hash, len).is_none());
        assert!(!corrupt.exists());

        store(&corrupt, &zst);
        assert!(load(&corrupt, &hash, len - 1).is_none());
    }
}
