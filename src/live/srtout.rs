//! The SRT stream output: packets from the shared encode are muxed into
//! MPEG-TS here and sent by a worker thread over `srt::Conn`, which resends
//! what the receiver reports lost. Reconnects with backoff; under
//! congestion video is dropped up to the next keyframe, as for RTMP.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;

use super::audio::AudioCodec;
use super::srt::{self, Conn, SrtUrl};
use super::ts::{self, Muxer};
use super::{Formats, Output, OutputStatus, Packet};

/// Media time ahead of the stream's start (90 kHz), so the clock
/// reference (`ts::PCR_LEAD` earlier) is never negative.
const PTS_BASE: u64 = 2 * ts::PCR_LEAD;

/// Queued media spanning more than this means the network is behind.
const MAX_QUEUE_SECS: f64 = 3.0;

/// One SRT payload of TS packets and the media time it carries.
struct Chunk {
    t: f64,
    video: bool,
    data: Vec<u8>,
}

#[derive(Default)]
struct Shared {
    queue: VecDeque<Chunk>,
    state: String,
    error: Option<String>,
    live_since: Option<Instant>,
    dropped: u64,
    reconnects: u64,
    samples: VecDeque<(Instant, u64)>,
    want_key: bool,
    skip_to_key: bool,
}

pub struct SrtOutput {
    name: String,
    shared: Arc<(Mutex<Shared>, Condvar)>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    t0: Option<f64>,
    mux: Option<Muxer>,
    /// TS packets not yet filling a whole payload.
    pending: Vec<u8>,
}

