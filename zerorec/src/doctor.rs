//! `zerorec doctor`: one report covering what most support questions come
//! down to (hardware, macOS version, permissions, encoders, devices, disk).

use std::ffi::{CString, c_void};
use std::path::PathBuf;
use std::ptr::NonNull;

use objc2::rc::Retained;
use objc2_core_foundation::CFDictionary;
use objc2_core_media::{CMVideoCodecType, kCMVideoCodecType_AppleProRes422, kCMVideoCodecType_H264, kCMVideoCodecType_HEVC};
use objc2_video_toolbox::*;

use crate::tcc::{self, Status};
use crate::util::*;
use crate::{camera, disk, screen, writer::Codec};

/// Oldest macOS the app supports (support/Info.plist LSMinimumSystemVersion).
const MIN_MACOS: u32 = 15;

fn sysctl_string(name: &str) -> Option<String> {
    let c = CString::new(name).ok()?;
    let mut len = 0usize;
    unsafe {
        if libc::sysctlbyname(c.as_ptr(), std::ptr::null_mut(), &mut len, std::ptr::null_mut(), 0) != 0 || len == 0 {
            return None;
        }
        let mut buf = vec![0u8; len];
        if libc::sysctlbyname(c.as_ptr(), buf.as_mut_ptr() as *mut c_void, &mut len, std::ptr::null_mut(), 0) != 0 {
            return None;
        }
        buf.truncate(len);
        while buf.last() == Some(&0) {
            buf.pop();
        }
        String::from_utf8(buf).ok()
    }
}

fn sysctl_u64(name: &str) -> Option<u64> {
    let c = CString::new(name).ok()?;
    let mut v = 0u64;
    let mut len = std::mem::size_of::<u64>();
    let r = unsafe { libc::sysctlbyname(c.as_ptr(), &mut v as *mut u64 as *mut c_void, &mut len, std::ptr::null_mut(), 0) };
    // Some keys are 32-bit; on little-endian the low bytes land first.
    if r == 0 { Some(if len == 4 { v & 0xffff_ffff } else { v }) } else { None }
}

#[derive(Debug, PartialEq, Eq)]
enum Encoder {
    Hardware,
    Software,
    Unavailable,
}

/// Try a 1080p compression session: hardware required first, then any.
fn encoder(codec: CMVideoCodecType) -> Encoder {
    let try_create = |require_hw: bool| -> bool {
        let key = if require_hw {
            unsafe { kVTVideoEncoderSpecification_RequireHardwareAcceleratedVideoEncoder }
        } else {
            unsafe { kVTVideoEncoderSpecification_EnableHardwareAcceleratedVideoEncoder }
        };
        let spec: Retained<Dict> = dict(vec![(cf_ns(key), num_b(true))]);
        let mut out: *mut VTCompressionSession = std::ptr::null_mut();
        unsafe {
            VTCompressionSession::create(
                None,
                1920,
                1080,
                codec,
                Some(&*(Retained::as_ptr(&spec) as *const CFDictionary)),
                None,
                None,
                None,
                std::ptr::null_mut(),
                NonNull::from(&mut out),
            )
        };
        match NonNull::new(out) {
            Some(s) => {
                let s = unsafe { objc2_core_foundation::CFRetained::from_raw(s) };
                unsafe { s.invalidate() };
                true
            }
            None => false,
        }
    };
    if try_create(true) {
        Encoder::Hardware
    } else if try_create(false) {
        Encoder::Software
    } else {
        Encoder::Unavailable
    }
}

fn mark(ok: bool) -> &'static str {
    if ok { "ok  " } else { "FAIL" }
}

fn warn_mark(ok: bool) -> &'static str {
    if ok { "ok  " } else { "WARN" }
}

