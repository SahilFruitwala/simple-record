//! The host app's identity: names shown in messages and the on-disk / Keychain
//! locations the engine uses. An app embedding zerorec calls [`set`] once at
//! startup, before anything else, so its users keep their existing settings,
//! logs and stream keys.

use std::sync::OnceLock;

#[derive(Debug, Clone)]
pub struct Brand {
    /// Command name in hints (`<cli> devices`).
    pub cli: &'static str,
    /// Prefix of the engine's stderr messages (`<prefix>: stopping...`).
    pub prefix: &'static str,
    /// Display name (permission messages, diagnostics folder).
    pub display: &'static str,
    /// Folder name under ~/Library/Logs and ~/Library/Application Support.
    pub dir: &'static str,
    /// Log file stem (`<stem>.log`, `<stem>.1.log`) and pid file stem.
    pub file_stem: &'static str,
    /// Keychain service for stream keys.
    pub keychain_service: &'static str,
    /// Encoder name sent to RTMP servers.
    pub encoder: &'static str,
    /// Crash report name prefixes collected into diagnostics.
    pub crash_prefixes: &'static [&'static str],
}

pub const ZEROREC: Brand = Brand {
    cli: "zerorec",
    prefix: "zerorec",
    display: "ZeroRec",
    dir: "zerorec",
    file_stem: "zerorec",
    keychain_service: "zerorec stream key",
    encoder: "zerorec",
    crash_prefixes: &["zerorec"],
};

static BRAND: OnceLock<Brand> = OnceLock::new();

/// Set the identity. Only the first call counts; returns false if one was
/// already set (or the default was already read).
pub fn set(b: Brand) -> bool {
    BRAND.set(b).is_ok()
}

pub fn get() -> &'static Brand {
    BRAND.get_or_init(|| ZEROREC)
}
