//! MPEG transport stream muxing (ISO/IEC 13818-1) for SRT: H.264 or HEVC
//! video and AAC audio, with the tables (PAT, PMT) ahead of every keyframe
//! and the clock reference (PCR) on the video. Plain Rust, no allocation
//! beyond the output.

/// TS packet size.
pub const PACKET: usize = 188;
const PID_PAT: u16 = 0;
const PID_PMT: u16 = 0x1000;
const PID_VIDEO: u16 = 0x100;
const PID_AUDIO: u16 = 0x101;
const STREAM_H264: u8 = 0x1B;
const STREAM_HEVC: u8 = 0x24;
const STREAM_AAC: u8 = 0x0F;

/// Presentation times run this far (90 kHz ticks, 0.7 s) ahead of the
/// clock reference: the receiver's decoder buffer.
pub const PCR_LEAD: u64 = 63_000;

/// CRC-32/MPEG-2 of PSI sections.
fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= (b as u32) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 { (crc << 1) ^ 0x04C1_1DB7 } else { crc << 1 };
        }
    }
    crc
}

/// A 33-bit timestamp in the 5-byte PES form, with a 4-bit prefix.
fn pes_ts(prefix: u8, t: u64) -> [u8; 5] {
    let t = t & 0x1_FFFF_FFFF;
    [
        (prefix << 4) | (((t >> 29) as u8) & 0x0E) | 1,
        (t >> 22) as u8,
        (((t >> 14) as u8) & 0xFE) | 1,
        (t >> 7) as u8,
        (((t << 1) as u8) & 0xFE) | 1,
    ]
}

/// An ADTS header for a raw AAC-LC frame of `len` bytes (48 kHz stereo
/// unless told otherwise).
pub fn adts(len: usize, freq_index: u8, channels: u8) -> [u8; 7] {
    let n = len + 7;
    let profile = 1u8; // AAC LC (object type 2) - 1
    [
        0xFF,
        0xF1, // MPEG-4, layer 0, no CRC
        (profile << 6) | (freq_index << 2) | ((channels >> 2) & 1),
        ((channels & 3) << 6) | ((n >> 11) as u8 & 3),
        (n >> 3) as u8,
        (((n & 7) as u8) << 5) | 0x1F,
        0xFC,
    ]
}

/// Length-prefixed (AVCC/HVCC, 4-byte lengths) NAL units as Annex B, with
/// an access unit delimiter first and, on keyframes, the parameter sets.
pub fn annex_b(data: &[u8], hevc: bool, key: bool, param_sets: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 64);
    let start = [0u8, 0, 0, 1];
    out.extend_from_slice(&start);
    if hevc {
        out.extend_from_slice(&[0x46, 0x01, 0x50]);
    } else {
        out.extend_from_slice(&[0x09, 0xF0]);
    }
    if key {
        for p in param_sets {
            out.extend_from_slice(&start);
            out.extend_from_slice(p);
        }
    }
    let mut i = 0;
    while i + 4 <= data.len() {
        let n = u32::from_be_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]) as usize;
        i += 4;
        let end = (i + n).min(data.len());
        out.extend_from_slice(&start);
        out.extend_from_slice(&data[i..end]);
        i = end;
    }
    out
}

pub struct Muxer {
    hevc: bool,
    /// Continuity counters by PID: PAT, PMT, video, audio.
    cc: [u8; 4],
}

impl Muxer {
    pub fn new(hevc: bool) -> Muxer {
        Muxer { hevc, cc: [0; 4] }
    }

    pub fn hevc(&self) -> bool {
        self.hevc
    }

    fn next_cc(&mut self, pid: u16) -> u8 {
        let i = match pid {
            PID_PAT => 0,
            PID_PMT => 1,
            PID_VIDEO => 2,
            _ => 3,
        };
        let c = self.cc[i];
        self.cc[i] = (c + 1) & 0x0F;
        c
    }

