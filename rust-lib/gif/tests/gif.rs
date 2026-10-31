use gif::{Encoder, Image, encode, shrink, shrink_dims};

fn rgba(w: usize, h: usize, f: impl Fn(usize, usize) -> [u8; 3]) -> Vec<u8> {
    let mut v = vec![255u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            v[(y * w + x) * 4..][..3].copy_from_slice(&f(x, y));
        }
    }
    v
}

#[test]
fn shrink_averages_blocks() {
    let data = rgba(640, 480, |x, _| if x % 2 == 0 { [200, 100, 0] } else { [100, 0, 0] });
    let img = Image::Rgba { data: &data, width: 640, height: 480, stride: 640 * 4 };
    assert_eq!(shrink_dims(640, 480, 320), (2, 320, 240));
    let s = shrink(&img, 320);
    assert_eq!((s.width, s.height), (320, 240));
    assert_eq!(&s.pix[..4], &[150, 50, 0, 255]);
    // already small enough: kept as is
    assert_eq!(shrink_dims(200, 100, 320), (1, 200, 100));
}

#[test]
fn encodes_a_gif_with_every_frame() {
    let data = rgba(64, 48, |x, y| [(x * 4) as u8, (y * 5) as u8, 90]);
    let img = Image::Rgba { data: &data, width: 64, height: 48, stride: 64 * 4 };
    for dither in [false, true] {
        let gif = encode(&[img, img, img], 10, dither).unwrap();
        assert_eq!(&gif[..6], b"GIF89a");
        assert_eq!(*gif.last().unwrap(), 0x3b);
        assert_eq!(gif.windows(2).filter(|w| w == &[0x21, 0xf9]).count(), 3);
    }
    assert!(encode(&[], 10, false).is_none());
}

#[test]
fn flat_colour_compresses() {
    let data = rgba(320, 240, |_, _| [0, 0, 0]);
    let img = Image::Rgba { data: &data, width: 320, height: 240, stride: 320 * 4 };
    let mut enc = Encoder::new(320, 240, 5, false);
    enc.add(&img);
    assert!(enc.finish().len() < 3000);
}
