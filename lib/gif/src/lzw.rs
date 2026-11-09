//! GIF's LZW (literal width 8), as a series of data sub-blocks. The coding
//! follows Go's `compress/lzw` writer.

const LIT_WIDTH: u32 = 8;
const CLEAR: u32 = 1 << LIT_WIDTH;
const EOF: u32 = CLEAR + 1;
const MAX_CODE: u32 = (1 << 12) - 1;
const TABLE_BITS: u32 = 14;
const TABLE_MASK: u32 = (1 << TABLE_BITS) - 1;

struct Writer<'a> {
    out: &'a mut Vec<u8>,
    block: Vec<u8>,
    bits: u32,
    nbits: u32,
    width: u32,
    hi: u32,
    overflow: u32,
}

impl Writer<'_> {
    fn write(&mut self, code: u32) {
        self.bits |= code << self.nbits;
        self.nbits += self.width;
        while self.nbits >= 8 {
            self.push(self.bits as u8);
            self.bits >>= 8;
            self.nbits -= 8;
        }
    }

    fn push(&mut self, b: u8) {
        self.block.push(b);
        if self.block.len() == 255 {
            self.flush_block();
        }
    }

    fn flush_block(&mut self) {
        if !self.block.is_empty() {
            self.out.push(self.block.len() as u8);
            self.out.extend_from_slice(&self.block);
            self.block.clear();
        }
    }

    /// Moves on to the next implied code; true if the table was full and got reset.
    fn inc_hi(&mut self, table: &mut [u32]) -> bool {
        self.hi += 1;
        if self.hi == self.overflow {
            self.width += 1;
            self.overflow <<= 1;
        }
        if self.hi == MAX_CODE {
            self.write(CLEAR);
            self.width = LIT_WIDTH + 1;
            self.hi = EOF;
            self.overflow = CLEAR << 1;
            table.fill(0);
            return true;
        }
        false
    }
}

/// Appends `data` as LZW image data: the literal width, the sub-blocks and
/// the block terminator.
pub fn encode(data: &[u8], out: &mut Vec<u8>) {
    out.push(LIT_WIDTH as u8);
    let mut w = Writer {
        out,
        block: Vec::with_capacity(255),
        bits: 0,
        nbits: 0,
        width: LIT_WIDTH + 1,
        hi: EOF,
        overflow: CLEAR << 1,
    };
    let mut table = vec![0u32; 1 << TABLE_BITS];
    w.write(CLEAR);

    let mut it = data.iter();
    let Some(&first) = it.next() else {
        w.write(EOF);
        finish(w);
        return;
    };
    let mut code = first as u32;
    for &x in it {
        let lit = x as u32;
        let key = code << 8 | lit;
        let mut h = (key >> 12 ^ key) & TABLE_MASK;
        let mut found = None;
        loop {
            let t = table[h as usize];
            if t == 0 {
                break;
            }
            if t >> 12 == key {
                found = Some(t & MAX_CODE);
                break;
            }
            h = (h + 1) & TABLE_MASK;
        }
        if let Some(c) = found {
            code = c;
            continue;
        }
        w.write(code);
        code = lit;
        if w.inc_hi(&mut table) {
            continue;
        }
        table[h as usize] = key << 12 | w.hi;
    }
    w.write(code);
    w.inc_hi(&mut table);
    w.write(EOF);
    finish(w);
}

fn finish(mut w: Writer) {
    if w.nbits > 0 {
        w.push(w.bits as u8);
    }
    w.flush_block();
    w.out.push(0);
}
