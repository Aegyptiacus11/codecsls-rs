//! The entropy-coded segment as bits, most significant first, with the bit stuffing of
//! T.87 A.1: the byte after an 0xFF carries seven bits under a zero, so no marker can
//! appear in coded data.
//!
//! Both sides keep 64 bits and move four bytes at a time when none of them is 0xFF, which
//! is nearly always.

use super::Error;

/// Whether any of the four bytes is 0xFF.
#[inline(always)]
fn has_ff(w: u32) -> bool {
    let x = !w;
    x.wrapping_sub(0x0101_0101) & !x & 0x8080_8080 != 0
}

pub(crate) struct BitWriter {
    out: Vec<u8>,
    /// The bits not written yet are the low `n`; what is above them is stale.
    acc: u64,
    n: u32,
    after_ff: bool,
}

impl BitWriter {
    /// Bits go after what `out` already holds.
    pub fn new(out: Vec<u8>) -> BitWriter {
        BitWriter {
            out,
            acc: 0,
            n: 0,
            after_ff: false,
        }
    }

    /// The low `len` bits of `value`, which has none above them; `len` at most 32.
    #[inline(always)]
    pub fn put(&mut self, value: u32, len: u32) {
        debug_assert!(len <= 32 && u64::from(value) >> len == 0);
        self.acc = (self.acc << len) | u64::from(value);
        self.n += len;
        if self.n >= 32 {
            self.flush();
        }
    }

    #[inline(always)]
    fn flush(&mut self) {
        if !self.after_ff {
            let word = (self.acc >> (self.n - 32)) as u32;
            if !has_ff(word) {
                self.out.extend_from_slice(&word.to_be_bytes());
                self.n -= 32;
                return;
            }
        }
        self.flush_bytes();
    }

    /// Byte by byte, for the bytes around an 0xFF. Leaves fewer than 8 bits.
    #[cold]
    fn flush_bytes(&mut self) {
        while self.n >= 8 {
            let byte = if self.after_ff {
                self.n -= 7;
                (self.acc >> self.n) as u8 & 0x7F
            } else {
                self.n -= 8;
                (self.acc >> self.n) as u8
            };
            self.after_ff = byte == 0xFF;
            self.out.push(byte);
        }
    }

    /// `count` zero bits.
    #[inline]
    pub fn zeros(&mut self, mut count: u32) {
        while count > 32 {
            self.put(0, 32);
            count -= 32;
        }
        self.put(0, count);
    }

    /// `value` in the limited-length Golomb code of T.87 A.5.3: the quotient by 2^k in
    /// unary, then k bits; past `escape` zeros, `value - 1` in `qbpp` bits instead.
    #[inline(always)]
    pub fn golomb(&mut self, value: u32, k: u32, escape: u32, qbpp: u32) {
        let quotient = value >> k;
        if quotient < escape {
            let low = (1 << k) | (value & ((1 << k) - 1));
            if quotient + k < 32 {
                self.put(low, quotient + k + 1);
            } else {
                self.zeros(quotient);
                self.put(low, k + 1);
            }
        } else {
            self.zeros(escape);
            self.put((1 << qbpp) | (value - 1), qbpp + 1);
        }
    }

    /// Pads the last byte with zeros and gives the bytes back. A last byte of 0xFF gets
    /// its stuffed byte, so the marker that follows is not read as its continuation.
    pub fn finish(mut self) -> Vec<u8> {
        self.flush_bytes();
        let width = if self.after_ff { 7 } else { 8 };
        if self.n > 0 || self.after_ff {
            let mask = (1u64 << width) - 1;
            // n <= 7, so the padded byte ends in a zero and cannot be 0xFF
            self.out.push(((self.acc << (width - self.n)) & mask) as u8);
        }
        self.out
    }
}

pub(crate) struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
    /// The next bit is the top one; below the `valid` bits everything is zero.
    acc: u64,
    valid: u32,
    /// How many of the valid bits are zeros made up past the end of the coded data.
    made_up: u64,
    after_ff: bool,
    /// The data ended, or a marker was met at `pos`.
    ended: bool,
}

