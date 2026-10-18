//! SRT (Secure Reliable Transport) caller for live streaming, as servers
//! like OBS, vMix, Haivision, MediaMTX and FFmpeg's libsrt take it: the
//! HSv5 handshake (stream ID, latency, encryption), data packets carrying
//! MPEG-TS, retransmission of what the receiver reports lost (NAK),
//! ACK/ACKACK round trips, keepalives, and dropping packets too late to
//! play. With a passphrase, payloads are AES-CTR encrypted (the key is
//! derived and wrapped as SRT specifies). Plain Rust over a UDP socket.

use std::collections::VecDeque;
use std::net::{ToSocketAddrs, UdpSocket};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};

/// TS packets per SRT payload (7 x 188 = 1316 bytes, the usual).
pub const PAYLOAD: usize = 7 * 188;
const SEQ_MASK: u32 = 0x7FFF_FFFF;
const SRT_VERSION: u32 = 0x0001_0502;
const MTU: u32 = 1500;
const FLOW_WINDOW: u32 = 8192;

// Control packet types.
const CTRL_HANDSHAKE: u16 = 0;
const CTRL_KEEPALIVE: u16 = 1;
const CTRL_ACK: u16 = 2;
const CTRL_NAK: u16 = 3;
const CTRL_SHUTDOWN: u16 = 5;
const CTRL_ACKACK: u16 = 6;

// Handshake types and extensions.
const HS_INDUCTION: u32 = 1;
const HS_CONCLUSION: u32 = 0xFFFF_FFFF;
const HS_MAGIC: u16 = 0x4A17;
const EXT_HSREQ: u16 = 1;
const EXT_HSRSP: u16 = 2;
const EXT_KMREQ: u16 = 3;
const EXT_KMRSP: u16 = 4;
const EXT_SID: u16 = 5;
const HS_FLAG_HSREQ: u16 = 1;
const HS_FLAG_KMREQ: u16 = 2;
const HS_FLAG_CONFIG: u16 = 4;
// HSREQ flags: TSBPD send and receive, crypto, too-late drop, periodic
// NAK, retransmit flag.
const SRT_FLAGS: u32 = 0x01 | 0x02 | 0x04 | 0x08 | 0x10 | 0x20;

#[derive(Clone, Debug, PartialEq)]
pub struct SrtUrl {
    pub host: String,
    pub port: u16,
    pub streamid: Option<String>,
    pub passphrase: Option<String>,
    /// Receiver buffer delay (TSBPD latency).
    pub latency_ms: u32,
    /// AES key size in bytes: 16, 24 or 32.
    pub pbkeylen: usize,
}

