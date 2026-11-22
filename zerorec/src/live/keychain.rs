//! Stream keys live in the macOS Keychain (generic passwords), never in
//! the settings file or logs.

use anyhow::{Result, anyhow};
use security_framework::passwords;

fn service() -> &'static str {
    crate::brand::get().keychain_service
}

/// Keychain account for a destination (service id or server URL).
pub fn account(dest: &str) -> String {
    dest.trim().trim_end_matches('/').to_string()
}

pub fn get(dest: &str) -> Option<String> {
    passwords::get_generic_password(service(), &account(dest)).ok().and_then(|b| String::from_utf8(b).ok())
}

pub fn set(dest: &str, key: &str) -> Result<()> {
    passwords::set_generic_password(service(), &account(dest), key.as_bytes()).map_err(|e| anyhow!("Keychain: {e}"))
}

pub fn delete(dest: &str) -> Result<()> {
    passwords::delete_generic_password(service(), &account(dest)).map_err(|e| anyhow!("Keychain: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let dest = format!("zerorec-test://{}", std::process::id());
        set(&dest, "sk_live_123").unwrap();
        assert_eq!(get(&dest).as_deref(), Some("sk_live_123"));
        set(&dest, "sk_live_456").unwrap();
        assert_eq!(get(&dest).as_deref(), Some("sk_live_456"));
        delete(&dest).unwrap();
        assert_eq!(get(&dest), None);
    }
}