impl SrtOutput {
    pub fn start(url: &str, key: &str) -> Result<SrtOutput> {
        let u = srt::parse_url(url, key)?;
        let name = format!("srt://{}:{}", u.host, u.port);
        let shared = Arc::new((Mutex::new(Shared { state: "connecting".into(), want_key: true, skip_to_key: true, ..Default::default() }), Condvar::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (sh, st) = (shared.clone(), stop.clone());
        let thread = std::thread::Builder::new().name("srt".into()).spawn(move || run(u, sh, st))?;
        Ok(SrtOutput { name, shared, stop, thread: Some(thread), t0: None, mux: None, pending: vec![] })
    }

    /// Queue the TS just muxed as whole payloads (the tail too: a frame
    /// goes out as soon as it is muxed).
    fn queue(&mut self, s: &mut Shared, t: f64, video: bool) {
        for c in self.pending.chunks(srt::PAYLOAD) {
            s.queue.push_back(Chunk { t, video, data: c.to_vec() });
        }
        self.pending.clear();
    }
}

/// Secrets in a URL or key, hidden from logs and the UI.
fn redact(text: &str, u: &SrtUrl) -> String {
    let mut t = text.to_string();
    for secret in [u.passphrase.as_deref(), u.streamid.as_deref()].into_iter().flatten() {
        t = super::redact(&t, secret);
    }
    t
}

fn run(u: SrtUrl, shared: Arc<(Mutex<Shared>, Condvar)>, stop: Arc<AtomicBool>) {
    let (lock, cv) = &*shared;
    let mut backoff = 1u64;
    while !stop.load(Ordering::SeqCst) {
        lock.lock().unwrap().state = "connecting".into();
        let mut conn = match Conn::connect(&u) {
            Ok(c) => c,
            Err(e) => {
                let msg = redact(&format!("{e:#}"), &u);
                let mut s = lock.lock().unwrap();
                s.error = Some(msg.clone());
                s.state = format!("retrying in {backoff}s");
                s.reconnects += 1;
                drop(s);
                eprintln!("rec: stream srt://{}:{}: {msg}", u.host, u.port);
                super::rtmp::sleep_unless_stopped(&stop, Duration::from_secs(backoff));
                backoff = (backoff * 2).min(30);
                continue;
            }
        };
        {
            let mut s = lock.lock().unwrap();
            s.state = "live".into();
            s.error = None;
            s.live_since.get_or_insert_with(Instant::now);
            // A new connection starts at a keyframe (with the tables).
            s.queue.retain(|c| !c.video);
            s.skip_to_key = true;
            s.want_key = true;
        }
        let result: Result<()> = (|| loop {
            if stop.load(Ordering::SeqCst) {
                return Ok(());
            }
            let chunks: Vec<Chunk> = {
                let mut s = lock.lock().unwrap();
                if s.queue.is_empty() {
                    // Wake often enough to answer loss reports quickly.
                    s = cv.wait_timeout(s, Duration::from_millis(5)).unwrap().0;
                }
                s.queue.drain(..).collect()
            };
            for c in chunks {
                conn.send(&c.data)?;
            }
            conn.poll()?;
            let mut s = lock.lock().unwrap();
            let now = Instant::now();
            s.samples.push_back((now, conn.sent_bytes));
            while s.samples.front().is_some_and(|(t, _)| now - *t > Duration::from_secs(3)) {
                s.samples.pop_front();
            }
            s.dropped = s.dropped.max(conn.dropped);
            backoff = 1;
        })();
        if stop.load(Ordering::SeqCst) {
            conn.close();
            break;
        }
        if let Err(e) = result {
            let msg = redact(&format!("{e:#}"), &u);
            let mut s = lock.lock().unwrap();
            s.error = Some(msg.clone());
            s.state = "reconnecting".into();
            s.reconnects += 1;
            s.samples.clear();
            drop(s);
            eprintln!("rec: stream srt://{}:{}: {msg} (reconnecting)", u.host, u.port);
            super::rtmp::sleep_unless_stopped(&stop, Duration::from_secs(backoff));
            backoff = (backoff * 2).min(30);
        }
    }
    lock.lock().unwrap().state = "stopped".into();
}

impl Output for SrtOutput {
    fn packet(&mut self, p: &Packet, f: &Formats) {
        let shared = self.shared.clone();
        let (lock, cv) = &*shared;
        let mut s = lock.lock().unwrap();
        match p {
            Packet::Video(v) => {
                if f.param_sets.is_empty() || f.hevc != v.hevc {
                    return;
                }
                if self.t0.is_none() {
                    if !v.key {
                        return;
                    }
                    self.t0 = Some(v.pts);
                }
                if s.skip_to_key {
                    if !v.key {
                        s.dropped += 1;
                        return;
                    }
                    s.skip_to_key = false;
                }
                // Congestion: the queue spans too much time.
                if !v.key
                    && let Some(first) = s.queue.front().map(|c| c.t)
                    && v.pts - first > MAX_QUEUE_SECS
                {
                    s.queue.retain(|c| !c.video);
                    s.skip_to_key = true;
                    s.dropped += 1;
                    return;
                }
                // A codec switch (H.264 <-> HEVC) gets new tables.
                if self.mux.as_ref().is_none_or(|m| m.hevc() != v.hevc) {
                    if !v.key {
                        return;
                    }
                    self.mux = Some(Muxer::new(v.hevc));
                }
                let pts = PTS_BASE + ((v.pts - self.t0.unwrap()).max(0.0) * 90_000.0).round() as u64;
                let au = ts::annex_b(&v.data, v.hevc, v.key, &f.param_sets);
                self.mux.as_mut().unwrap().video(pts, v.key, &au, &mut self.pending);
                self.queue(&mut s, v.pts, true);
            }
            Packet::Audio(a) => {
                let (Some(t0), Some(mux)) = (self.t0, self.mux.as_mut()) else { return };
                if a.codec != AudioCodec::Aac || a.pts < t0 || s.queue.len() > 20_000 {
                    return;
                }
                let pts = PTS_BASE + ((a.pts - t0) * 90_000.0).round() as u64;
                // The shared encode is AAC-LC, 48 kHz (index 3), stereo.
                let mut adts = ts::adts(a.data.len(), 3, 2).to_vec();
                adts.extend_from_slice(&a.data);
                mux.audio(pts, &adts, &mut self.pending);
                self.queue(&mut s, a.pts, false);
            }
        }
        cv.notify_one();
    }

    fn take_keyframe_request(&mut self) -> bool {
        std::mem::take(&mut self.shared.0.lock().unwrap().want_key)
    }

    /// MPEG-TS carries HEVC; SRT receivers generally take it.
    fn accepts_hevc(&self) -> bool {
        true
    }

    fn status(&self) -> OutputStatus {
        let s = self.shared.0.lock().unwrap();
        let bitrate = match (s.samples.front(), s.samples.back()) {
            (Some(a), Some(b)) if s.state == "live" && b.0 > a.0 => b.1.saturating_sub(a.1) as f64 * 8.0 / (b.0 - a.0).as_secs_f64(),
            _ => 0.0,
        };
        OutputStatus {
            name: self.name.clone(),
            state: s.state.clone(),
            live: s.state == "live",
            bitrate,
            dropped_frames: s.dropped,
            reconnects: s.reconnects,
            duration: s.live_since.map_or(0.0, |t| t.elapsed().as_secs_f64()),
            error: s.error.clone(),
        }
    }

    fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.shared.1.notify_all();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for SrtOutput {
    fn drop(&mut self) {
        Output::stop(self);
    }
}
