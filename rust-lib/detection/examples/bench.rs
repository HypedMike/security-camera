use detection::{Background, Frame};
use std::time::Instant;

fn main() {
    // moving 60px square over a noisy gradient, 1280x720 Y plane
    let (w, h) = (1280usize, 720usize);
    let mut seed = 1u32;
    let mut frames = Vec::new();
    for f in 0..8 {
        let mut y = vec![0u8; w * h];
        for (i, p) in y.iter_mut().enumerate() {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            *p = (100 + (i % w) / 16 + ((seed >> 28) as usize % 9)) as u8;
        }
        if f >= 4 {
            for r in 300..360 {
                y[r * w + 200 + f * 5..r * w + 260 + f * 5].fill(20);
            }
        }
        frames.push(y);
    }
    let mut d = Background::new();
    for i in 0..40 {
        d.score(Frame::Gray { data: &frames[i % 4], width: w, height: h, stride: w }).unwrap();
    }
    let n = 2000;
    let t = Instant::now();
    let mut last = 0.0;
    for i in 0..n {
        last = d.score(Frame::Gray { data: &frames[i % 8], width: w, height: h, stride: w }).unwrap();
    }
    println!("{:?}/frame (last score {last})", t.elapsed() / n as u32);
}