/// Percent-decoding of a URL query value.
fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let hex = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2]))
        {
            out.push(h << 4 | l);
            i += 3;
        } else {
            out.push(if b[i] == b'+' { b' ' } else { b[i] });
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `srt://host:port?streamid=...&passphrase=...&latency=MS&pbkeylen=16`.
/// A `key` (from the Keychain) is the stream ID when the URL has none.
pub fn parse_url(url: &str, key: &str) -> Result<SrtUrl> {
    let rest = url.strip_prefix("srt://").ok_or_else(|| anyhow!("not an srt:// URL"))?;
    let (addr, query) = rest.split_once('?').unwrap_or((rest, ""));
    let addr = addr.trim_end_matches('/');
    let (host, port) = addr.rsplit_once(':').ok_or_else(|| anyhow!("srt:// URL needs a port (srt://host:port)"))?;
    let host = host.trim_start_matches('[').trim_end_matches(']').to_string();
    let port: u16 = port.parse().map_err(|_| anyhow!("bad port in {url}"))?;
    let mut u = SrtUrl { host, port, streamid: None, passphrase: None, latency_ms: 200, pbkeylen: 16 };
    for kv in query.split('&').filter(|s| !s.is_empty()) {
        let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
        let v = unescape(v);
        match k {
            "streamid" => u.streamid = Some(v),
            "passphrase" => u.passphrase = Some(v),
            "latency" => u.latency_ms = v.parse().map_err(|_| anyhow!("bad latency {v:?} (milliseconds)"))?,
            "pbkeylen" => u.pbkeylen = v.parse().map_err(|_| anyhow!("bad pbkeylen {v:?}"))?,
            _ => {}
        }
    }
    if u.streamid.is_none() && !key.is_empty() {
        u.streamid = Some(key.to_string());
    }
    if let Some(p) = &u.passphrase
        && !(10..=79).contains(&p.len())
    {
        bail!("SRT passphrase must be 10 to 79 characters");
    }
    if ![16, 24, 32].contains(&u.pbkeylen) {
        bail!("pbkeylen must be 16, 24 or 32");
    }
    u.latency_ms = u.latency_ms.clamp(20, 8000);
    Ok(u)
}

/// `a - b` for 31-bit sequence numbers (wrapping), signed.
fn seq_diff(a: u32, b: u32) -> i32 {
    let d = a.wrapping_sub(b) & SEQ_MASK;
    if d & 0x4000_0000 != 0 { (d | 0x8000_0000) as i32 } else { d as i32 }
}

fn random_u32() -> u32 {
    let mut b = [0u8; 4];
    let _ = getrandom::fill(&mut b);
    u32::from_le_bytes(b)
}

/// Packet encryption: AES-CTR with a random stream key, sent to the
/// receiver wrapped in a key derived from the passphrase.
pub struct Crypto {
    salt: [u8; 16],
    sek: Vec<u8>,
    /// The keying material message (KMREQ).
    pub km: Vec<u8>,
}

impl Crypto {
    pub fn new(passphrase: &str, keylen: usize) -> Result<Crypto> {
        let mut salt = [0u8; 16];
        let mut sek = vec![0u8; keylen];
        getrandom::fill(&mut salt).map_err(|e| anyhow!("random: {e}"))?;
        getrandom::fill(&mut sek).map_err(|e| anyhow!("random: {e}"))?;
        Self::with(passphrase, salt, sek)
    }

    /// With a given salt and stream key (tests).
    pub fn with(passphrase: &str, salt: [u8; 16], sek: Vec<u8>) -> Result<Crypto> {
        let keylen = sek.len();
        // KEK = PBKDF2-HMAC-SHA1(passphrase, last 8 bytes of the salt, 2048).
        let mut kek = vec![0u8; keylen];
        pbkdf2::pbkdf2_hmac::<sha1::Sha1>(passphrase.as_bytes(), &salt[8..], 2048, &mut kek);
        let mut wrapped = vec![0u8; keylen + 8];
        let res = match keylen {
            16 => aes_kw::KekAes128::from(<[u8; 16]>::try_from(&kek[..]).unwrap()).wrap(&sek, &mut wrapped),
            24 => aes_kw::KekAes192::from(<[u8; 24]>::try_from(&kek[..]).unwrap()).wrap(&sek, &mut wrapped),
            32 => aes_kw::KekAes256::from(<[u8; 32]>::try_from(&kek[..]).unwrap()).wrap(&sek, &mut wrapped),
            _ => bail!("bad key length {keylen}"),
        };
        res.map_err(|e| anyhow!("key wrap: {e:?}"))?;
        let mut km = vec![
            0x12, // version 1, packet type 2 (keying material)
            0x20,
            0x29, // signature "HAI"
            0x01, // even key
            0,
            0,
            0,
            0, // KEK index
            2, // AES-CTR
            0, // no authentication
            2, // MPEG-TS over SRT
            0,
            0,
            0,
            4, // salt length / 4
            (keylen / 4) as u8,
        ];
        km.extend_from_slice(&salt);
        km.extend_from_slice(&wrapped);
        Ok(Crypto { salt, sek, km })
    }

    /// Encrypt a payload of packet `seq` in place.
    pub fn apply(&self, seq: u32, data: &mut [u8]) {
        use ctr::cipher::{KeyIvInit, StreamCipher};
        // IV: the salt's first 112 bits XOR the sequence number, then a
        // 16-bit block counter.
        let mut iv = [0u8; 16];
        iv[..14].copy_from_slice(&self.salt[..14]);
        for (i, b) in seq.to_be_bytes().iter().enumerate() {
            iv[10 + i] ^= b;
        }
        match self.sek.len() {
            16 => ctr::Ctr128BE::<aes::Aes128>::new(self.sek[..].into(), &iv.into()).apply_keystream(data),
            24 => ctr::Ctr128BE::<aes::Aes192>::new(self.sek[..].into(), &iv.into()).apply_keystream(data),
            _ => ctr::Ctr128BE::<aes::Aes256>::new(self.sek[..].into(), &iv.into()).apply_keystream(data),
        }
    }
}

/// A control packet: header (type, type info, timestamp, destination).
fn control(kind: u16, info: u32, ts: u32, dest: u32, cif: &[u8]) -> Vec<u8> {
    let mut p = Vec::with_capacity(16 + cif.len());
    p.extend_from_slice(&(0x8000_0000u32 | (kind as u32) << 16).to_be_bytes());
    p.extend_from_slice(&info.to_be_bytes());
    p.extend_from_slice(&ts.to_be_bytes());
    p.extend_from_slice(&dest.to_be_bytes());
    p.extend_from_slice(cif);
    p
}

struct Handshake {
    version: u32,
    enc: u16,
    ext: u16,
    isn: u32,
    kind: u32,
    socket: u32,
    cookie: u32,
    /// Extensions: (type, data).
    exts: Vec<(u16, Vec<u8>)>,
}

impl Handshake {
    fn encode(&self) -> Vec<u8> {
        let mut c = Vec::with_capacity(64);
        c.extend_from_slice(&self.version.to_be_bytes());
        c.extend_from_slice(&self.enc.to_be_bytes());
        c.extend_from_slice(&self.ext.to_be_bytes());
        c.extend_from_slice(&self.isn.to_be_bytes());
        c.extend_from_slice(&MTU.to_be_bytes());
        c.extend_from_slice(&FLOW_WINDOW.to_be_bytes());
        c.extend_from_slice(&self.kind.to_be_bytes());
        c.extend_from_slice(&self.socket.to_be_bytes());
        c.extend_from_slice(&self.cookie.to_be_bytes());
        c.extend_from_slice(&[0u8; 16]); // peer address (informational)
        for (t, d) in &self.exts {
            c.extend_from_slice(&t.to_be_bytes());
            c.extend_from_slice(&((d.len() / 4) as u16).to_be_bytes());
            c.extend_from_slice(d);
        }
        c
    }

    fn decode(cif: &[u8]) -> Option<Handshake> {
        if cif.len() < 48 {
            return None;
        }
        let u = |i: usize| u32::from_be_bytes([cif[i], cif[i + 1], cif[i + 2], cif[i + 3]]);
        let mut exts = vec![];
        let mut i = 48;
        while i + 4 <= cif.len() {
            let t = u16::from_be_bytes([cif[i], cif[i + 1]]);
            let n = u16::from_be_bytes([cif[i + 2], cif[i + 3]]) as usize * 4;
            i += 4;
            if i + n > cif.len() {
                break;
            }
            exts.push((t, cif[i..i + n].to_vec()));
            i += n;
        }
        Some(Handshake {
            version: u(0),
            enc: (u(4) >> 16) as u16,
            ext: u(4) as u16,
            isn: u(8),
            kind: u(20),
            socket: u(24),
            cookie: u(28),
            exts,
        })
    }
}

/// The stream ID as SRT carries it: padded to 4 bytes and byte-reversed in
/// each 4-byte group (libsrt sends it as host-order 32-bit words).
pub fn sid_bytes(s: &str) -> Vec<u8> {
    let mut b = s.as_bytes().to_vec();
    b.truncate(512);
    while b.len() % 4 != 0 {
        b.push(0);
    }
    for w in b.chunks_mut(4) {
        w.reverse();
    }
    b
}

/// Why the listener refused us, from a rejecting handshake type.
fn reject_reason(kind: u32) -> String {
    let code = kind.wrapping_sub(1000);
    let text = match code {
        0 => "unknown reason",
        1 => "system error on the server",
        2 => "rejected by the server",
        3 => "server out of resources",
        4 => "rogue handshake",
        5 => "server backlog full",
        6 => "server internal error",
        7 => "server closing",
        8 => "SRT version not supported",
        9 => "rendezvous cookie collision",
        10 => "wrong passphrase",
        11 => "the server requires a passphrase",
        12 => "message API mismatch",
        13 => "congestion control mismatch",
        14 => "packet filter mismatch",
        15 => "group mismatch",
        16 => "connection timeout",
        17 => "crypto error",
        c if (1000..2000).contains(&c) => return format!("rejected by the server application (code {c}); check the stream ID"),
        _ => "rejected",
    };
    format!("{text} ({kind})")
}

pub struct Conn {
    sock: UdpSocket,
    peer: u32,
    start: Instant,
    next_seq: u32,
    next_msg: u32,
    /// Sent, not yet acknowledged: (sequence number, sent at, packet).
    unacked: VecDeque<(u32, Instant, Vec<u8>)>,
    /// Packets older than this are too late to play: dropped, not resent.
    drop_after: Duration,
    crypto: Option<Crypto>,
    last_heard: Instant,
    last_sent: Instant,
    pub sent_bytes: u64,
    pub retransmits: u64,
    /// Packets dropped as too late.
    pub dropped: u64,
    pub rtt_ms: f64,
}

/// Silence from the receiver for this long means the connection is gone.
const PEER_TIMEOUT: Duration = Duration::from_secs(5);
const KEEPALIVE: Duration = Duration::from_secs(1);

impl Conn {
    pub fn connect(u: &SrtUrl) -> Result<Conn> {
        let addr = (u.host.as_str(), u.port).to_socket_addrs()?.next().ok_or_else(|| anyhow!("{} has no address", u.host))?;
        let sock = UdpSocket::bind(if addr.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" })?;
        sock.connect(addr)?;
        // Room for a keyframe burst.
        unsafe {
            let size: libc::c_int = 4 << 20;
            libc::setsockopt(
                std::os::fd::AsRawFd::as_raw_fd(&sock),
                libc::SOL_SOCKET,
                libc::SO_SNDBUF,
                &size as *const _ as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            );
        }
        let crypto = match &u.passphrase {
            Some(p) => Some(Crypto::new(p, u.pbkeylen)?),
            None => None,
        };
        let me = (random_u32() & 0x3FFF_FFFF) | 1;
        let isn = random_u32() & SEQ_MASK;
        let start = Instant::now();
        let ts = || start.elapsed().as_micros() as u32;
        let mut buf = vec![0u8; 2048];
        // Induction: learn the listener's cookie.
        let induction = Handshake { version: 4, enc: 0, ext: 2, isn, kind: HS_INDUCTION, socket: me, cookie: 0, exts: vec![] };
        let reply = Self::exchange(&sock, &mut buf, &control(CTRL_HANDSHAKE, 0, ts(), 0, &induction.encode()), &|h| h.kind == HS_INDUCTION)?;
        if reply.version != 5 || reply.ext != HS_MAGIC {
            bail!("the server doesn't speak SRT 1.3+ (handshake version {})", reply.version);
        }
        // Conclusion: our settings.
        let latency = u.latency_ms.min(0xFFFF);
        let hsreq: Vec<u8> = [SRT_VERSION, SRT_FLAGS, (latency << 16) | latency].iter().flat_map(|w| w.to_be_bytes()).collect();
        let mut exts = vec![(EXT_HSREQ, hsreq)];
        let mut flags = HS_FLAG_HSREQ;
        if let Some(c) = &crypto {
            exts.push((EXT_KMREQ, c.km.clone()));
            flags |= HS_FLAG_KMREQ;
        }
        if let Some(sid) = &u.streamid {
            exts.push((EXT_SID, sid_bytes(sid)));
            flags |= HS_FLAG_CONFIG;
        }
        let conclusion = Handshake {
            version: 5,
            enc: crypto.as_ref().map_or(0, |_| (u.pbkeylen / 8) as u16),
            ext: flags,
            isn,
            kind: HS_CONCLUSION,
            socket: me,
            cookie: reply.cookie,
            exts,
        };
        let reply = Self::exchange(&sock, &mut buf, &control(CTRL_HANDSHAKE, 0, ts(), 0, &conclusion.encode()), &|h| h.kind != HS_INDUCTION)?;
        if reply.kind != HS_CONCLUSION {
            bail!("SRT server refused the connection: {}", reject_reason(reply.kind));
        }
        if crypto.is_some() {
            // A one-word KMRSP is an error state.
            match reply.exts.iter().find(|(t, _)| *t == EXT_KMRSP) {
                Some((_, d)) if d.len() == 4 => {
                    let state = u32::from_be_bytes([d[0], d[1], d[2], d[3]]);
                    bail!("SRT encryption refused: {}", match state {
                        3 => "wrong passphrase",
                        4 => "the server has no passphrase",
                        _ => "key exchange failed",
                    });
                }
                Some(_) => {}
                None => bail!("SRT server didn't accept encryption"),
            }
        }
        if !reply.exts.iter().any(|(t, _)| *t == EXT_HSRSP) {
            bail!("SRT server sent no handshake response");
        }
        sock.set_nonblocking(true)?;
        let now = Instant::now();
        // Too late: past the receiver's latency (with slack), like libsrt.
        let drop_after = Duration::from_millis((u.latency_ms as u64 * 5 / 4 + 20).max(1000));
        Ok(Conn {
            sock,
            peer: reply.socket,
            start,
            next_seq: isn,
            next_msg: 1,
            unacked: VecDeque::new(),
            drop_after,
            crypto,
            last_heard: now,
            last_sent: now,
            sent_bytes: 0,
            retransmits: 0,
            dropped: 0,
            rtt_ms: 0.0,
        })
    }

    /// Send a handshake until a matching reply arrives (3 s).
    fn exchange(sock: &UdpSocket, buf: &mut [u8], packet: &[u8], want: &dyn Fn(&Handshake) -> bool) -> Result<Handshake> {
        let deadline = Instant::now() + Duration::from_secs(3);
        sock.set_read_timeout(Some(Duration::from_millis(250)))?;
        while Instant::now() < deadline {
            sock.send(packet)?;
            loop {
                match sock.recv(buf) {
                    Ok(n) if n >= 16 && buf[0] & 0x80 != 0 && u16::from_be_bytes([buf[0], buf[1]]) & 0x7FFF == CTRL_HANDSHAKE => {
                        if let Some(h) = Handshake::decode(&buf[16..n])
                            && want(&h)
                        {
                            return Ok(h);
                        }
                    }
                    Ok(_) => {}
                    Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => break,
                    Err(e) => return Err(anyhow!("SRT handshake: {e}")),
                }
            }
        }
        bail!("SRT server didn't answer (is it listening, and is UDP allowed?)")
    }

    fn ts(&self) -> u32 {
        self.start.elapsed().as_micros() as u32
    }

    fn raw_send(&mut self, p: &[u8]) -> Result<()> {
        match self.sock.send(p) {
            Ok(_) => {
                self.last_sent = Instant::now();
                self.sent_bytes += p.len() as u64;
                Ok(())
            }
            // A full socket buffer: the packet is lost; the receiver asks
            // for it again.
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::ConnectionRefused => bail!("SRT server went away"),
            Err(e) => Err(e.into()),
        }
    }

    /// Send one payload (at most `PAYLOAD` bytes of TS) as a data packet.
    pub fn send(&mut self, payload: &[u8]) -> Result<()> {
        let seq = self.next_seq;
        self.next_seq = (seq + 1) & SEQ_MASK;
        let msg = self.next_msg;
        self.next_msg = if msg >= 0x03FF_FFFF { 1 } else { msg + 1 };
        let kk = if self.crypto.is_some() { 1 } else { 0 };
        // A whole message in one packet (PP=11), not in-order, not resent.
        let word1 = (0b11 << 30) | (kk << 27) | msg;
        let mut p = Vec::with_capacity(16 + payload.len());
        p.extend_from_slice(&seq.to_be_bytes());
        p.extend_from_slice(&word1.to_be_bytes());
        p.extend_from_slice(&self.ts().to_be_bytes());
        p.extend_from_slice(&self.peer.to_be_bytes());
        p.extend_from_slice(payload);
        if let Some(c) = &self.crypto {
            c.apply(seq, &mut p[16..]);
        }
        self.raw_send(&p)?;
        self.unacked.push_back((seq, Instant::now(), p));
        if self.unacked.len() > FLOW_WINDOW as usize {
            self.unacked.pop_front();
            self.dropped += 1;
        }
        Ok(())
    }

    /// Handle what the receiver sent (ACKs, loss reports, keepalives),
    /// drop packets too late to play, and keep the connection alive.
    pub fn poll(&mut self) -> Result<()> {
        let mut buf = [0u8; 1500];
        loop {
            let n = match self.sock.recv(&mut buf) {
                Ok(n) => n,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == std::io::ErrorKind::ConnectionRefused => bail!("SRT server went away"),
                Err(e) => return Err(e.into()),
            };
            if n < 16 || buf[0] & 0x80 == 0 {
                continue;
            }
            self.last_heard = Instant::now();
            let kind = u16::from_be_bytes([buf[0], buf[1]]) & 0x7FFF;
            let info = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]);
            let cif = &buf[16..n];
            let word = |i: usize| u32::from_be_bytes([cif[4 * i], cif[4 * i + 1], cif[4 * i + 2], cif[4 * i + 3]]);
            match kind {
                CTRL_ACK if cif.len() >= 4 => {
                    let acked = word(0) & SEQ_MASK;
                    while self.unacked.front().is_some_and(|(s, _, _)| seq_diff(*s, acked) < 0) {
                        self.unacked.pop_front();
                    }
                    // A full ACK (not a light one) wants an ACKACK back.
                    if cif.len() >= 16 {
                        self.rtt_ms = word(1) as f64 / 1000.0;
                        let p = control(CTRL_ACKACK, info, self.ts(), self.peer, &[]);
                        self.raw_send(&p)?;
                    }
                }
                CTRL_NAK => {
                    let mut lost = vec![];
                    let mut i = 0;
                    while 4 * i + 4 <= cif.len() {
                        let w = word(i);
                        if w & 0x8000_0000 != 0 && 4 * i + 8 <= cif.len() {
                            lost.push((w & SEQ_MASK, word(i + 1) & SEQ_MASK));
                            i += 2;
                        } else {
                            lost.push((w & SEQ_MASK, w & SEQ_MASK));
                            i += 1;
                        }
                    }
                    self.resend(&lost)?;
                }
                CTRL_SHUTDOWN => bail!("the SRT server closed the connection"),
                _ => {}
            }
        }
        // Too late to play: drop (the receiver skips them too).
        let now = Instant::now();
        while self.unacked.front().is_some_and(|(_, t, _)| now - *t > self.drop_after) {
            self.unacked.pop_front();
            self.dropped += 1;
        }
        if now - self.last_heard > PEER_TIMEOUT {
            bail!("SRT server stopped answering");
        }
        if now - self.last_sent > KEEPALIVE {
            let p = control(CTRL_KEEPALIVE, 0, self.ts(), self.peer, &[]);
            self.raw_send(&p)?;
        }
        Ok(())
    }

    /// Send the packets in the lost ranges again, flagged as resent.
    fn resend(&mut self, lost: &[(u32, u32)]) -> Result<()> {
        let mut again = vec![];
        for (a, b) in lost {
            for (s, _, p) in &self.unacked {
                if seq_diff(*s, *a) >= 0 && seq_diff(*s, *b) <= 0 {
                    let mut p = p.clone();
                    p[4] |= 0x04; // R: retransmitted
                    again.push(p);
                }
            }
        }
        for p in again {
            self.raw_send(&p)?;
            self.retransmits += 1;
        }
        Ok(())
    }

    /// Tell the server we're leaving.
    pub fn close(&mut self) {
        let p = control(CTRL_SHUTDOWN, 0, self.ts(), self.peer, &[0; 4]);
        let _ = self.sock.send(&p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls() {
        let u = parse_url("srt://example.com:9000?streamid=live/abc&latency=300", "").unwrap();
        assert_eq!((u.host.as_str(), u.port, u.streamid.as_deref(), u.latency_ms), ("example.com", 9000, Some("live/abc"), 300));
        let u = parse_url("srt://1.2.3.4:4200", "mykey").unwrap();
        assert_eq!(u.streamid.as_deref(), Some("mykey"), "the Keychain key is the stream ID");
        let u = parse_url("srt://h:1?streamid=%23!%3A%3Ar%3Dlive%2Fx%2Cm%3Dpublish&passphrase=0123456789", "k").unwrap();
        assert_eq!(u.streamid.as_deref(), Some("#!::r=live/x,m=publish"));
        assert_eq!(u.passphrase.as_deref(), Some("0123456789"));
        assert!(parse_url("srt://h", "").is_err());
        assert!(parse_url("srt://h:1?passphrase=short", "").is_err());
    }

    #[test]
    fn sequence_numbers_wrap() {
        assert_eq!(seq_diff(5, 3), 2);
        assert_eq!(seq_diff(3, 5), -2);
        assert_eq!(seq_diff(0, SEQ_MASK), 1);
        assert_eq!(seq_diff(SEQ_MASK, 0), -1);
    }

    #[test]
    fn stream_id_word_order() {
        assert_eq!(sid_bytes("abcdef"), b"dcba\0\0fe");
    }

    #[test]
    fn handshake_round_trip() {
        let h = Handshake { version: 5, enc: 2, ext: 7, isn: 123, kind: HS_CONCLUSION, socket: 9, cookie: 77, exts: vec![(EXT_SID, sid_bytes("x"))] };
        let d = Handshake::decode(&h.encode()).unwrap();
        assert_eq!((d.version, d.enc, d.ext, d.isn, d.kind, d.socket, d.cookie), (5, 2, 7, 123, HS_CONCLUSION, 9, 77));
        assert_eq!(d.exts, h.exts);
    }

    /// RFC 3394 4.1 (the key wrap) and the KM message layout.
    #[test]
    fn keying_material() {
        let kek: [u8; 16] = (0..16).collect::<Vec<u8>>().try_into().unwrap();
        let key: Vec<u8> = (0..16).map(|i| (i * 0x11) as u8).collect();
        let mut out = [0u8; 24];
        aes_kw::KekAes128::from(kek).wrap(&key, &mut out).unwrap();
        assert_eq!(out[..8], [0x1F, 0xA6, 0x8B, 0x0A, 0x81, 0x12, 0xB4, 0x47]);
        let c = Crypto::with("0123456789", [7; 16], key.clone()).unwrap();
        assert_eq!(c.km.len(), 16 + 16 + 24);
        assert_eq!(&c.km[..4], &[0x12, 0x20, 0x29, 0x01]);
        assert_eq!(c.km[15], 4);
        // CTR is its own inverse, and depends on the sequence number.
        let mut d = b"hello transport stream".to_vec();
        c.apply(42, &mut d);
        let enc = d.clone();
        c.apply(42, &mut d);
        assert_eq!(d, b"hello transport stream");
        let mut e = b"hello transport stream".to_vec();
        c.apply(43, &mut e);
        assert_ne!(e, enc);
    }
}
