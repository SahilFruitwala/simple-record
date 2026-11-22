//! Plays a recording for the editor: AVPlayer for the sound, the clock and
//! seeking, and an `AVPlayerItemVideoOutput` that hands over each frame as
//! an IOSurface-backed BGRA buffer, which the UI draws without a copy (see
//! `preview::texture_from_buffer`).
//!
//! The picture can come from another file than the sound (a smart zoom
//! recording keeps the screen apart from the composed frame): both are put
//! in one `AVMutableComposition`, so they play as one item.

#![allow(deprecated)]

use std::path::Path;

use anyhow::{Result, anyhow};
use objc2::rc::Retained;
use objc2::{AnyThread, MainThreadMarker};
use objc2_av_foundation::*;
use objc2_core_foundation::CFRetained;
use objc2_core_media::{CMTime, CMTimeRange};
use objc2_core_video::{CVPixelBuffer, CVPixelBufferGetHeight, CVPixelBufferGetWidth};
use objc2_foundation::NSURL;

use crate::util::*;

pub struct Player {
    player: Retained<AVPlayer>,
    item: Retained<AVPlayerItem>,
    output: Retained<AVPlayerItemVideoOutput>,
    /// Seconds.
    pub duration: f64,
    /// Frame rate of the picture (for stepping a frame).
    pub fps: f64,
    /// The time of the last frame handed out.
    shown: f64,
    /// Where playback was last asked to go while paused (the item reports
    /// its time late after a seek).
    seek_to: Option<f64>,
}

fn asset(path: &Path) -> Retained<AVURLAsset> {
    unsafe { AVURLAsset::URLAssetWithURL_options(&NSURL::fileURLWithPath(&ns(&path.to_string_lossy())), None) }
}

fn time(t: f64) -> CMTime {
    unsafe { CMTime::with_seconds(t.max(0.0), 600) }
}

impl Player {
    /// Play the picture of `video` with the sound of `audio` (the first
    /// audio track: the mix).
    pub fn open(video: &Path, audio: &Path) -> Result<Player> {
        let mtm = MainThreadMarker::new().ok_or_else(|| anyhow!("the player must be opened on the main thread"))?;
        unsafe {
            let vt = AVMediaTypeVideo.ok_or_else(|| anyhow!("no video media type"))?;
            let at = AVMediaTypeAudio.ok_or_else(|| anyhow!("no audio media type"))?;
            let va = asset(video);
            let vtrack = va.tracksWithMediaType(vt).firstObject().ok_or_else(|| anyhow!("{}: no video", video.display()))?;
            let range = vtrack.timeRange();
            let fps = vtrack.nominalFrameRate() as f64;
            let comp = AVMutableComposition::composition();
            let add = |track: &AVAssetTrack, kind: &AVMediaType, range: CMTimeRange| -> Result<()> {
                let t = comp.addMutableTrackWithMediaType_preferredTrackID(kind, 0).ok_or_else(|| anyhow!("can't add a track"))?;
                t.insertTimeRange_ofTrack_atTime_error(range, track, CMTime::new(0, 600)).map_err(|e| anyhow!("{}", e.localizedDescription()))
            };
            add(&vtrack, vt, range)?;
            let aa = if audio == video { va.clone() } else { asset(audio) };
            if let Some(atrack) = aa.tracksWithMediaType(at).firstObject() {
                let ar = atrack.timeRange();
                // No longer than the picture.
                let ar = CMTimeRange { start: ar.start, duration: if secs(ar.duration) > secs(range.duration) { range.duration } else { ar.duration } };
                add(&atrack, at, ar)?;
            }
            let item = AVPlayerItem::playerItemWithAsset(&comp, mtm);
            let attrs = dict(vec![
                (cf_ns(objc2_core_video::kCVPixelBufferPixelFormatTypeKey), num_i(PIX_BGRA as i64)),
                (cf_ns(objc2_core_video::kCVPixelBufferIOSurfacePropertiesKey), obj(&*objc2_foundation::NSDictionary::<objc2_foundation::NSString, objc2::runtime::AnyObject>::new())),
                (cf_ns(objc2_core_video::kCVPixelBufferMetalCompatibilityKey), num_b(true)),
            ]);
            let output = AVPlayerItemVideoOutput::initWithPixelBufferAttributes(AVPlayerItemVideoOutput::alloc(), Some(&attrs));
            item.addOutput(&output);
            let player = AVPlayer::playerWithPlayerItem(Some(&item), mtm);
            // Stop at the end instead of going on to nothing.
            player.setActionAtItemEnd(AVPlayerActionAtItemEnd::Pause);
            let p = Player { player, item, output, duration: secs(range.duration).max(0.0), fps: if fps > 0.0 { fps } else { 30.0 }, shown: -1.0, seek_to: Some(0.0) };
            p.seek(0.0);
            Ok(p)
        }
    }

    pub fn playing(&self) -> bool {
        unsafe { self.player.rate() != 0.0 }
    }

    pub fn play(&mut self) {
        // From the start again when at the end.
        if self.time() >= self.duration - 0.05 {
            self.seek(0.0);
        }
        self.seek_to = None;
        unsafe { self.player.play() };
    }

    pub fn pause(&mut self) {
        unsafe { self.player.pause() };
    }

    pub fn toggle(&mut self) {
        if self.playing() { self.pause() } else { self.play() }
    }

    /// Go to `t` seconds, exactly (for scrubbing and stepping).
    pub fn seek(&self, t: f64) {
        let t = t.clamp(0.0, self.duration);
        let zero = time(0.0);
        unsafe { self.player.seekToTime_toleranceBefore_toleranceAfter(time(t), zero, zero) };
    }

    pub fn seek_to(&mut self, t: f64) {
        let t = t.clamp(0.0, self.duration);
        self.seek_to = (!self.playing()).then_some(t);
        self.seek(t);
    }

    /// The playhead in seconds.
    pub fn time(&self) -> f64 {
        if let Some(t) = self.seek_to {
            return t;
        }
        let t = secs(unsafe { self.item.currentTime() });
        if t.is_finite() { t.clamp(0.0, self.duration) } else { 0.0 }
    }

    /// The frame to show now, when it changed since the last call:
    /// (buffer, width, height).
    pub fn frame(&mut self) -> Option<(CFRetained<CVPixelBuffer>, u32, u32)> {
        unsafe {
            let t = self.item.currentTime();
            if !self.output.hasNewPixelBufferForItemTime(t) {
                return None;
            }
            let mut shown = CMTime::new(0, 600);
            let buf = self.output.copyPixelBufferForItemTime_itemTimeForDisplay(t, &mut shown)?;
            self.shown = secs(shown);
            if let Some(want) = self.seek_to
                && (self.shown - want).abs() < 1.5 / self.fps
            {
                self.seek_to = None;
            }
            // The same +1 reference, owned the CoreFoundation way.
            let buf = CFRetained::from_raw(std::ptr::NonNull::new_unchecked(Retained::into_raw(buf)));
            let (w, h) = (CVPixelBufferGetWidth(&buf) as u32, CVPixelBufferGetHeight(&buf) as u32);
            Some((buf, w, h))
        }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        unsafe { self.player.pause() };
    }
}
