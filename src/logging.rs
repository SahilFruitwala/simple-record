//! The app's log. Everything written to stderr also goes, timestamped, to
//! `~/Library/Logs/simple-record/rec.log` (rotated at 5 MB, one old file kept), so
//! a problem in an app opened from Finder, where stderr goes nowhere, can
//! still be diagnosed. Panics are logged with a backtrace and leave a
//! marker, so the next launch can offer to report them; `diagnostics`
//! gathers what a report needs.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The log is rotated past this size.
pub const MAX_LOG_BYTES: u64 = 5 << 20;
const LOG: &str = "rec.log";
const OLD_LOG: &str = "rec.1.log";
const CRASH_MARKER: &str = "last-crash.txt";

/// $REC_LOG_DIR, else ~/Library/Logs/simple-record.
pub fn log_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("REC_LOG_DIR") {
        return PathBuf::from(d);
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    home.join("Library/Logs/simple-record")
}

pub fn log_path() -> PathBuf {
    log_dir().join(LOG)
}

/// Local time as `YYYY-MM-DD HH:MM:SS.mmm`.
pub fn timestamp() -> String {
    let mut tv = libc::timeval { tv_sec: 0, tv_usec: 0 };
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe {
        libc::gettimeofday(&mut tv, std::ptr::null_mut());
        let t = tv.tv_sec;
        libc::localtime_r(&t, &mut tm);
    }
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec,
        tv.tv_usec / 1000
    )
}

/// Move `rec.log` to `rec.1.log` (replacing it) once it is over `max`.
pub fn rotate_if_needed(dir: &Path, max: u64) -> std::io::Result<bool> {
    let log = dir.join(LOG);
    if std::fs::metadata(&log).map(|m| m.len()).unwrap_or(0) <= max {
        return Ok(false);
    }
    std::fs::rename(&log, dir.join(OLD_LOG))?;
    Ok(true)
}

fn open_log(dir: &Path) -> Option<std::fs::File> {
    std::fs::create_dir_all(dir).ok()?;
    let _ = rotate_if_needed(dir, MAX_LOG_BYTES);
    std::fs::OpenOptions::new().create(true).append(true).open(dir.join(LOG)).ok()
}

/// The tee: the original stderr and the reader thread.
struct Tee {
    original: libc::c_int,
    thread: std::thread::JoinHandle<()>,
}

static TEE: Mutex<Option<Tee>> = Mutex::new(None);

/// Send stderr to the log file as well (timestamped lines), keeping it on
/// the original stderr too (a terminal, when there is one). Call once, early.
pub fn start() {
    let dir = log_dir();
    let Some(mut file) = open_log(&dir) else { return };
    let mut fds = [0 as libc::c_int; 2];
    unsafe {
        if libc::pipe(fds.as_mut_ptr()) != 0 {
            return;
        }
        let original = libc::dup(2);
        if original < 0 || libc::dup2(fds[1], 2) < 0 {
            libc::close(fds[0]);
            libc::close(fds[1]);
            return;
        }
        libc::close(fds[1]);
        let read_fd = fds[0];
        let _ = writeln!(file, "[{}] --- simple-record {} started (pid {})", timestamp(), env!("CARGO_PKG_VERSION"), std::process::id());
        let thread = std::thread::Builder::new().name("log".into()).spawn(move || {
            use std::os::fd::FromRawFd;
            let mut pipe = std::fs::File::from_raw_fd(read_fd);
            // The original stderr stays open (restored by `finish`).
            let mut out = std::mem::ManuallyDrop::new(std::fs::File::from_raw_fd(original));
            let mut buf = [0u8; 4096];
            let mut line: Vec<u8> = Vec::with_capacity(256);
            let mut written: u64 = 0;
            loop {
                let n = match pipe.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                let _ = out.write_all(&buf[..n]);
                for &b in &buf[..n] {
                    // Carriage returns (a live status line) end a line too.
                    if b == b'\n' || b == b'\r' {
                        if !line.is_empty() {
                            let _ = writeln!(file, "[{}] {}", timestamp(), String::from_utf8_lossy(&line));
                            written += line.len() as u64 + 26;
                            line.clear();
                        }
                    } else {
                        line.push(b);
                    }
                }
                if written > MAX_LOG_BYTES / 16 {
                    // Rotate while running too (a long session).
                    written = 0;
                    if rotate_if_needed(&dir, MAX_LOG_BYTES).unwrap_or(false)
                        && let Some(f) = open_log(&dir)
                    {
                        file = f;
                    }
                }
            }
            if !line.is_empty() {
                let _ = writeln!(file, "[{}] {}", timestamp(), String::from_utf8_lossy(&line));
            }
        });
        match thread {
            Ok(thread) => *TEE.lock().unwrap() = Some(Tee { original, thread }),
            Err(_) => {
                libc::dup2(original, 2);
                libc::close(original);
            }
        }
    }
}

/// Put stderr back and wait (briefly) for the last lines to reach the file.
pub fn finish() {
    let Some(t) = TEE.lock().unwrap().take() else { return };
    // The pipe's last writer goes, so the reader drains it and ends; it
    // echoes to the original stderr, which is closed only after that.
    unsafe { libc::dup2(t.original, 2) };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    while !t.thread.is_finished() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    if t.thread.is_finished() {
        unsafe { libc::close(t.original) };
    }
}

