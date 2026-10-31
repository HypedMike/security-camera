use detection_lib::{Background, Frame};

struct Rng(u64);
impl Rng {
    fn below(&mut self, n: u32) -> u32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.0 >> 33) as u32) % n
    }
}

/// 640x480 gray RGBA frame with sensor noise, an optional square of side
/// `size` at (x, y) and an overall brightness shift.
fn scene(rng: &mut Rng, x: usize, y: usize, size: usize, shift: i32) -> Vec<u8> {
    let mut img = vec![255u8; 640 * 480 * 4];
    for py in 0..480 {
        for px in 0..640 {
            let mut v = 100 + (px / 16) as i32 + shift + rng.below(9) as i32 - 4;
            if size > 0 && px >= x && px < x + size && py >= y && py < y + size {
                v = 30;
            }
            let i = (py * 640 + px) * 4;
            img[i..i + 3].fill(v.clamp(0, 255) as u8);
        }
    }
    img
}

fn score(d: &mut Background, img: &[u8]) -> f64 {
    d.score(Frame::Rgba { data: img, width: 640, height: 480, stride: 640 * 4 }).unwrap()
}

fn warm(d: &mut Background, rng: &mut Rng) {
    for i in 0..40 {
        let s = score(d, &scene(rng, 0, 0, 0, 0));
        assert!(i < 20 || s == 0.0, "still frame {i} scored {s}");
    }
}

#[test]
fn small_motion() {
    let mut rng = Rng(1);
    let mut d = Background::new();
    warm(&mut d, &mut rng);
    let mut s = 0.0;
    for i in 0..5 {
        s = score(&mut d, &scene(&mut rng, 560 + 4 * i, 40, 16, 0));
    }
    assert!(s >= 0.0005, "small moving square scored {s}");
    assert!(s <= 0.01, "scored {s}, larger than the square");
}

#[test]
fn persistence() {
    let mut rng = Rng(2);
    let mut d = Background::new();
    warm(&mut d, &mut rng);
    assert_eq!(score(&mut d, &scene(&mut rng, 300, 200, 40, 0)), 0.0);
    score(&mut d, &scene(&mut rng, 0, 0, 0, 0));
    assert_eq!(score(&mut d, &scene(&mut rng, 0, 0, 0, 0)), 0.0);
}

#[test]
fn lighting() {
    let mut rng = Rng(3);
    let mut d = Background::new();
    warm(&mut d, &mut rng);
    for i in 0..30 {
        assert_eq!(score(&mut d, &scene(&mut rng, 0, 0, 0, 60)), 0.0, "frame {i}");
    }
    let mut s = 0.0;
    for _ in 0..5 {
        s = score(&mut d, &scene(&mut rng, 300, 200, 40, 60));
    }
    assert!(s != 0.0, "motion after the lighting change not detected");
}

#[test]
fn still_object_stays_detected() {
    let mut rng = Rng(4);
    let mut d = Background::new();
    warm(&mut d, &mut rng);
    for i in 0..300 {
        let s = score(&mut d, &scene(&mut rng, 300, 200, 40, 0));
        assert!(i < 5 || s != 0.0, "still object forgotten after {i} frames");
    }
}

#[test]
fn reset() {
    let mut rng = Rng(5);
    let mut d = Background::new();
    warm(&mut d, &mut rng);
    d.reset();
    for i in 0..=d.settle {
        assert_eq!(score(&mut d, &scene(&mut rng, 300, 200, 40, 0)), 0.0, "frame {i}");
    }
}

#[test]
fn gray_plane() {
    let mut d = Background::new();
    let frame = |obj: bool| {
        let mut y = vec![120u8; 1280 * 720];
        if obj {
            for r in 100..160 {
                y[r * 1280 + 100..r * 1280 + 160].fill(20);
            }
        }
        y
    };
    let run = |d: &mut Background, f: &[u8]| {
        d.score(Frame::Gray { data: f, width: 1280, height: 720, stride: 1280 }).unwrap()
    };
    for _ in 0..30 {
        run(&mut d, &frame(false));
    }
    let mut s = 0.0;
    for _ in 0..5 {
        s = run(&mut d, &frame(true));
    }
    // 60x60 source pixels are 15x15 of the 320x180 detection image
    let want = 225.0 / (320.0 * 180.0);
    assert!(s > want * 0.8 && s < want * 1.8, "score {s}, want about {want}");
}
