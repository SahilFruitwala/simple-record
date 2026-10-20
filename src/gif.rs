//! GIF export: a recording's video decoded with AVAssetReader, scaled with
//! CoreImage's Lanczos filter and written frame by frame with ImageIO. No
//! sound (GIFs have none).

#![allow(deprecated)]

use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow, bail, ensure};
use objc2::rc::Retained;
use objc2_av_foundation::*;
use objc2_core_foundation::{CFDictionary, CFString, CFURL, CGRect};
use objc2_core_graphics::CGImage;
use objc2_core_image::{CIContext, CIImage};
use objc2_core_media::{CMTime, CMTimeRange};
use objc2_foundation::{NSDictionary, NSString, NSURL};
use objc2_image_io::*;

use crate::util::*;

/// What to export.
#[derive(Clone, Debug, PartialEq)]
pub struct GifOptions {
    /// Frames per second (1-50; GIF delays are in hundredths of a second).
    pub fps: u32,
    /// The largest width in pixels (never upscaled).
    pub width: u32,
    /// Seconds into the recording.
    pub start: f64,
    /// Seconds to export; None: to the end.
    pub duration: Option<f64>,
}

impl Default for GifOptions {
    fn default() -> Self {
        GifOptions { fps: 15, width: 720, start: 0.0, duration: None }
    }
}

#[derive(Debug)]
pub struct GifStats {
    pub frames: usize,
    pub size: (usize, usize),
    pub bytes: u64,
}

/// `rec-….mov` -> `rec-….gif`.
pub fn gif_path(input: &Path) -> PathBuf {
    input.with_extension("gif")
}

/// Frame `i`'s delay in hundredths of a second: whole hundredths that keep
/// the frames to `fps` on average (15 fps is 7, 7, 6, 7, 7, 6…).
pub fn delay_cs(i: u64, fps: u32) -> u32 {
    let at = |n: u64| (n * 100 + fps as u64 / 2) / fps as u64;
    (at(i + 1) - at(i)) as u32
}

/// The output size for a `w`×`h` video at most `max_w` wide: the same
/// shape, never larger, at least 2×2.
pub fn out_size(w: usize, h: usize, max_w: u32) -> (usize, usize) {
    let k = (max_w as f64 / w.max(1) as f64).min(1.0);
    (((w as f64 * k).round() as usize).max(2), ((h as f64 * k).round() as usize).max(2))
}

/// The length in seconds of a recording's video.
pub fn duration(input: &Path) -> Result<f64> {
    unsafe {
        let url = NSURL::fileURLWithPath(&ns(&input.to_string_lossy()));
        let asset = AVURLAsset::URLAssetWithURL_options(&url, None);
        Ok(secs(asset.duration()))
    }
}

fn cf<T>(d: &NSDictionary<NSString, T>) -> &CFDictionary {
    // SAFETY: NSDictionary and CFDictionary are toll-free bridged.
    unsafe { &*(d as *const NSDictionary<NSString, T> as *const CFDictionary) }
}

fn key(s: &CFString) -> &NSString {
    cf_ns(s)
}

/// Write `input`'s video (from `o.start`, for `o.duration`) to `output` as
/// a looping GIF. `progress` gets the share done (0-1) now and then.
pub fn export(input: &Path, output: &Path, o: &GifOptions, progress: &dyn Fn(f64)) -> Result<GifStats> {
    ensure!((1..=50).contains(&o.fps), "the frame rate must be 1-50 fps");
    ensure!(o.width >= 2, "the width must be at least 2 pixels");
    let r = write(input, output, o, progress);
    if r.is_err() {
        let _ = std::fs::remove_file(output);
    }
    r
}