impl<'a> BitReader<'a> {
    pub fn new(data: &'a [u8], pos: usize) -> BitReader<'a> {
        BitReader {
            data,
            pos,
            acc: 0,
            valid: 0,
            made_up: 0,
            after_ff: false,
            ended: false,
        }
    }

    /// After this, more than 32 bits are valid, or exactly 32.
    #[inline(always)]
    fn fill(&mut self) {
        if self.valid <= 32 {
            self.refill();
        }
    }

    #[inline(always)]
    fn refill(&mut self) {
        if !self.after_ff && !self.ended {
            if let Some(bytes) = self.data.get(self.pos..self.pos + 4) {
                let word = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                if !has_ff(word) {
                    self.acc |= u64::from(word) << (32 - self.valid);
                    self.valid += 32;
                    self.pos += 4;
                    return;
                }
            }
        }
        self.refill_bytes();
    }

    /// Byte by byte, for the bytes around an 0xFF and the end of the data, past which the
    /// bits are zeros that [`BitReader::overrun`] accounts for.
    #[cold]
    fn refill_bytes(&mut self) {
        while self.valid <= 56 {
            if self.ended {
                self.valid += 8;
                self.made_up += 8;
                continue;
            }
            match self.data.get(self.pos) {
                None => self.ended = true,
                Some(&byte) if self.after_ff => {
                    // known to be under 0x80 since the 0xFF before it was taken as data
                    self.acc |= u64::from(byte) << (57 - self.valid);
                    self.valid += 7;
                    self.pos += 1;
                    self.after_ff = false;
                }
                Some(0xFF) => match self.data.get(self.pos + 1) {
                    Some(&next) if next < 0x80 => {
                        self.acc |= 0xFF << (56 - self.valid);
                        self.valid += 8;
                        self.pos += 1;
                        self.after_ff = true;
                    }
                    // a marker, or the data stops on an 0xFF
                    _ => self.ended = true,
                },
                Some(&byte) => {
                    self.acc |= u64::from(byte) << (56 - self.valid);
                    self.valid += 8;
                    self.pos += 1;
                }
            }
        }
    }

    /// Whether bits were read past the end of the coded data.
    #[inline]
    pub fn overrun(&self) -> bool {
        u64::from(self.valid) < self.made_up
    }

    #[inline(always)]
    pub fn bit(&mut self) -> u32 {
        self.fill();
        let bit = (self.acc >> 63) as u32;
        self.acc <<= 1;
        self.valid -= 1;
        bit
    }

    /// `len` bits, at most 32.
    #[inline(always)]
    pub fn bits(&mut self, len: u32) -> u32 {
        debug_assert!(len <= 32);
        self.fill();
        // two shifts, so that a length of zero is no shift by 64
        let value = ((self.acc >> 1) >> (63 - len)) as u32;
        self.acc = (self.acc << (len - len.min(1))) << len.min(1);
        self.valid -= len;
        value
    }

    /// The value [`BitWriter::golomb`] wrote with the same `k`, `escape` and `qbpp`.
    #[inline(always)]
    pub fn golomb(&mut self, k: u32, escape: u32, qbpp: u32) -> Result<u32, Error> {
        debug_assert!(k < 32);
        self.fill();
        let zeros = self.acc.leading_zeros();
        if zeros < escape && zeros + 1 + k <= self.valid {
            let rest = (self.acc << zeros) << 1;
            let low = ((rest >> 1) >> (63 - k)) as u32;
            self.acc = rest << k;
            self.valid -= zeros + 1 + k;
            return Ok((zeros << k) | low);
        }
        self.golomb_slow(k, escape, qbpp)
    }

    /// The codes that do not fit the bits at hand: long ones, and the escape.
    #[cold]
    fn golomb_slow(&mut self, k: u32, escape: u32, qbpp: u32) -> Result<u32, Error> {
        let mut zeros = 0;
        loop {
            self.fill();
            if self.acc != 0 {
                let here = self.acc.leading_zeros();
                zeros += here;
                self.acc = (self.acc << here) << 1;
                self.valid -= here + 1;
                break;
            }
            zeros += self.valid;
            self.valid = 0;
            if zeros > escape {
                break;
            }
        }
        if zeros < escape {
            Ok((zeros << k) | self.bits(k))
        } else if zeros == escape {
            Ok(self.bits(qbpp) + 1)
        } else if self.overrun() {
            Err(Error::Truncated)
        } else {
            Err(Error::InvalidData("a code longer than the limit"))
        }
    }

    /// Where the marker that ends the coded data starts: `Truncated` when there is none.
    pub fn end(&self) -> Result<usize, Error> {
        // what was not needed to decode the last sample: its padding, a stuffed byte
        let mut pos = self.pos + usize::from(self.after_ff);
        while let Some(pair) = self.data.get(pos..pos + 2) {
            if pair[0] == 0xFF && pair[1] >= 0x80 {
                return Ok(pos);
            }
            pos += 1;
        }
        Err(Error::Truncated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn written(put: impl FnOnce(&mut BitWriter)) -> Vec<u8> {
        let mut w = BitWriter::new(Vec::new());
        put(&mut w);
        w.finish()
    }

    #[test]
    fn has_ff_finds_the_byte_wherever_it_is() {
        for word in [
            0xFF00_0000u32,
            0x00FF_0000,
            0x0000_FF00,
            0x0000_00FF,
            0xFFFF_FFFF,
        ] {
            assert!(has_ff(word), "{word:08x}");
        }
        for word in [0u32, 0xFEFE_FEFE, 0x7F80_01FE, 0x0FF0_0FF0, 0xF00F_F00F] {
            assert!(!has_ff(word), "{word:08x}");
        }
    }

    #[test]
    fn bits_are_written_most_significant_first_and_padded_with_zeros() {
        assert_eq!(written(|w| w.put(0b101, 3)), [0b1010_0000]);
        assert_eq!(
            written(|w| {
                w.put(0xAB, 8);
                w.put(0xC, 4)
            }),
            [0xAB, 0xC0]
        );
        assert_eq!(written(|_| ()), [0u8; 0]);
        assert_eq!(
            written(|w| w.put(0x1234_5678, 32)),
            [0x12, 0x34, 0x56, 0x78]
        );
    }

    #[test]
    fn the_byte_after_ff_carries_seven_bits() {
        // sixteen ones: 0xFF, then seven of them under the stuffed zero, then the last one
        assert_eq!(written(|w| w.put(0xFFFF, 16)), [0xFF, 0x7F, 0x80]);
        // a last byte of 0xFF is followed by its stuffed byte
        assert_eq!(written(|w| w.put(0xFF, 8)), [0xFF, 0x00]);
        // in the middle of a word written four bytes at a time
        assert_eq!(
            written(|w| {
                w.put(0x12FF_FFFF, 32);
                w.put(0xFFFF_FFFF, 32)
            }),
            [0x12, 0xFF, 0x7F, 0xFF, 0x7F, 0xFF, 0x7F, 0xFF, 0x70]
        );
    }

    #[test]
    fn golomb_codes_of_the_standard() {
        // 9 with k 2: quotient 2 in unary (001), then 01
        assert_eq!(written(|w| w.golomb(9, 2, 23, 8)), [0b0010_1000]);
        // k 0: the value in unary
        assert_eq!(written(|w| w.golomb(3, 0, 23, 8)), [0b0001_0000]);
        // the escape: 23 zeros, a one, then 200 - 1 in 8 bits
        assert_eq!(written(|w| w.golomb(200, 0, 23, 8)), [0, 0, 1, 199]);
        // a quotient just under the escape is still unary: 22 zeros, a one, k bits
        assert_eq!(
            written(|w| w.golomb(22 << 3 | 5, 3, 23, 8)),
            [0, 0, 0b0000_0011, 0b0100_0000]
        );
    }

    #[test]
    fn reads_back_what_was_written() {
        // every k and a spread of values at the 16-bit limits, where codes reach 64 bits
        let (escape, qbpp) = (47, 16);
        let mut values = Vec::new();
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        for i in 0..20_000u32 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let k = (seed >> 60) as u32 + i % 2;
            let magnitude = (seed >> 32) as u32 % 20;
            values.push((((seed as u32) >> 12) >> magnitude, k));
        }
        let bytes = written(|w| {
            for &(value, k) in &values {
                w.golomb(value.clamp(1, 65_536), k, escape, qbpp);
                w.put(value & 1, 1);
                w.put(value & 0x1FFF, 13);
            }
        });
        let mut r = BitReader::new(&bytes, 0);
        for &(value, k) in &values {
            assert_eq!(r.golomb(k, escape, qbpp), Ok(value.clamp(1, 65_536)));
            assert_eq!(r.bit(), value & 1);
            assert_eq!(r.bits(13), value & 0x1FFF);
        }
        assert!(!r.overrun());
    }

    #[test]
    fn reading_stops_at_a_marker_and_tells_overrun() {
        let mut bytes = written(|w| w.put(0xFFFF, 16));
        let coded = bytes.len();
        bytes.extend([0xFF, 0xD9, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA]);
        let mut r = BitReader::new(&bytes, 0);
        assert_eq!(r.bits(16), 0xFFFF);
        assert_eq!(r.end(), Ok(coded));
        // the padding of the last byte is still coded data
        assert_eq!(r.bits(7), 0);
        assert!(!r.overrun());
        // past it there is nothing: zeros, and it shows
        assert_eq!(r.bits(32), 0);
        assert!(r.overrun());
        assert_eq!(r.end(), Ok(coded));
    }

    #[test]
    fn a_stream_of_zeros_is_not_a_code() {
        let bytes = [0u8; 32];
        let mut r = BitReader::new(&bytes, 0);
        assert_eq!(
            r.golomb(0, 47, 16),
            Err(Error::InvalidData("a code longer than the limit"))
        );
        let mut r = BitReader::new(&bytes[..2], 0);
        assert_eq!(r.golomb(0, 47, 16), Err(Error::Truncated));
        assert_eq!(BitReader::new(&bytes, 0).end(), Err(Error::Truncated));
    }

    #[test]
    fn zero_length_reads_change_nothing() {
        let bytes = [0xA5u8, 0x5A];
        let mut r = BitReader::new(&bytes, 0);
        assert_eq!(r.bits(0), 0);
        assert_eq!(r.bits(8), 0xA5);
        assert_eq!(r.bits(0), 0);
        assert_eq!(r.bits(8), 0x5A);
    }
}
