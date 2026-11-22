//! Mic monitoring: play the microphone, after its filters (noise
//! suppression, gate, ...), to the default output so you hear what is being
//! recorded. Use headphones: through speakers the mic hears itself.
//!
//! The writer pushes each filtered mic buffer here; an AudioQueue on its own
//! thread plays them. The queue is kept short (it drops audio that piles up)
//! so the delay stays small.

use std::collections::VecDeque;
use std::ffi::c_void;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use objc2_core_audio_types::AudioStreamBasicDescription;

type QueueRef = *mut c_void;

#[repr(C)]
struct QueueBuffer {
    capacity: u32,
    data: *mut c_void,
    size: u32,
    user: *mut c_void,
    desc_capacity: u32,
    descs: *mut c_void,
    desc_count: u32,
}

type OutputCallback = extern "C" fn(user: *mut c_void, q: QueueRef, buf: *mut QueueBuffer);

#[link(name = "AudioToolbox", kind = "framework")]
unsafe extern "C" {
    fn AudioQueueNewOutput(
        fmt: *const AudioStreamBasicDescription,
        cb: OutputCallback,
        user: *mut c_void,
        run_loop: *const c_void,
        mode: *const c_void,
        flags: u32,
        out: *mut QueueRef,
    ) -> i32;
    fn AudioQueueAllocateBuffer(q: QueueRef, bytes: u32, out: *mut *mut QueueBuffer) -> i32;
    fn AudioQueueEnqueueBuffer(q: QueueRef, buf: *mut QueueBuffer, n: u32, descs: *const c_void) -> i32;
    fn AudioQueueStart(q: QueueRef, at: *const c_void) -> i32;
    fn AudioQueueDispose(q: QueueRef, immediate: u8) -> i32;
}

/// Frames per queue buffer (~10 ms at 48 kHz).
const BUF_SECS: f64 = 0.01;
const BUFFERS: usize = 3;
/// More queued than this and the oldest is dropped, down to `KEEP_SECS`.
const MAX_SECS: f64 = 0.12;
const KEEP_SECS: f64 = 0.04;

static ON: AtomicBool = AtomicBool::new(false);
/// Linear gain as f32 bits.
static GAIN: AtomicU32 = AtomicU32::new(0x3f80_0000); // 1.0
static STATE: Mutex<Option<Player>> = Mutex::new(None);
static RING: Mutex<VecDeque<[f32; 2]>> = Mutex::new(VecDeque::new());

struct Player {
    q: SendQueue,
    rate: f64,
}

struct SendQueue(QueueRef);
// SAFETY: AudioQueue references may be used from any thread.
unsafe impl Send for SendQueue {}

impl Drop for Player {
    fn drop(&mut self) {
        unsafe { AudioQueueDispose(self.q.0, 1) };
    }
}

/// Turn monitoring on or off.
pub fn set_enabled(on: bool) {
    ON.store(on, Ordering::SeqCst);
    if !on {
        *STATE.lock().unwrap() = None;
        RING.lock().unwrap().clear();
    }
}

pub fn enabled() -> bool {
    ON.load(Ordering::Relaxed)
}

/// Monitor volume in dB (0 = as recorded).
pub fn set_gain_db(db: f32) {
    GAIN.store(10f32.powf(db / 20.0).to_bits(), Ordering::Relaxed);
}

/// Called with each (filtered) mic buffer; cheap when monitoring is off.
pub fn push(rate: f64, frames: &[[f32; 2]]) {
    if !enabled() || rate <= 0.0 {
        return;
    }
    {
        let mut st = STATE.lock().unwrap();
        if st.as_ref().is_none_or(|p| p.rate != rate) {
            *st = None;
            RING.lock().unwrap().clear();
            match Player::new(rate) {
                Some(p) => *st = Some(p),
                None => {
                    eprintln!("mic monitor: could not open the output at {rate} Hz");
                    ON.store(false, Ordering::SeqCst);
                    return;
                }
            }
        }
    }
    let mut ring = RING.lock().unwrap();
    ring.extend(frames.iter().copied());
    if ring.len() as f64 > rate * MAX_SECS {
        let keep = (rate * KEEP_SECS) as usize;
        let drop = ring.len() - keep;
        ring.drain(..drop);
    }
}

extern "C" fn fill(_user: *mut c_void, q: QueueRef, buf: *mut QueueBuffer) {
    unsafe {
        let b = &mut *buf;
        let n = b.capacity as usize / 8;
        let out = std::slice::from_raw_parts_mut(b.data as *mut [f32; 2], n);
        let g = f32::from_bits(GAIN.load(Ordering::Relaxed));
        let on = ON.load(Ordering::Relaxed);
        let mut ring = RING.lock().unwrap();
        for o in out.iter_mut() {
            *o = match ring.pop_front() {
                Some(f) if on => [(f[0] * g).clamp(-1.0, 1.0), (f[1] * g).clamp(-1.0, 1.0)],
                _ => [0.0, 0.0],
            };
        }
        drop(ring);
        b.size = (n * 8) as u32;
        AudioQueueEnqueueBuffer(q, buf, 0, std::ptr::null());
    }
}

impl Player {
    fn new(rate: f64) -> Option<Player> {
        let fmt = AudioStreamBasicDescription {
            mSampleRate: rate,
            mFormatID: u32::from_be_bytes(*b"lpcm"),
            mFormatFlags: 1 | 8, // float, packed
            mBytesPerPacket: 8,
            mFramesPerPacket: 1,
            mBytesPerFrame: 8,
            mChannelsPerFrame: 2,
            mBitsPerChannel: 32,
            mReserved: 0,
        };
        unsafe {
            let mut q: QueueRef = std::ptr::null_mut();
            if AudioQueueNewOutput(&fmt, fill, std::ptr::null_mut(), std::ptr::null(), std::ptr::null(), 0, &mut q) != 0 || q.is_null() {
                return None;
            }
            let p = Player { q: SendQueue(q), rate };
            let bytes = ((rate * BUF_SECS).ceil() as u32).max(64) * 8;
            for _ in 0..BUFFERS {
                let mut b: *mut QueueBuffer = std::ptr::null_mut();
                if AudioQueueAllocateBuffer(q, bytes, &mut b) != 0 || b.is_null() {
                    return None;
                }
                std::ptr::write_bytes((*b).data as *mut u8, 0, bytes as usize);
                (*b).size = bytes;
                AudioQueueEnqueueBuffer(q, b, 0, std::ptr::null());
            }
            if AudioQueueStart(q, std::ptr::null()) != 0 {
                return None;
            }
            Some(p)
        }
    }
}