fn write(input: &Path, output: &Path, o: &GifOptions, progress: &dyn Fn(f64)) -> Result<GifStats> {
    let name = input.display();
    unsafe {
        let url = NSURL::fileURLWithPath(&ns(&input.to_string_lossy()));
        let asset = AVURLAsset::URLAssetWithURL_options(&url, None);
        let video = AVMediaTypeVideo.ok_or_else(|| anyhow!("no video media type"))?;
        let track = asset.tracksWithMediaType(video).firstObject().ok_or_else(|| anyhow!("{name}: no video track"))?;
        let total = secs(asset.duration());
        let start = o.start.clamp(0.0, total.max(0.0));
        let end = o.duration.map_or(total, |d| (start + d.max(0.0)).min(total));
        ensure!(end - start > 0.01, "{name}: nothing to export between {start:.2}s and {end:.2}s");
        let n = ((end - start) * o.fps as f64).ceil().max(1.0) as u64;

        let reader = AVAssetReader::assetReaderWithAsset_error(&asset).map_err(|e| anyhow!("{name}: {}", e.localizedDescription()))?;
        let settings = dict(vec![(cf_ns(objc2_core_video::kCVPixelBufferPixelFormatTypeKey), num_i(PIX_BGRA as i64))]);
        let out = AVAssetReaderTrackOutput::assetReaderTrackOutputWithTrack_outputSettings(&track, Some(&settings));
        out.setAlwaysCopiesSampleData(false);
        reader.addOutput(&out);
        reader.setTimeRange(CMTimeRange { start: CMTime::with_seconds(start, 600), duration: CMTime::with_seconds(end - start, 600) });
        if !reader.startReading() {
            bail!("{name}: can't be read ({})", reader.error().map(|e| e.localizedDescription().to_string()).unwrap_or_default());
        }

        let out_url = NSURL::fileURLWithPath(&ns(&output.to_string_lossy()));
        // SAFETY: NSURL and CFURL are toll-free bridged.
        let out_url = &*(&*out_url as *const NSURL as *const CFURL);
        let dest = CGImageDestination::with_url(out_url, &CFString::from_str("com.compuserve.gif"), n as usize, None)
            .ok_or_else(|| anyhow!("can't write {}", output.display()))?;
        // Loop forever.
        let looping = dict(vec![(key(kCGImagePropertyGIFLoopCount), num_i(0))]);
        let props = dict(vec![(key(kCGImagePropertyGIFDictionary), Retained::into_super(Retained::into_super(looping)))]);
        dest.set_properties(Some(cf(&props)));

        let ctx = CIContext::contextWithOptions(None);
        let mut size = None;
        // The frame shown at each GIF frame's time: the last one decoded at
        // or before it.
        let mut shown: Option<Retained<CGImage>> = None;
        let mut written = 0u64;
        let add = |img: &CGImage, written: &mut u64| {
            let d = delay_cs(*written, o.fps) as f64 / 100.0;
            let timing = dict(vec![(key(kCGImagePropertyGIFDelayTime), num_f(d)), (key(kCGImagePropertyGIFUnclampedDelayTime), num_f(d))]);
            let props = dict(vec![(key(kCGImagePropertyGIFDictionary), Retained::into_super(Retained::into_super(timing)))]);
            dest.add_image(img, Some(cf(&props)));
            *written += 1;
            if written.is_multiple_of(10) {
                progress(*written as f64 / n as f64);
            }
        };
        let slot = |i: u64| start + i as f64 / o.fps as f64;
        while written < n {
            let Some(sb) = out.copyNextSampleBuffer() else { break };
            let Some(pb) = sb.image_buffer() else { continue };
            let pts = secs(sb.presentation_time_stamp());
            // Slots before this frame show the previous one.
            if let Some(prev) = &shown {
                while written < n && slot(written) < pts - 1e-6 {
                    add(prev, &mut written);
                }
            }
            let (pw, ph) = (objc2_core_video::CVPixelBufferGetWidth(&pb), objc2_core_video::CVPixelBufferGetHeight(&pb));
            let (w, h) = *size.get_or_insert_with(|| out_size(pw, ph, o.width));
            // Clamped first, so the scaler doesn't fade the edges to
            // transparent (a dark or see-through border in the GIF).
            let img = CIImage::imageWithCVPixelBuffer(&pb).imageByClampingToExtent();
            let params = dict(vec![
                (&*ns("inputScale"), num_f(h as f64 / ph as f64)),
                (&*ns("inputAspectRatio"), num_f((w as f64 / pw as f64) / (h as f64 / ph as f64))),
            ]);
            let scaled = img.imageByApplyingFilter_withInputParameters(&ns("CILanczosScaleTransform"), &params);
            let bounds = CGRect::new(objc2_core_foundation::CGPoint::new(0.0, 0.0), objc2_core_foundation::CGSize::new(w as f64, h as f64));
            shown = Some(ctx.createCGImage_fromRect(&scaled, bounds).ok_or_else(|| anyhow!("{name}: a frame couldn't be scaled"))?);
        }
        if let Some(r) = reader.error() {
            bail!("{name}: {}", r.localizedDescription());
        }
        // The rest show the last frame (a file that ends early).
        let last = shown.ok_or_else(|| anyhow!("{name}: no frames between {start:.2}s and {end:.2}s"))?;
        while written < n {
            add(&last, &mut written);
        }
        if !dest.finalize() {
            bail!("couldn't finish {}", output.display());
        }
        progress(1.0);
        let (w, h) = size.unwrap_or_default();
        Ok(GifStats { frames: written as usize, size: (w, h), bytes: std::fs::metadata(output).map(|m| m.len()).unwrap_or(0) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delays_keep_the_rate() {
        assert_eq!((0..3).map(|i| delay_cs(i, 15)).collect::<Vec<_>>(), [7, 6, 7]);
        for fps in [1, 10, 12, 15, 20, 24, 25, 30, 50] {
            let total: u32 = (0..fps as u64 * 10).map(|i| delay_cs(i, fps)).sum();
            assert_eq!(total, 1000, "{fps} fps: 10 s");
            assert!((0..fps as u64 * 10).all(|i| delay_cs(i, fps) >= 2), "browsers slow down delays under 2");
        }
    }

    #[test]
    fn sizes() {
        assert_eq!(out_size(1920, 1080, 720), (720, 405));
        assert_eq!(out_size(640, 360, 720), (640, 360));
        assert_eq!(out_size(1080, 1920, 480), (480, 853));
        assert_eq!(out_size(4000, 10, 100), (100, 2));
    }

    /// A 2 s, 30 fps clip with a moving box, to a 10 fps GIF of its second
    /// half: 10 frames, the right size, read back by ImageIO.
    #[test]
    fn exports_a_clip() {
        let dir = std::env::temp_dir().join(format!("rec-gif-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let clip = dir.join("clip.mp4");
        let ok = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=640x360:rate=30", "-t", "2", "-pix_fmt", "yuv420p"])
            .arg(&clip)
            .status()
            .expect("ffmpeg (brew install ffmpeg)");
        assert!(ok.success());
        let gif = gif_path(&clip);
        let o = GifOptions { fps: 10, width: 320, start: 1.0, duration: Some(1.0) };
        let calls = std::cell::Cell::new(0);
        let stats = export(&clip, &gif, &o, &|p| {
            assert!((0.0..=1.0).contains(&p));
            calls.set(calls.get() + 1);
        })
        .unwrap();
        assert_eq!((stats.frames, stats.size), (10, (320, 180)));
        assert!(calls.get() >= 1 && stats.bytes > 1000);
        unsafe {
            let url = NSURL::fileURLWithPath(&ns(&gif.to_string_lossy()));
            let url = &*(&*url as *const NSURL as *const CFURL);
            let src = CGImageSource::with_url(url, None).expect("readable GIF");
            assert_eq!(src.count(), 10);
            assert_eq!(src.r#type().map(|t| t.to_string()).as_deref(), Some("com.compuserve.gif"));
        }
        // Past the end: an error, and no file left behind.
        let bad = dir.join("bad.gif");
        assert!(export(&clip, &bad, &GifOptions { start: 5.0, ..Default::default() }, &|_| {}).is_err());
        assert!(!bad.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
