//! Free disk space checks: before a recording starts and while it runs, so a
//! full volume ends the session cleanly (finalized file) instead of failing
//! the writer mid-stream.

use std::ffi::CString;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

use crate::writer::Codec;

pub const MB: u64 = 1_000_000;
/// Below this, recording is refused (preflight) or stopped (runtime).
pub const STOP_BELOW: u64 = 100 * MB;
/// Preflight warning thresholds.
pub const WARN_BELOW_COMPRESSED: u64 = 500 * MB;
pub const WARN_BELOW_PRORES: u64 = 2_000 * MB;

/// The nearest existing directory for `path` (the output file usually
/// doesn't exist yet, and its folder may be created later).
fn existing_ancestor(path: &Path) -> PathBuf {
    let mut p = if path.is_dir() { path } else { path.parent().unwrap_or(Path::new(".")) };
    loop {
        if p.as_os_str().is_empty() {
            return PathBuf::from(".");
        }
        if p.exists() {
            return p.to_path_buf();
        }
        match p.parent() {
            Some(parent) => p = parent,
            None => return PathBuf::from("/"),
        }
    }
}

/// Bytes available to this user on the volume holding `path`.
pub fn free_bytes(path: &Path) -> Option<u64> {
    let dir = existing_ancestor(path);
    let c = CString::new(dir.as_os_str().as_encoded_bytes()).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    Some(st.f_bavail as u64 * st.f_frsize as u64)
}

pub fn warn_threshold(codec: Codec) -> u64 {
    if codec == Codec::Prores { WARN_BELOW_PRORES } else { WARN_BELOW_COMPRESSED }
}

pub fn human(bytes: u64) -> String {
    if bytes >= 1_000 * MB { format!("{:.1} GB", bytes as f64 / 1e9) } else { format!("{:.0} MB", bytes as f64 / 1e6) }
}

/// What the preflight found.
#[derive(Debug, PartialEq, Eq)]
pub enum Preflight {
    Ok,
    Low(u64),
    Unknown,
}

/// Decide from a free-space figure (pure, for tests).
pub fn judge(free: Option<u64>, codec: Codec) -> Result<Preflight> {
    let Some(free) = free else { return Ok(Preflight::Unknown) };
    if free < STOP_BELOW {
        bail!(
            "not enough disk space: {} free on the output volume (needs at least {}); free up space or choose another --out",
            human(free),
            human(STOP_BELOW)
        );
    }
    Ok(if free < warn_threshold(codec) { Preflight::Low(free) } else { Preflight::Ok })
}

/// Check the output volume before recording. Errors when it's too full;
/// returns a warning to print when it's low.
pub fn preflight(output: &Path, codec: Codec) -> Result<Option<String>> {
    Ok(match judge(free_bytes(output), codec)? {
        Preflight::Low(free) => Some(format!(
            "low disk space: {} free on the output volume (recommended at least {} for {:?}); recording stops automatically below {}",
            human(free),
            human(warn_threshold(codec)),
            codec,
            human(STOP_BELOW)
        )),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thresholds() {
        assert!(judge(Some(50 * MB), Codec::H264).is_err());
        assert_eq!(judge(Some(300 * MB), Codec::Hevc).unwrap(), Preflight::Low(300 * MB));
        assert_eq!(judge(Some(800 * MB), Codec::H264).unwrap(), Preflight::Ok);
        assert_eq!(judge(Some(800 * MB), Codec::Prores).unwrap(), Preflight::Low(800 * MB));
        assert_eq!(judge(Some(3_000 * MB), Codec::Prores).unwrap(), Preflight::Ok);
        assert_eq!(judge(None, Codec::H264).unwrap(), Preflight::Unknown);
    }

    #[test]
    fn free_space_of_a_file_that_does_not_exist_yet() {
        let p = std::env::temp_dir().join("no/such/dir/out.mov");
        assert!(free_bytes(&p).unwrap() > 0);
        assert!(free_bytes(Path::new("relative-out.mov")).is_some());
    }
}
