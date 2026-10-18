//! A video file as a scene source: decoded on a thread of its own with
//! AVAssetReader and handed to the sink at the file's pace, as an extra
//! source (`Source::Media`), looped or holding its last frame. The file's
//! sound isn't played.

#![allow(deprecated)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use objc2::rc::Retained;
use objc2_av_foundation::*;
use objc2_core_media::CMSampleBuffer;
use objc2_foundation::NSURL;

use crate::layout::Source;
use crate::record::Shared;
use crate::util::*;

pub struct MediaSource {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

/// One pass over the file's first video track, as BGRA frames.
struct Pass {
    _reader: Retained<AVAssetReader>,
    out: Retained<AVAssetReaderTrackOutput>,
}

unsafe impl Send for Pass {}

fn open(path: &str) -> Result<Pass> {
    unsafe {
        let url = NSURL::fileURLWithPath(&ns(path));
        let asset = AVURLAsset::URLAssetWithURL_options(&url, None);
        let video = AVMediaTypeVideo.ok_or_else(|| anyhow!("no video media type"))?;
        let track = asset.tracksWithMediaType(video).firstObject().ok_or_else(|| anyhow!("{path}: no video track"))?;
        let reader = AVAssetReader::assetReaderWithAsset_error(&asset).map_err(|e| anyhow!("{path}: {}", e.localizedDescription()))?;
        let settings = dict(vec![(cf_ns(objc2_core_video::kCVPixelBufferPixelFormatTypeKey), num_i(PIX_BGRA as i64))]);
        let out = AVAssetReaderTrackOutput::assetReaderTrackOutputWithTrack_outputSettings(&track, Some(&settings));
        out.setAlwaysCopiesSampleData(false);
        reader.addOutput(&out);
        if !reader.startReading() {
            bail!("{path}: can't be read ({})", reader.error().map(|e| e.localizedDescription().to_string()).unwrap_or_default());
        }
        Ok(Pass { _reader: reader, out })
    }
}

/// Sleep until `t`, or until stopped (checked every 50 ms).
fn sleep_until(t: Instant, stop: &AtomicBool) {
    while !stop.load(Ordering::SeqCst) {
        let now = Instant::now();
        if now >= t {
            return;
        }
        std::thread::sleep((t - now).min(Duration::from_millis(50)));
    }
}

impl MediaSource {
    /// Play `path` into `shared` as `src` (fails if it can't be read).
    pub fn start(path: &str, looped: bool, src: Source, shared: Arc<Shared>) -> Result<MediaSource> {
        let first = open(path)?;
        let stop = Arc::new(AtomicBool::new(false));
        let st = stop.clone();
        let path = path.to_string();
        let thread = std::thread::Builder::new().name("media".into()).spawn(move || {
            let mut pass = Some(first);
            while let Some(p) = pass.take() {
                let start = Instant::now();
                let mut t0 = None;
                let mut frames = 0u64;
                while !st.load(Ordering::SeqCst) {
                    let Some(sb) = (unsafe { p.out.copyNextSampleBuffer() }) else { break };
                    let sb: Retained<CMSampleBuffer> = sb;
                    let Some(pb) = (unsafe { sb.image_buffer() }) else { continue };
                    let pts = secs(unsafe { sb.presentation_time_stamp() });
                    let t0 = *t0.get_or_insert(pts);
                    sleep_until(start + Duration::from_secs_f64((pts - t0).max(0.0)), &st);
                    shared.with(|s| s.push_extra(&src, host_now(), pb));
                    frames += 1;
                }
                // Again from the start (a file that gave no frames isn't
                // retried: it would spin).
                if looped && frames > 0 && !st.load(Ordering::SeqCst) {
                    match open(&path) {
                        Ok(p) => pass = Some(p),
                        Err(e) => eprintln!("rec: warning: {e:#}"),
                    }
                }
            }
        })?;
        Ok(MediaSource { stop, thread: Some(thread) })
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for MediaSource {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file's frames come out as BGRA, all of them.
    #[test]
    fn reads_every_frame() {
        let dir = std::env::temp_dir().join(format!("rec-media-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("clip.mp4");
        let ok = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=320x180:rate=10", "-t", "1", "-pix_fmt", "yuv420p"])
            .arg(&file)
            .status()
            .expect("ffmpeg (brew install ffmpeg)");
        assert!(ok.success());
        let p = open(file.to_str().unwrap()).unwrap();
        let mut n = 0;
        while let Some(sb) = unsafe { p.out.copyNextSampleBuffer() } {
            let pb = unsafe { sb.image_buffer() }.unwrap();
            use objc2_core_video::*;
            assert_eq!(CVPixelBufferGetPixelFormatType(&pb), PIX_BGRA);
            assert_eq!((CVPixelBufferGetWidth(&pb), CVPixelBufferGetHeight(&pb)), (320, 180));
            n += 1;
        }
        assert_eq!(n, 10);
        assert!(open(dir.join("missing.mp4").to_str().unwrap()).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
