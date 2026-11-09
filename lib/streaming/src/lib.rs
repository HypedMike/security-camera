//! The frames a stream remembers: the last `capacity` frames, copied as
//! tightly packed RGBA, oldest first. A slot's memory is reused when the next
//! frame has the same size.

use gif::Image;


#[derive(Default)]
struct Slot {
    width: usize,
    height: usize,
    data: Vec<u8>,
}

pub struct Ring {
    slots: Vec<Slot>,
    /// The slot the next frame goes into.
    next: usize,
    count: usize,
}

impl Ring {
    pub fn new(capacity: usize) -> Ring {
        Ring { slots: (0..capacity).map(|_| Slot::default()).collect(), next: 0, count: 0 }
    }

    pub fn capacity(&self) -> usize {
        self.slots.len()
    }

    /// How many frames are stored.
    pub fn len(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Changes how many frames are kept. When shrinking, the newest are kept.
    pub fn set_capacity(&mut self, n: usize) {
        if n == self.slots.len() {
            return;
        }
        let keep = self.count.min(n);
        let cap = self.slots.len();
        let mut slots: Vec<Slot> = (0..keep)
            .map(|i| std::mem::take(&mut self.slots[(self.next + cap - keep + i) % cap]))
            .collect();
        slots.resize_with(n, Slot::default);
        self.slots = slots;
        self.count = keep;
        self.next = if n > 0 { keep % n } else { 0 };
    }

    /// Copies `img` into the ring, forgetting the oldest frame if it is full.
    pub fn remember(&mut self, img: &Image) {
        if self.slots.is_empty() {
            return;
        }
        let (width, height) = img.size();
        let slot = &mut self.slots[self.next];
        slot.width = width;
        slot.height = height;
        slot.data.resize(width * height * 4, 0);
        img.write_rgba(&mut slot.data);
        self.next = (self.next + 1) % self.slots.len();
        self.count = (self.count + 1).min(self.slots.len());
    }

    /// Frame `i`, 0 being the oldest: its RGBA bytes, width and height.
    pub fn frame(&self, i: usize) -> Option<(&[u8], usize, usize)> {
        if i >= self.count {
            return None;
        }
        let cap = self.slots.len();
        let s = &self.slots[(self.next + cap - self.count + i) % cap];
        Some((&s.data, s.width, s.height))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn push(r: &mut Ring, v: u8) {
        let px = [v, 0, 0, 255];
        r.remember(&Image::Rgba { data: &px, width: 1, height: 1, stride: 4 });
    }

    fn reds(r: &Ring) -> Vec<u8> {
        (0..r.len()).map(|i| r.frame(i).unwrap().0[0]).collect()
    }

    #[test]
    fn keeps_the_newest_oldest_first() {
        let mut r = Ring::new(3);
        for i in 1..=5u8 {
            assert_eq!(r.len(), (i as usize - 1).min(3));
            push(&mut r, i);
        }
        assert_eq!(reds(&r), [3, 4, 5]);
        assert!(r.frame(3).is_none());
    }

    #[test]
    fn set_capacity_keeps_newest() {
        let mut r = Ring::new(4);
        for i in 1..=6 {
            push(&mut r, i);
        }
        assert_eq!(reds(&r), [3, 4, 5, 6]);
        r.set_capacity(2);
        assert_eq!(reds(&r), [5, 6]);
        push(&mut r, 7);
        assert_eq!(reds(&r), [6, 7]);
        r.set_capacity(5);
        assert_eq!(reds(&r), [6, 7]);
        push(&mut r, 8);
        assert_eq!(reds(&r), [6, 7, 8]);
        r.set_capacity(0);
        push(&mut r, 9);
        assert!(r.is_empty());
    }

    #[test]
    fn ycbcr_frames_are_converted() {
        let mut r = Ring::new(1);
        r.remember(&Image::YCbCr { y: &[255], cb: &[128], cr: &[128], ystride: 1, cstride: 1, xs: 0, ys: 0, width: 1, height: 1 });
        assert_eq!(r.frame(0).unwrap().0, [255, 255, 255, 255]);
    }
}
