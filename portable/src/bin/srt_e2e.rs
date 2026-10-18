//! End to end: real H.264 + AAC (made by ffmpeg) -> the TS muxer -> the
//! SRT caller -> a listener (FFmpeg's libsrt), or straight to a .ts file.
//! Run by `scripts/srt-e2e.sh`.
//!
//! Usage: srt_e2e MEDIA_DIR (OUT.ts | srt://...)
use rec_rs_portable::{srt, ts};
use std::time::{Duration, Instant};

fn nals(data: &[u8]) -> Vec<&[u8]> {
    let mut starts = vec![];
    let mut i = 0;
    while i + 3 <= data.len() {
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            starts.push(i + 3);
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut out = vec![];
    for (k, &s) in starts.iter().enumerate() {
        let mut e = if k + 1 < starts.len() { starts[k + 1] - 3 } else { data.len() };
        while e > s && data[e - 1] == 0 {
            e -= 1;
        }
        out.push(&data[s..e]);
    }
    out
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: srt_e2e MEDIA_DIR (OUT.ts | srt://...)");
        std::process::exit(64);
    }
    let dir = std::path::Path::new(&args[1]);
    let target = &args[2];
    let v = std::fs::read(dir.join("v.h264")).expect("v.h264 (Annex B with AUDs)");
    let a = std::fs::read(dir.join("a.aac")).expect("a.aac (ADTS)");
    // Access units, split at AUDs, as AVCC (what VideoToolbox gives).
    let mut aus: Vec<(bool, Vec<u8>)> = vec![];
    let mut params: Vec<Vec<u8>> = vec![];
    for n in nals(&v) {
        let t = n[0] & 0x1F;
        match t {
            9 => aus.push((false, vec![])),
            7 | 8 => {
                if params.len() < 2 { params.push(n.to_vec()); }
            }
            _ => {
                let au = aus.last_mut().unwrap();
                if t == 5 { au.0 = true; }
                au.1.extend_from_slice(&(n.len() as u32).to_be_bytes());
                au.1.extend_from_slice(n);
            }
        }
    }
    // Raw AAC frames from ADTS.
    let mut frames = vec![];
    let mut i = 0;
    while i + 7 <= a.len() {
        let len = (((a[i + 3] & 3) as usize) << 11) | ((a[i + 4] as usize) << 3) | ((a[i + 5] >> 5) as usize);
        frames.push(a[i + 7..i + len].to_vec());
        i += len;
    }
    eprintln!("{} video AUs ({} key), {} audio frames, {} param sets", aus.len(), aus.iter().filter(|x| x.0).count(), frames.len(), params.len());
    // Interleave by time.
    let mut events: Vec<(f64, bool, usize)> = (0..aus.len()).map(|k| (k as f64 / 30.0, true, k)).collect();
    events.extend((0..frames.len()).map(|k| (k as f64 * 1024.0 / 48000.0, false, k)));
    events.sort_by(|x, y| x.0.total_cmp(&y.0));
    let mut mux = ts::Muxer::new(false);
    let base = 126_000u64;
    let mut stream = vec![];
    let mut chunks: Vec<(f64, Vec<u8>)> = vec![];
    for (t, video, k) in events {
        let mut out = vec![];
        let pts = base + (t * 90_000.0).round() as u64;
        if video {
            let (key, d) = &aus[k];
            mux.video(pts, *key, &ts::annex_b(d, false, *key, &params), &mut out);
        } else {
            let mut adts = ts::adts(frames[k].len(), 3, 2).to_vec();
            adts.extend_from_slice(&frames[k]);
            mux.audio(pts, &adts, &mut out);
        }
        for c in out.chunks(srt::PAYLOAD) {
            chunks.push((t, c.to_vec()));
        }
        stream.extend(out);
    }
    if target.ends_with(".ts") {
        std::fs::write(target, &stream).unwrap();
        return;
    }
    let u = srt::parse_url(target, "").unwrap();
    let mut c = srt::Conn::connect(&u).unwrap_or_else(|e| { eprintln!("connect: {e:#}"); std::process::exit(2) });
    eprintln!("connected");
    let start = Instant::now();
    for (t, data) in chunks {
        while start.elapsed().as_secs_f64() < t {
            c.poll().unwrap();
            std::thread::sleep(Duration::from_millis(2));
        }
        c.send(&data).unwrap();
    }
    let end = Instant::now() + Duration::from_millis(1500);
    while Instant::now() < end {
        c.poll().unwrap();
        std::thread::sleep(Duration::from_millis(5));
    }
    eprintln!("sent {} bytes, {} resent, {} dropped, rtt {:.2} ms", c.sent_bytes, c.retransmits, c.dropped, c.rtt_ms);
    c.close();
}