/// Print the report. Returns the number of failed checks.
pub fn run() -> usize {
    let mut failures = 0;
    let mut fail = |ok: bool| {
        if !ok {
            failures += 1;
        }
        ok
    };

    let brand = crate::brand::get();
    println!("{} doctor (engine: zerorec {})", brand.cli, env!("CARGO_PKG_VERSION"));

    println!("\nHardware");
    let chip = sysctl_string("machdep.cpu.brand_string").unwrap_or_else(|| "unknown".into());
    let apple = chip.starts_with("Apple");
    println!("  [{}] chip             {chip}", warn_mark(apple));
    let cores = sysctl_u64("hw.ncpu").unwrap_or(0);
    match (sysctl_u64("hw.perflevel0.physicalcpu"), sysctl_u64("hw.perflevel1.physicalcpu")) {
        (Some(p), Some(e)) => println!("  [ok  ] cores            {cores} ({p} performance, {e} efficiency)"),
        _ => println!("  [ok  ] cores            {cores}"),
    }
    let mem = sysctl_u64("hw.memsize").unwrap_or(0);
    println!("  [ok  ] memory           {:.0} GB", mem as f64 / (1u64 << 30) as f64);
    if !apple {
        println!("         note: ZeroRec's zero-copy path is tuned for Apple silicon");
    }

    println!("\nmacOS");
    let os = sysctl_string("kern.osproductversion").unwrap_or_else(|| "unknown".into());
    let major: u32 = os.split('.').next().and_then(|m| m.parse().ok()).unwrap_or(0);
    let os_ok = fail(major >= MIN_MACOS);
    let build = sysctl_string("kern.osversion").unwrap_or_default();
    println!("  [{}] version          {os} ({build})", mark(os_ok));
    if !os_ok {
        println!("         ZeroRec needs macOS {MIN_MACOS} or later (ScreenCaptureKit microphone + system audio capture)");
    }

    println!("\nPermissions");
    let perms = [
        ("screen recording", tcc::screen_status(), true),
        ("microphone", tcc::mic_status(), false),
        ("camera", tcc::camera_status(), false),
    ];
    let mut missing = false;
    for (name, st, required) in perms {
        let ok = st == Status::Granted;
        missing |= !ok;
        let tag = if ok {
            "ok  "
        } else if required {
            fail(false);
            "FAIL"
        } else {
            "WARN"
        };
        println!("  [{tag}] {name:<16} {st}");
    }
    if missing {
        println!("         fix: `zerorec permissions --request`, then System Settings > Privacy & Security");
    }

    println!("\nVideo encoders (VideoToolbox, 1080p)");
    for (name, codec, required) in [
        ("H.264", kCMVideoCodecType_H264, true),
        ("HEVC", kCMVideoCodecType_HEVC, true),
        ("ProRes 422", kCMVideoCodecType_AppleProRes422, false),
    ] {
        let e = encoder(codec);
        let tag = match e {
            Encoder::Hardware => "ok  ",
            Encoder::Software if !required => "ok  ",
            Encoder::Software => "WARN",
            Encoder::Unavailable => {
                fail(false);
                "FAIL"
            }
        };
        let what = match e {
            Encoder::Hardware => "hardware",
            Encoder::Software => "software only (higher CPU use)",
            Encoder::Unavailable => "unavailable",
        };
        println!("  [{tag}] {name:<16} {what}");
    }

    println!("\nDisplays");
    let screen_ok = tcc::screen_status() == Status::Granted;
    match screen::displays() {
        Err(_) if !screen_ok => println!("  [WARN] not listed (needs the screen recording permission)"),
        Ok(ds) if !ds.is_empty() => {
            for (i, d) in ds.iter().enumerate() {
                println!("  [ok  ] [{}] {}x{} pixels ({}x{} points), id {}", i + 1, d.pixels.0, d.pixels.1, d.points.0, d.points.1, d.id);
            }
        }
        Ok(_) => {
            fail(false);
            println!("  [FAIL] no displays found");
        }
        Err(e) => {
            fail(false);
            println!("  [FAIL] unavailable: {e}");
        }
    }

    println!("\nAudio inputs");
    let mics = camera::microphones();
    if mics.is_empty() {
        println!("  [WARN] no microphones found");
    }
    for m in mics {
        println!("  [ok  ] {}", m.name);
    }

    println!("\nCameras");
    let cams = camera::cameras();
    if cams.is_empty() {
        println!("  [ok  ] none connected");
    }
    for c in cams {
        println!("  [ok  ] {}", c.name);
    }

    println!("\nStorage");
    let movies = std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Movies")).unwrap_or_default();
    match disk::free_bytes(&movies) {
        Some(free) => {
            let tag = if free < disk::STOP_BELOW {
                fail(false);
                "FAIL"
            } else if free < disk::warn_threshold(Codec::Prores) {
                "WARN"
            } else {
                "ok  "
            };
            println!("  [{tag}] {}  {} free", movies.display(), disk::human(free));
            println!("         ~300 MB/min at 4K60 HEVC, ~1.5-2.5 GB/min with ProRes");
        }
        None => println!("  [WARN] {}  free space unknown", movies.display()),
    }

    println!();
    if failures == 0 {
        println!("All required checks passed.");
    } else {
        println!("{failures} required check(s) failed.");
    }
    failures
}