    /// One PSI section in one packet.
    fn section(&mut self, pid: u16, section: &[u8], out: &mut Vec<u8>) {
        let mut p = [0xFFu8; PACKET];
        p[0] = 0x47;
        p[1] = 0x40 | (pid >> 8) as u8;
        p[2] = pid as u8;
        p[3] = 0x10 | self.next_cc(pid);
        p[4] = 0; // pointer field
        p[5..5 + section.len()].copy_from_slice(section);
        let crc = crc32(section);
        p[5 + section.len()..9 + section.len()].copy_from_slice(&crc.to_be_bytes());
        out.extend_from_slice(&p);
    }

    /// The program tables.
    pub fn tables(&mut self, out: &mut Vec<u8>) {
        // PAT: program 1 -> PMT.
        let pat = [0x00, 0xB0, 13, 0x00, 0x01, 0xC1, 0x00, 0x00, 0x00, 0x01, 0xE0 | (PID_PMT >> 8) as u8, PID_PMT as u8];
        self.section(PID_PAT, &pat, out);
        let video_type = if self.hevc { STREAM_HEVC } else { STREAM_H264 };
        let len = 9 + 5 * 2 + 4;
        let pmt = [
            0x02,
            0xB0 | (len >> 8) as u8,
            len as u8,
            0x00,
            0x01,
            0xC1,
            0x00,
            0x00,
            0xE0 | (PID_VIDEO >> 8) as u8,
            PID_VIDEO as u8,
            0xF0,
            0x00,
            video_type,
            0xE0 | (PID_VIDEO >> 8) as u8,
            PID_VIDEO as u8,
            0xF0,
            0x00,
            STREAM_AAC,
            0xE0 | (PID_AUDIO >> 8) as u8,
            PID_AUDIO as u8,
            0xF0,
            0x00,
        ];
        self.section(PID_PMT, &pmt, out);
    }

    /// A PES packet split into TS packets. The first carries the PCR and
    /// the random access flag when given.
    fn pes(&mut self, pid: u16, stream_id: u8, pts: u64, pcr: Option<u64>, random_access: bool, payload: &[u8], out: &mut Vec<u8>) {
        let mut pes = Vec::with_capacity(payload.len() + 14);
        pes.extend_from_slice(&[0, 0, 1, stream_id]);
        let len = 3 + 5 + payload.len();
        // Video PES may leave the length open (0); audio fits.
        let pes_len = if stream_id & 0xF0 == 0xE0 || len > 0xFFFF { 0 } else { len };
        pes.extend_from_slice(&(pes_len as u16).to_be_bytes());
        pes.extend_from_slice(&[0x84, 0x80, 5]); // data aligned; PTS only
        pes.extend_from_slice(&pes_ts(0b0010, pts));
        pes.extend_from_slice(payload);
        let mut pos = 0;
        let mut first = true;
        while pos < pes.len() {
            let mut af: Vec<u8> = vec![];
            if first && (pcr.is_some() || random_access) {
                af.push(if random_access { 0x40 } else { 0 } | if pcr.is_some() { 0x10 } else { 0 });
                if let Some(p) = pcr {
                    let b = p & 0x1_FFFF_FFFF;
                    af.extend_from_slice(&[(b >> 25) as u8, (b >> 17) as u8, (b >> 9) as u8, (b >> 1) as u8, (((b & 1) as u8) << 7) | 0x7E, 0]);
                }
            }
            let remaining = pes.len() - pos;
            // Adaptation field size including its length byte.
            let mut af_total = if af.is_empty() { 0 } else { 1 + af.len() };
            if remaining < 184 - af_total {
                // The last packet: stuff the adaptation field to fill it.
                af_total = 184 - remaining;
                if af.is_empty() && af_total >= 2 {
                    af.push(0);
                }
                while 1 + af.len() < af_total {
                    af.push(0xFF);
                }
            }
            let n = remaining.min(184 - af_total);
            let mut p = [0u8; PACKET];
            p[0] = 0x47;
            p[1] = if first { 0x40 } else { 0 } | (pid >> 8) as u8 & 0x1F;
            p[2] = pid as u8;
            p[3] = if af_total > 0 { 0x30 } else { 0x10 } | self.next_cc(pid);
            let mut i = 4;
            if af_total > 0 {
                p[4] = (af_total - 1) as u8;
                p[5..5 + af.len()].copy_from_slice(&af);
                i = 4 + af_total;
            }
            p[i..i + n].copy_from_slice(&pes[pos..pos + n]);
            out.extend_from_slice(&p);
            pos += n;
            first = false;
        }
    }

