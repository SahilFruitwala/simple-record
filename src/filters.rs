//! Microphone filters, in the order they run: a low-cut (rumble, desk
//! knocks), noise suppression (RNNoise, via the pure-Rust nnnoiseless), a
//! noise gate, a three-band voice EQ, a de-esser and a compressor. They process the decoded mic audio before
//! it reaches the file, the mix, the meters and the live encode, so every
//! output hears the same voice. Plain Rust on f32 samples; no allocation
//! per buffer once running.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

/// Which filters run, and how.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MicFilters {
    /// High-pass at `low_cut_hz`.
    pub low_cut: bool,
    pub low_cut_hz: f32,
    /// RNNoise noise suppression (mics at 48 kHz or 16/24/12/8 kHz, as
    /// Bluetooth headsets use; others pass through).
    pub denoise: bool,
    pub gate: bool,
    /// The gate opens above this level (dBFS) and closes 6 dB below it.
    pub gate_db: f32,
    /// Voice EQ: low shelf at 150 Hz (warmth), peak at 3.5 kHz (presence)
    /// and high shelf at 10 kHz (air), each in dB.
    pub eq: bool,
    pub eq_low_db: f32,
    pub eq_mid_db: f32,
    pub eq_high_db: f32,
    /// De-esser: turns the 4-10 kHz band down when it is louder than
    /// `deess_db` (dBFS).
    pub deess: bool,
    pub deess_db: f32,
    pub compressor: bool,
    /// Compression starts here (dBFS).
    pub comp_threshold_db: f32,
    pub comp_ratio: f32,
    /// Gain after compression (dB).
    pub comp_makeup_db: f32,
    /// Silence the mic (the recording controls' mute); never saved.
    #[serde(skip)]
    pub mute: bool,
}

impl Default for MicFilters {
    fn default() -> Self {
        MicFilters {
            low_cut: false,
            low_cut_hz: 80.0,
            denoise: false,
            gate: false,
            gate_db: -45.0,
            eq: false,
            eq_low_db: 0.0,
            eq_mid_db: 0.0,
            eq_high_db: 0.0,
            deess: false,
            deess_db: -30.0,
            compressor: false,
            comp_threshold_db: -20.0,
            comp_ratio: 3.0,
            comp_makeup_db: 6.0,
            mute: false,
        }
    }
}

impl MicFilters {
    pub fn any(&self) -> bool {
        self.low_cut || self.denoise || self.gate || self.eq || self.deess || self.compressor || self.mute
    }
}