/// Log panics with a backtrace, and leave a marker for the next launch.
pub fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current().name().unwrap_or("unnamed").to_string();
        let msg = format!("panic in thread '{thread}': {info}\n{}", std::backtrace::Backtrace::force_capture());
        eprintln!("rec: {msg}");
        let _ = std::fs::create_dir_all(log_dir());
        let _ = std::fs::write(log_dir().join(CRASH_MARKER), format!("{}\n{msg}", timestamp()));
        default(info);
    }));
}

/// The panic the last run ended with, if any (read once: the marker goes).
pub fn take_last_crash() -> Option<String> {
    let p = log_dir().join(CRASH_MARKER);
    let text = std::fs::read_to_string(&p).ok()?;
    let _ = std::fs::remove_file(&p);
    Some(text)
}

/// Settings JSON with secrets removed: URL query strings (SRT passphrases
/// and stream IDs, tokens) are cut. Stream keys are never in settings.
pub fn redact_settings(json: &str) -> String {
    let Ok(mut v) = serde_json::from_str::<serde_json::Value>(json) else { return "(unreadable settings)".into() };
    fn walk(v: &mut serde_json::Value) {
        match v {
            serde_json::Value::String(s) => {
                if s.contains("://")
                    && let Some(i) = s.find('?')
                {
                    s.truncate(i);
                    s.push_str("?(redacted)");
                }
            }
            serde_json::Value::Array(a) => a.iter_mut().for_each(walk),
            serde_json::Value::Object(o) => o.values_mut().for_each(walk),
            _ => {}
        }
    }
    walk(&mut v);
    serde_json::to_string_pretty(&v).unwrap_or_default()
}

/// Make a folder with what a problem report needs: the logs, `system`
/// (a description of the machine and devices), the settings with secrets
/// removed, and macOS crash reports of the last `days` days. Returns it.
pub fn diagnostics(parent: &Path, system: &str, settings_json: Option<&str>, crash_reports: &Path, days: u64) -> std::io::Result<PathBuf> {
    let dir = parent.join(format!("Rec Diagnostics {}", timestamp().replace(':', ".")));
    std::fs::create_dir_all(&dir)?;
    for name in [LOG, OLD_LOG] {
        let p = log_dir().join(name);
        if p.exists() {
            std::fs::copy(&p, dir.join(name))?;
        }
    }
    std::fs::write(dir.join("system.txt"), system)?;
    if let Some(s) = settings_json {
        std::fs::write(dir.join("settings.json"), redact_settings(s))?;
    }
    // macOS writes native crashes (not panics) as simple-record-….ips.
    let cutoff = std::time::SystemTime::now() - std::time::Duration::from_secs(days * 86_400);
    if let Ok(entries) = std::fs::read_dir(crash_reports) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let recent = e.metadata().and_then(|m| m.modified()).is_ok_and(|t| t >= cutoff);
            if recent && name.starts_with("simple-record") && (name.ends_with(".ips") || name.ends_with(".crash")) {
                let _ = std::fs::copy(e.path(), dir.join(&name));
            }
        }
    }
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rec-log-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn rotation_keeps_one_old_log() {
        let d = temp("rotate");
        std::fs::write(d.join(LOG), vec![b'x'; 100]).unwrap();
        assert!(!rotate_if_needed(&d, 200).unwrap());
        assert!(rotate_if_needed(&d, 50).unwrap());
        assert!(!d.join(LOG).exists() && d.join(OLD_LOG).exists());
        std::fs::write(d.join(LOG), vec![b'y'; 100]).unwrap();
        assert!(rotate_if_needed(&d, 50).unwrap());
        assert_eq!(std::fs::read(d.join(OLD_LOG)).unwrap()[0], b'y', "the newer one is kept");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn settings_lose_their_secrets() {
        let json = r#"{"stream_custom_url":"srt://h:9000?passphrase=hunter2-hunter2&streamid=key","stream_extra":[{"custom_url":"https://x/whip?token=abc"}],"fps":60}"#;
        let out = redact_settings(json);
        assert!(!out.contains("hunter2") && !out.contains("streamid=key") && !out.contains("token=abc"), "{out}");
        assert!(out.contains("srt://h:9000?(redacted)") && out.contains("\"fps\": 60"));
    }

    #[test]
    fn timestamps_have_milliseconds() {
        let t = timestamp();
        assert_eq!(t.len(), 23, "{t}");
        assert_eq!(&t[4..5], "-");
        assert_eq!(&t[19..20], ".");
    }

    #[test]
    fn diagnostics_folder() {
        let d = temp("diag");
        let reports = d.join("reports");
        std::fs::create_dir_all(&reports).unwrap();
        std::fs::write(reports.join("simple-record-2026-10-18.ips"), "crash").unwrap();
        std::fs::write(reports.join("Safari-2026.ips"), "other app").unwrap();
        let out = diagnostics(&d, "macOS 15", Some(r#"{"stream_custom_url":"rtmp://h/app?k=secret"}"#), &reports, 7).unwrap();
        assert!(out.join("system.txt").exists());
        assert!(out.join("simple-record-2026-10-18.ips").exists());
        assert!(!out.join("Safari-2026.ips").exists(), "only Rec's crash reports");
        assert!(!std::fs::read_to_string(out.join("settings.json")).unwrap().contains("secret"));
        let _ = std::fs::remove_dir_all(&d);
    }
}
