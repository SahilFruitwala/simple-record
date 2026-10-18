//! A lossy UDP relay for testing SRT retransmission: listen on LISTEN,
//! forward to SERVER, dropping data packets (client -> server) at UP percent
//! and packets back at DOWN percent (pseudo-random, repeatable).
//!
//! Usage: lossy_relay LISTEN SERVER UP DOWN
use std::net::UdpSocket;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let front = UdpSocket::bind(&a[1]).unwrap();
    let back = UdpSocket::bind("127.0.0.1:0").unwrap();
    back.connect(&a[2]).unwrap();
    let (loss_up, loss_down): (f64, f64) = (a[3].parse().unwrap(), a[4].parse().unwrap());
    let client = std::sync::Arc::new(std::sync::Mutex::new(None));
    let (f2, b2, c2) = (front.try_clone().unwrap(), back.try_clone().unwrap(), client.clone());
    let mut seed = 12345u64;
    let mut rnd = move || { seed ^= seed << 13; seed ^= seed >> 7; seed ^= seed << 17; (seed % 10000) as f64 / 100.0 };
    let mut rnd2 = { let mut s = 999u64; move || { s ^= s << 13; s ^= s >> 7; s ^= s << 17; (s % 10000) as f64 / 100.0 } };
    std::thread::spawn(move || {
        let mut buf = [0u8; 2048];
        let (mut dropped, mut total) = (0u64, 0u64);
        loop {
            let n = b2.recv(&mut buf).unwrap();
            total += 1;
            if rnd2() < loss_down && total > 4 { dropped += 1; if dropped % 20 == 0 { eprintln!("relay: dropped {dropped} of {total} back"); } continue; }
            if let Some(c) = *c2.lock().unwrap() { let _ = f2.send_to(&buf[..n], c); }
        }
    });
    let mut buf = [0u8; 2048];
    let (mut dropped, mut total) = (0u64, 0u64);
    loop {
        let (n, from) = front.recv_from(&mut buf).unwrap();
        *client.lock().unwrap() = Some(from);
        let data = buf[0] & 0x80 == 0;
        if data { total += 1; }
        if data && rnd() < loss_up { dropped += 1; if dropped % 50 == 0 { eprintln!("relay: dropped {dropped} of {total} data"); } continue; }
        let _ = back.send(&buf[..n]);
    }
}