fn db_to_lin(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

fn lin_to_db(v: f32) -> f32 {
    20.0 * v.max(1e-9).log10()
}

/// One-pole smoothing coefficient for a time constant at a sample rate.
fn coef(secs: f32, rate: f32) -> f32 {
    (-1.0 / (secs * rate).max(1.0)).exp()
}

/// RBJ biquad (transposed direct form II), per channel.
#[derive(Clone, Copy, Default)]
struct Biquad {
    b: [f32; 3],
    a: [f32; 2],
    z: [[f32; 2]; 2],
}

impl Biquad {
    fn high_pass(freq: f32, rate: f32) -> Biquad {
        let w = 2.0 * std::f32::consts::PI * freq / rate;
        let (sin, cos) = w.sin_cos();
        let alpha = sin / (2.0 * std::f32::consts::FRAC_1_SQRT_2);
        let a0 = 1.0 + alpha;
        Biquad {
            b: [(1.0 + cos) / 2.0 / a0, -(1.0 + cos) / a0, (1.0 + cos) / 2.0 / a0],
            a: [-2.0 * cos / a0, (1.0 - alpha) / a0],
            z: [[0.0; 2]; 2],
        }
    }

    /// From normalized coefficients `b0 b1 b2 a0 a1 a2`.
    fn from(c: [f32; 6]) -> Biquad {
        let a0 = c[3];
        Biquad { b: [c[0] / a0, c[1] / a0, c[2] / a0], a: [c[4] / a0, c[5] / a0], z: [[0.0; 2]; 2] }
    }

    /// RBJ shelf (`high` or low) with slope 1.
    fn shelf(high: bool, freq: f32, db: f32, rate: f32) -> Biquad {
        let a = 10f32.powf(db / 40.0);
        let w = 2.0 * std::f32::consts::PI * freq / rate;
        let (sin, cos) = w.sin_cos();
        let alpha = sin / 2.0 * std::f32::consts::SQRT_2;
        let k = 2.0 * a.sqrt() * alpha;
        let s = if high { 1.0 } else { -1.0 };
        Biquad::from([
            a * ((a + 1.0) + s * (a - 1.0) * cos + k),
            -s * 2.0 * a * ((a - 1.0) + s * (a + 1.0) * cos),
            a * ((a + 1.0) + s * (a - 1.0) * cos - k),
            (a + 1.0) - s * (a - 1.0) * cos + k,
            s * 2.0 * ((a - 1.0) - s * (a + 1.0) * cos),
            (a + 1.0) - s * (a - 1.0) * cos - k,
        ])
    }

    /// RBJ peaking EQ.
    fn peak(freq: f32, q: f32, db: f32, rate: f32) -> Biquad {
        let a = 10f32.powf(db / 40.0);
        let w = 2.0 * std::f32::consts::PI * freq / rate;
        let (sin, cos) = w.sin_cos();
        let alpha = sin / (2.0 * q);
        Biquad::from([1.0 + alpha * a, -2.0 * cos, 1.0 - alpha * a, 1.0 + alpha / a, -2.0 * cos, 1.0 - alpha / a])
    }

    /// RBJ band-pass, 0 dB at the center.
    fn band_pass(freq: f32, q: f32, rate: f32) -> Biquad {
        let w = 2.0 * std::f32::consts::PI * freq / rate;
        let (sin, cos) = w.sin_cos();
        let alpha = sin / (2.0 * q);
        Biquad::from([alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * cos, 1.0 - alpha])
    }

    /// Same filter with new coefficients, keeping its state (no click
    /// when a slider moves).
    fn retune(&mut self, to: Biquad) {
        self.b = to.b;
        self.a = to.a;
    }

    fn run(&mut self, ch: usize, x: f32) -> f32 {
        let z = &mut self.z[ch];
        let y = self.b[0] * x + z[0];
        z[0] = self.b[1] * x - self.a[0] * y + z[1];
        z[1] = self.b[2] * x - self.a[1] * y;
        y
    }
}

/// Linear-phase low-pass FIR (Blackman-windowed sinc) for the resampling
/// around RNNoise, per channel.
struct Fir {
    taps: Vec<f32>,
    hist: Vec<[f32; 2]>,
    pos: usize,
}

/// Odd, so the delay is a whole number of samples; `TAPS - 1` divides by
/// every supported ratio (2, 3, 4, 6).
const TAPS: usize = 97;

impl Fir {
    /// Cutoff as a fraction of the sample rate, `gain` at DC.
    fn low_pass(cutoff: f32, gain: f32) -> Fir {
        let m = (TAPS - 1) as f32;
        let mut taps: Vec<f32> = (0..TAPS)
            .map(|i| {
                let x = i as f32 - m / 2.0;
                let sinc = if x == 0.0 { 2.0 * cutoff } else { (2.0 * std::f32::consts::PI * cutoff * x).sin() / (std::f32::consts::PI * x) };
                let t = 2.0 * std::f32::consts::PI * i as f32 / m;
                sinc * (0.42 - 0.5 * t.cos() + 0.08 * (2.0 * t).cos())
            })
            .collect();
        let sum: f32 = taps.iter().sum();
        taps.iter_mut().for_each(|t| *t *= gain / sum);
        Fir { taps, hist: vec![[0.0; 2]; TAPS], pos: 0 }
    }

    fn push(&mut self, x: [f32; 2]) {
        self.pos = (self.pos + 1) % TAPS;
        self.hist[self.pos] = x;
    }

    fn out(&self) -> [f32; 2] {
        let mut y = [0.0f32; 2];
        for (k, t) in self.taps.iter().enumerate() {
            let h = self.hist[(self.pos + TAPS - k) % TAPS];
            y[0] += t * h[0];
            y[1] += t * h[1];
        }
        y
    }
}

/// RNNoise works on 480-sample frames of 48 kHz audio. Mics at 48 kHz / L
/// (Bluetooth headsets like AirPods run at 16 or 24 kHz) are upsampled by L
/// for it and filtered back down. Samples go in and come out one buffer at
/// a time, `delay()` samples late.
struct Denoiser {
    /// Upsampling ratio to 48 kHz (1 = native).
    l: usize,
    up: Fir,
    down: Fir,
    states: [Box<nnnoiseless::DenoiseState<'static>>; 2],
    input: VecDeque<[f32; 2]>,
    output: VecDeque<[f32; 2]>,
    frame_in: [Vec<f32>; 2],
    frame_out: [Vec<f32>; 2],
}

const FRAME: usize = nnnoiseless::DenoiseState::FRAME_SIZE;

impl Denoiser {
    /// The ratio for a mic rate, or None if RNNoise can't run on it.
    fn ratio(rate: f64) -> Option<usize> {
        [1, 2, 3, 4, 6].into_iter().find(|l| (rate * *l as f64 - 48000.0).abs() < 1.0)
    }

    /// Total delay in mic samples: the output queue is primed with one
    /// frame (so a buffer of any length always has output ready), RNNoise's
    /// overlap-add adds another, and the two resampling filters theirs.
    fn delay(l: usize) -> usize {
        2 * FRAME / l + if l > 1 { (TAPS - 1) / l } else { 0 }
    }

    fn new(l: usize) -> Denoiser {
        // Just under the mic's Nyquist frequency, at 48 kHz.
        let cutoff = 0.45 / l as f32;
        let n = FRAME / l;
        Denoiser {
            l,
            up: Fir::low_pass(cutoff, l as f32),
            down: Fir::low_pass(cutoff, 1.0),
            states: [nnnoiseless::DenoiseState::new(), nnnoiseless::DenoiseState::new()],
            input: VecDeque::with_capacity(4 * n),
            output: std::iter::repeat_n([0.0; 2], n).collect(),
            frame_in: [vec![0.0; FRAME], vec![0.0; FRAME]],
            frame_out: [vec![0.0; FRAME], vec![0.0; FRAME]],
        }
    }

    fn process(&mut self, frames: &mut [[f32; 2]]) {
        let (l, n) = (self.l, FRAME / self.l);
        self.input.extend(frames.iter().copied());
        while self.input.len() >= n {
            let mono = self.input.iter().take(n).all(|f| f[0] == f[1]);
            for (i, f) in self.input.drain(..n).enumerate() {
                for j in 0..l {
                    // Zero-stuffed and low-passed up to 48 kHz.
                    let x = if l == 1 {
                        f
                    } else {
                        self.up.push(if j == 0 { f } else { [0.0; 2] });
                        self.up.out()
                    };
                    // RNNoise expects 16-bit sample values.
                    self.frame_in[0][i * l + j] = x[0] * 32768.0;
                    self.frame_in[1][i * l + j] = x[1] * 32768.0;
                }
            }
            let chans = if mono { 1 } else { 2 };
            for c in 0..chans {
                let (inp, out) = (&self.frame_in[c], &mut self.frame_out[c]);
                self.states[c].process_frame(out, inp);
            }
            for i in 0..FRAME {
                let left = self.frame_out[0][i] / 32768.0;
                let y = [left, if mono { left } else { self.frame_out[1][i] / 32768.0 }];
                if l == 1 {
                    self.output.push_back(y);
                } else {
                    // Low-passed and decimated back to the mic's rate.
                    self.down.push(y);
                    if i % l == l - 1 {
                        self.output.push_back(self.down.out());
                    }
                }
            }
        }
        for f in frames.iter_mut() {
            *f = self.output.pop_front().unwrap_or([0.0; 2]);
        }
    }
}

/// Opens above the threshold, closes 6 dB below it after a hold; the gain
/// ramps (no clicks).
struct Gate {
    env: f32,
    gain: f32,
    open: bool,
    held: usize,
}

/// Voice EQ bands and the settings they were built for.
struct Eq {
    bands: [Biquad; 3],
    built: [f32; 3],
}

/// Split-band de-esser: the sibilance band is measured and only that band
/// is turned down, so the rest of the voice keeps its level.
struct DeEsser {
    band: Biquad,
    env: f32,
    gain: f32,
}

/// Compressor: peak detector in dB with attack and release, soft knee.
struct Compressor {
    env_db: f32,
}

pub struct FilterChain {
    cfg: MicFilters,
    rate: f64,
    hp: Option<(Biquad, f32)>,
    denoise: Option<Denoiser>,
    gate: Gate,
    eq: Option<Eq>,
    deess: Option<DeEsser>,
    comp: Compressor,
}

impl FilterChain {
    pub fn new(cfg: MicFilters) -> FilterChain {
        FilterChain {
            cfg,
            rate: 0.0,
            hp: None,
            denoise: None,
            gate: Gate { env: 0.0, gain: 0.0, open: false, held: 0 },
            eq: None,
            deess: None,
            comp: Compressor { env_db: -120.0 },
        }
    }

    /// Change the settings live; filters turned on start fresh.
    pub fn set(&mut self, cfg: MicFilters) {
        if !cfg.denoise {
            self.denoise = None;
        }
        if !cfg.low_cut {
            self.hp = None;
        }
        if !cfg.eq {
            self.eq = None;
        }
        if !cfg.deess {
            self.deess = None;
        }
        self.cfg = cfg;
    }

    pub fn active(&self) -> bool {
        self.cfg.any()
    }

    /// How late the output is, in seconds (noise suppression buffers).
    pub fn latency(&self) -> f64 {
        self.denoise.as_ref().map_or(0.0, |d| Denoiser::delay(d.l) as f64 / self.rate)
    }

    /// Filter `frames` (stereo, `rate` Hz) in place.
    pub fn process(&mut self, rate: f64, frames: &mut [[f32; 2]]) {
        if rate != self.rate {
            // A new device rate: filters restart at it.
            self.rate = rate;
            self.hp = None;
            self.denoise = None;
            self.eq = None;
            self.deess = None;
        }
        let r = rate as f32;
        if self.cfg.low_cut {
            let hz = self.cfg.low_cut_hz.clamp(40.0, 300.0);
            let (hp, built) = self.hp.get_or_insert_with(|| (Biquad::high_pass(hz, r), hz));
            if *built != hz {
                hp.retune(Biquad::high_pass(hz, r));
                *built = hz;
            }
            for f in frames.iter_mut() {
                f[0] = hp.run(0, f[0]);
                f[1] = hp.run(1, f[1]);
            }
        }
        // RNNoise is trained on 48 kHz; mics at 48 kHz / L are resampled
        // for it, others skip it.
        if self.cfg.denoise && let Some(l) = Denoiser::ratio(rate) {
            self.denoise.get_or_insert_with(|| Denoiser::new(l)).process(frames);
        }
        if self.cfg.gate {
            let g = &mut self.gate;
            let open_at = db_to_lin(self.cfg.gate_db);
            let close_at = db_to_lin(self.cfg.gate_db - 6.0);
            let (env_a, env_r) = (coef(0.001, r), coef(0.05, r));
            let (gain_open, gain_close) = (coef(0.002, r), coef(0.08, r));
            let hold = (0.2 * r) as usize;
            for f in frames.iter_mut() {
                let x = f[0].abs().max(f[1].abs());
                let k = if x > g.env { env_a } else { env_r };
                g.env = x + k * (g.env - x);
                if g.env >= open_at {
                    g.open = true;
                    g.held = 0;
                } else if g.env < close_at && g.open {
                    g.held += 1;
                    if g.held > hold {
                        g.open = false;
                    }
                }
                let target = if g.open { 1.0 } else { 0.0 };
                let k = if target > g.gain { gain_open } else { gain_close };
                g.gain = target + k * (g.gain - target);
                f[0] *= g.gain;
                f[1] *= g.gain;
            }
        }
        if self.cfg.eq {
            let want = [self.cfg.eq_low_db, self.cfg.eq_mid_db, self.cfg.eq_high_db].map(|d| d.clamp(-12.0, 12.0));
            let make = |d: [f32; 3]| [Biquad::shelf(false, 150.0, d[0], r), Biquad::peak(3500.0, 0.9, d[1], r), Biquad::shelf(true, 10_000f32.min(0.4 * r), d[2], r)];
            let eq = self.eq.get_or_insert_with(|| Eq { bands: make(want), built: want });
            if eq.built != want {
                for (b, to) in eq.bands.iter_mut().zip(make(want)) {
                    b.retune(to);
                }
                eq.built = want;
            }
            for f in frames.iter_mut() {
                for b in eq.bands.iter_mut() {
                    f[0] = b.run(0, f[0]);
                    f[1] = b.run(1, f[1]);
                }
            }
        }
        if self.cfg.deess {
            let d = self.deess.get_or_insert_with(|| DeEsser { band: Biquad::band_pass(6500f32.min(0.4 * r), 0.8, r), env: 0.0, gain: 1.0 });
            let threshold = db_to_lin(self.cfg.deess_db);
            let (env_a, env_r) = (coef(0.0005, r), coef(0.04, r));
            let (gain_a, gain_r) = (coef(0.001, r), coef(0.06, r));
            for f in frames.iter_mut() {
                let s = [d.band.run(0, f[0]), d.band.run(1, f[1])];
                let x = s[0].abs().max(s[1].abs());
                let k = if x > d.env { env_a } else { env_r };
                d.env = x + k * (d.env - x);
                // Down to the threshold, at most 12 dB.
                let target = if d.env > threshold { (threshold / d.env).max(0.25) } else { 1.0 };
                let k = if target < d.gain { gain_a } else { gain_r };
                d.gain = target + k * (d.gain - target);
                let cut = 1.0 - d.gain;
                f[0] -= s[0] * cut;
                f[1] -= s[1] * cut;
            }
        }
        if self.cfg.compressor {
            let c = &mut self.comp;
            let (t, ratio, knee) = (self.cfg.comp_threshold_db, self.cfg.comp_ratio.max(1.0), 6.0f32);
            let makeup = db_to_lin(self.cfg.comp_makeup_db);
            let (att, rel) = (coef(0.005, r), coef(0.12, r));
            for f in frames.iter_mut() {
                let x = lin_to_db(f[0].abs().max(f[1].abs()));
                let k = if x > c.env_db { att } else { rel };
                c.env_db = x + k * (c.env_db - x);
                // Gain reduction with a soft knee around the threshold.
                let over = c.env_db - t;
                let reduce = if over <= -knee / 2.0 {
                    0.0
                } else if over >= knee / 2.0 {
                    over * (1.0 - 1.0 / ratio)
                } else {
                    (1.0 - 1.0 / ratio) * (over + knee / 2.0).powi(2) / (2.0 * knee)
                };
                let g = db_to_lin(-reduce) * makeup;
                f[0] *= g;
                f[1] *= g;
            }
        }
        // Muted after the filters, so they stay settled for unmuting.
        if self.cfg.mute {
            frames.fill([0.0; 2]);
        }
    }
}

/// `frames` as raw linear PCM: `channels` channels (left in the first,
/// right in the second, left again in any others), float32, int16 or
/// int32, little-endian, interleaved or one plane per channel. None for
/// other sample formats.
pub fn encode_pcm(frames: &[[f32; 2]], channels: usize, float: bool, bits: u32, planar: bool) -> Option<Vec<u8>> {
    let bytes = match (float, bits) {
        (true, 32) | (false, 32) => 4,
        (false, 16) => 2,
        _ => return None,
    };
    let (n, ch) = (frames.len(), channels.max(1));
    let mut out = vec![0u8; n * ch * bytes];
    for (i, f) in frames.iter().enumerate() {
        for c in 0..ch {
            let v = if c == 1 { f[1] } else { f[0] };
            let at = if planar { c * n + i } else { i * ch + c } * bytes;
            let dst = &mut out[at..at + bytes];
            match (float, bits) {
                (true, _) => dst.copy_from_slice(&v.to_le_bytes()),
                (false, 16) => dst.copy_from_slice(&((v.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes()),
                _ => dst.copy_from_slice(&((v.clamp(-1.0, 1.0) as f64 * 2147483647.0).round() as i32).to_le_bytes()),
            }
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcm_layouts() {
        let f = [[0.5f32, -0.5], [1.0, 0.0]];
        let inter = encode_pcm(&f, 2, false, 16, false).unwrap();
        let v: Vec<i16> = inter.chunks(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
        assert_eq!(v, [16384, -16384, 32767, 0]);
        let planar = encode_pcm(&f, 2, true, 32, true).unwrap();
        let v: Vec<f32> = planar.chunks(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
        assert_eq!(v, [0.5, 1.0, -0.5, 0.0]);
        let mono = encode_pcm(&f, 1, true, 32, false).unwrap();
        assert_eq!(mono.len(), 8);
        assert!(encode_pcm(&f, 2, false, 24, false).is_none());
    }

    fn sine(freq: f32, amp: f32, n: usize, rate: f32) -> Vec<[f32; 2]> {
        (0..n).map(|i| {
            let v = amp * (2.0 * std::f32::consts::PI * freq * i as f32 / rate).sin();
            [v, v]
        }).collect()
    }

    fn rms(x: &[[f32; 2]]) -> f32 {
        (x.iter().map(|f| f[0] * f[0]).sum::<f32>() / x.len() as f32).sqrt()
    }

    fn run(cfg: MicFilters, input: &[[f32; 2]], chunk: usize) -> Vec<[f32; 2]> {
        let mut c = FilterChain::new(cfg);
        let mut out = vec![];
        for part in input.chunks(chunk) {
            let mut p = part.to_vec();
            c.process(48000.0, &mut p);
            out.extend(p);
        }
        out
    }

    #[test]
    fn eq_bands_boost_and_cut() {
        let (n, rate) = (24000, 48000.0);
        let tail = |v: &[[f32; 2]]| rms(&v[n / 2..]);
        let cfg = MicFilters { eq: true, eq_low_db: 6.0, eq_mid_db: -6.0, eq_high_db: 0.0, ..Default::default() };
        // A shelf reaches its gain well below its corner; the peak at its center.
        let low = sine(50.0, 0.1, n, rate);
        let db = 20.0 * (tail(&run(cfg, &low, 512)) / tail(&low)).log10();
        assert!((db - 6.0).abs() < 0.5, "low shelf {db}");
        let mid = sine(3500.0, 0.1, n, rate);
        let db = 20.0 * (tail(&run(cfg, &mid, 512)) / tail(&mid)).log10();
        assert!((db + 6.0).abs() < 0.3, "presence {db}");
        // Flat EQ is transparent.
        let flat = MicFilters { eq: true, ..Default::default() };
        let x = sine(1000.0, 0.3, n, rate);
        assert!((tail(&run(flat, &x, 512)) / tail(&x) - 1.0).abs() < 1e-3);
    }

    #[test]
    fn deesser_cuts_sibilance_only() {
        let (n, rate) = (24000, 48000.0);
        let cfg = MicFilters { deess: true, deess_db: -30.0, ..Default::default() };
        let tail = |v: &[[f32; 2]]| rms(&v[n / 2..]);
        let ess = sine(6500.0, 0.3, n, rate);
        assert!(tail(&run(cfg, &ess, 512)) < 0.5 * tail(&ess));
        let voice = sine(300.0, 0.3, n, rate);
        assert!((tail(&run(cfg, &voice, 512)) / tail(&voice) - 1.0).abs() < 0.02);
        // Quiet sibilance stays.
        let soft = sine(6500.0, 0.01, n, rate);
        assert!((tail(&run(cfg, &soft, 512)) / tail(&soft) - 1.0).abs() < 0.02);
    }

    #[test]
    fn low_cut_frequency_moves() {
        let x = sine(120.0, 0.3, 24000, 48000.0);
        let at = |hz| rms(&run(MicFilters { low_cut: true, low_cut_hz: hz, ..Default::default() }, &x, 512)[12000..]);
        assert!(at(60.0) > 0.18);
        assert!(at(200.0) < 0.12);
    }

    #[test]
    fn old_settings_load() {
        let f: MicFilters = serde_json::from_str(r#"{"low_cut":true,"compressor":true}"#).unwrap();
        assert!(f.low_cut && f.compressor && !f.eq && !f.deess);
        assert_eq!(f.low_cut_hz, 80.0);
    }

    #[test]
    fn off_is_untouched() {
        let x = sine(440.0, 0.3, 4800, 48000.0);
        assert_eq!(run(MicFilters::default(), &x, 1024), x);
        assert!(!MicFilters::default().any());
    }

    #[test]
    fn mute_silences_and_unmutes() {
        let x = sine(440.0, 0.3, 4800, 48000.0);
        let muted = MicFilters { mute: true, ..Default::default() };
        assert!(muted.any());
        assert!(run(muted, &x, 1024).iter().all(|f| *f == [0.0; 2]));
        let mut c = FilterChain::new(muted);
        c.set(MicFilters::default());
        let mut p = x.clone();
        c.process(48000.0, &mut p);
        assert_eq!(p, x);
        // Not saved: a settings file never comes back muted.
        let json = serde_json::to_string(&muted).unwrap();
        assert!(!json.contains("mute"));
    }

    #[test]
    fn low_cut_removes_rumble_keeps_voice() {
        let cfg = MicFilters { low_cut: true, ..Default::default() };
        let rumble = run(cfg, &sine(30.0, 0.5, 48000, 48000.0), 512);
        let voice = run(cfg, &sine(300.0, 0.5, 48000, 48000.0), 512);
        let cut = rms(&rumble[24000..]) / rms(&sine(30.0, 0.5, 24000, 48000.0));
        assert!(cut < 0.2, "30 Hz cut to {cut}");
        let ratio = rms(&voice[24000..]) / rms(&sine(300.0, 0.5, 24000, 48000.0));
        assert!(ratio > 0.95, "300 Hz kept: {ratio}");
    }

    #[test]
    fn gate_silences_quiet_and_passes_loud() {
        let cfg = MicFilters { gate: true, gate_db: -40.0, ..Default::default() };
        let quiet = run(cfg, &sine(300.0, 0.002, 48000, 48000.0), 480);
        assert!(rms(&quiet[24000..]) < 1e-5, "{}", rms(&quiet[24000..]));
        let loud = run(cfg, &sine(300.0, 0.2, 48000, 48000.0), 480);
        assert!((rms(&loud[24000..]) / rms(&sine(300.0, 0.2, 24000, 48000.0)) - 1.0).abs() < 0.02);
    }

    #[test]
    fn compressor_reduces_loud_more_than_quiet() {
        let cfg = MicFilters { compressor: true, comp_threshold_db: -20.0, comp_ratio: 4.0, comp_makeup_db: 0.0, ..Default::default() };
        let loud_in = sine(300.0, 0.9, 48000, 48000.0);
        let quiet_in = sine(300.0, 0.03, 48000, 48000.0);
        let loud = rms(&run(cfg, &loud_in, 512)[24000..]) / rms(&loud_in[24000..]);
        let quiet = rms(&run(cfg, &quiet_in, 512)[24000..]) / rms(&quiet_in[24000..]);
        assert!(loud < 0.4, "loud attenuated: {loud}");
        assert!(quiet > 0.95, "below threshold untouched: {quiet}");
    }

    /// The output lags the input by exactly the reported latency, with any
    /// buffer size (the file's mic timestamps are corrected by it).
    #[test]
    fn denoise_delay_is_exact() {
        let cfg = MicFilters { denoise: true, ..Default::default() };
        // A voice-like tone burst after silence.
        let mut x = vec![[0.0f32; 2]; 48000];
        for (i, f) in x.iter_mut().enumerate().skip(12000).take(9600) {
            let v = 0.3 * (2.0 * std::f32::consts::PI * 220.0 * i as f32 / 48000.0).sin() * (1.0 + 0.5 * (2.0 * std::f32::consts::PI * 3.0 * i as f32 / 48000.0).sin());
            *f = [v, v];
        }
        for chunk in [441, 480, 1024, 960] {
            let y = run(cfg, &x, chunk);
            assert_eq!(y.len(), x.len());
            // Cross-correlate to find the lag.
            let best = (0..3000)
                .map(|lag| (lag, (12000..21600).map(|i| x[i][0] * y.get(i + lag).map_or(0.0, |f| f[0])).sum::<f32>()))
                .max_by(|a, b| a.1.total_cmp(&b.1))
                .unwrap()
                .0;
            // Within a quarter millisecond (RNNoise's pitch filter shifts
            // a tone's phase slightly).
            assert!(best.abs_diff(Denoiser::delay(1)) <= 12, "chunk {chunk}: lag {best}");
        }
        let mut c = FilterChain::new(cfg);
        c.process(48000.0, &mut [[0.0; 2]; 16]);
        assert!((c.latency() - Denoiser::delay(1) as f64 / 48000.0).abs() < 1e-12);
    }

    #[test]
    fn denoise_reduces_steady_noise() {
        let cfg = MicFilters { denoise: true, ..Default::default() };
        // Pseudo-random white noise.
        let mut seed = 1u32;
        let noise: Vec<[f32; 2]> = (0..96000)
            .map(|_| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                let v = ((seed >> 8) as f32 / (1 << 24) as f32 - 0.5) * 0.05;
                [v, v]
            })
            .collect();
        let y = run(cfg, &noise, 1024);
        let ratio = rms(&y[48000..]) / rms(&noise[48000..]);
        assert!(ratio < 0.7, "noise reduced to {ratio}");
    }

    /// Bluetooth headset rates (AirPods: 16 / 24 kHz) are denoised too,
    /// with the reported delay, and a voice-band tone comes through.
    #[test]
    fn bluetooth_rates_denoise() {
        for rate in [16000.0f32, 24000.0] {
            let cfg = MicFilters { denoise: true, ..Default::default() };
            let mut seed = 7u32;
            let noise: Vec<[f32; 2]> = (0..rate as usize * 2)
                .map(|_| {
                    seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                    let v = ((seed >> 8) as f32 / (1 << 24) as f32 - 0.5) * 0.05;
                    [v, v]
                })
                .collect();
            let mut c = FilterChain::new(cfg);
            let mut y = vec![];
            for part in noise.chunks(333) {
                let mut p = part.to_vec();
                c.process(rate as f64, &mut p);
                y.extend(p);
            }
            let half = noise.len() / 2;
            let ratio = rms(&y[half..]) / rms(&noise[half..]);
            assert!(ratio < 0.7, "{rate}: noise reduced to {ratio}");
            let l = (48000.0 / rate) as usize;
            assert!((c.latency() - Denoiser::delay(l) as f64 / rate as f64).abs() < 1e-12);

            // Timing: a voice-like burst comes out `delay` samples late.
            let n = rate as usize;
            let mut x = vec![[0.0f32; 2]; n];
            let (s0, len) = (n / 4, n / 5);
            for (i, f) in x.iter_mut().enumerate().skip(s0).take(len) {
                let t = i as f32 / rate;
                let v = 0.3 * (2.0 * std::f32::consts::PI * 220.0 * t).sin() * (1.0 + 0.5 * (2.0 * std::f32::consts::PI * 3.0 * t).sin());
                *f = [v, v];
            }
            let mut c = FilterChain::new(cfg);
            let mut y = vec![];
            for part in x.chunks(256) {
                let mut p = part.to_vec();
                c.process(rate as f64, &mut p);
                y.extend(p);
            }
            let best = (0..1000)
                .map(|lag| (lag, (s0..s0 + len).map(|i| x[i][0] * y.get(i + lag).map_or(0.0, |f| f[0])).sum::<f32>()))
                .max_by(|a, b| a.1.total_cmp(&b.1))
                .unwrap()
                .0;
            assert!(best.abs_diff(Denoiser::delay(l)) <= 4, "{rate}: lag {best} vs {}", Denoiser::delay(l));
        }
    }

    #[test]
    fn unsupported_rate_skips_denoise() {
        let mut c = FilterChain::new(MicFilters { denoise: true, ..Default::default() });
        let mut x = sine(300.0, 0.3, 1000, 44100.0);
        let before = x.clone();
        c.process(44100.0, &mut x);
        assert_eq!(x, before);
        assert_eq!(c.latency(), 0.0);
    }
}