    /// A video access unit (Annex B) at `pts` (90 kHz); tables first on
    /// keyframes.
    pub fn video(&mut self, pts: u64, key: bool, annexb: &[u8], out: &mut Vec<u8>) {
        if key {
            self.tables(out);
        }
        self.pes(PID_VIDEO, 0xE0, pts, Some(pts.saturating_sub(PCR_LEAD)), key, annexb, out);
    }

    /// ADTS audio at `pts` (90 kHz).
    pub fn audio(&mut self, pts: u64, adts: &[u8], out: &mut Vec<u8>) {
        self.pes(PID_AUDIO, 0xC0, pts, None, false, adts, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_of_a_known_pat() {
        // The standard single-program PAT (program 1 -> PID 0x1000).
        let pat = [0x00, 0xB0, 0x0D, 0x00, 0x01, 0xC1, 0x00, 0x00, 0x00, 0x01, 0xF0, 0x00];
        assert_eq!(crc32(&pat), 0x2AB104B2);
    }

    #[test]
    fn packets_are_well_formed() {
        let mut m = Muxer::new(false);
        let mut out = vec![];
        for (size, key) in [(10, true), (183, false), (184, false), (5000, false), (1, true)] {
            let au: Vec<u8> = (0..size).map(|i| i as u8).collect();
            m.video(90_000, key, &au, &mut out);
        }
        m.audio(90_000, &[0u8; 300], &mut out);
        assert_eq!(out.len() % PACKET, 0);
        let mut cc: std::collections::HashMap<u16, u8> = Default::default();
        for p in out.chunks(PACKET) {
            assert_eq!(p[0], 0x47);
            let pid = ((p[1] as u16 & 0x1F) << 8) | p[2] as u16;
            let c = p[3] & 0x0F;
            if let Some(prev) = cc.insert(pid, c) {
                assert_eq!(c, (prev + 1) & 0x0F, "continuity on {pid:#x}");
            }
            if p[3] & 0x20 != 0 {
                assert!(p[4] as usize <= 183);
            }
        }
    }

    #[test]
    fn pes_timestamps_round_trip() {
        let t = 0x1_2345_6789u64;
        let b = pes_ts(2, t);
        let back = (((b[0] as u64 >> 1) & 7) << 30) | ((b[1] as u64) << 22) | (((b[2] as u64) >> 1) << 15) | ((b[3] as u64) << 7) | ((b[4] as u64) >> 1);
        assert_eq!(back, t);
        assert_eq!(b[0] >> 4, 2);
    }

    #[test]
    fn annex_b_conversion() {
        let avcc = [0, 0, 0, 2, 0x65, 0xAA, 0, 0, 0, 1, 0x06];
        let sets = vec![vec![0x67, 1], vec![0x68, 2]];
        let a = annex_b(&avcc, false, true, &sets);
        assert_eq!(a, [0, 0, 0, 1, 0x09, 0xF0, 0, 0, 0, 1, 0x67, 1, 0, 0, 0, 1, 0x68, 2, 0, 0, 0, 1, 0x65, 0xAA, 0, 0, 0, 1, 0x06]);
        let a = annex_b(&avcc, true, false, &sets);
        assert_eq!(&a[..7], &[0, 0, 0, 1, 0x46, 0x01, 0x50]);
    }

    #[test]
    fn adts_header() {
        let h = adts(100, 3, 2);
        assert_eq!(&h[..2], &[0xFF, 0xF1]);
        let n = (((h[3] & 3) as usize) << 11) | ((h[4] as usize) << 3) | ((h[5] >> 5) as usize);
        assert_eq!(n, 107);
        assert_eq!((h[2] >> 2) & 0xF, 3);
    }
}
