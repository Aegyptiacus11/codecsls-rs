//! Test images: made up, of every kind that takes a different path through the codec.
#![allow(dead_code)]

use jpegls::FrameInfo;

/// xorshift64: the same images on every run.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    pub fn below(&mut self, n: u64) -> u64 {
        (self.next() >> 11) % n
    }
}

pub const KINDS: [&str; 7] = [
    "noise", "flat", "ramp", "walk", "sparse", "stripes", "extremes",
];

/// Samples of an image of `kind`, each at most 2^bits - 1.
pub fn samples(kind: &str, width: usize, height: usize, bits: u8, rng: &mut Rng) -> Vec<u16> {
    let maxval = (1u32 << bits) - 1;
    let mut walk = i64::from(maxval / 2);
    let mut flat = rng.below(u64::from(maxval) + 1) as u16;
    (0..width * height)
        .map(|i| {
            let (x, y) = (i % width, i / width);
            let value = match kind {
                // every code length, hardly any run
                "noise" => rng.below(u64::from(maxval) + 1) as i64,
                // runs across whole lines
                "flat" => i64::from(flat),
                // smooth, with a little noise: the regular contexts
                "ramp" => ((x * 3 + y * 5) as i64 % (i64::from(maxval) + 1)) + rng.below(3) as i64,
                // what a CT looks like: neighbours close, drifting
                "walk" => {
                    walk += rng.below(31) as i64 - 15;
                    walk
                }
                // runs broken by single samples: run interruptions of both types
                "sparse" => {
                    if rng.below(9) == 0 {
                        flat = rng.below(u64::from(maxval) + 1) as u16;
                    }
                    if rng.below(13) == 0 {
                        rng.below(u64::from(maxval) + 1) as i64
                    } else {
                        i64::from(flat)
                    }
                }
                // runs of every length up to the line, then a step
                "stripes" => (((x + y) / (1 + y % 40)) as i64 * 37) % (i64::from(maxval) + 1),
                // the largest errors there are: the escape codes
                _ => {
                    if rng.below(2) == 0 {
                        0
                    } else {
                        i64::from(maxval)
                    }
                }
            };
            value.clamp(0, i64::from(maxval)) as u16
        })
        .collect()
}

/// Samples as the bytes of a pixel buffer.
pub fn pixels(samples: &[u16], bits: u8) -> Vec<u8> {
    if bits <= 8 {
        samples.iter().map(|&s| s as u8).collect()
    } else {
        samples.iter().flat_map(|s| s.to_le_bytes()).collect()
    }
}

pub fn frame(width: usize, height: usize, bits: u8) -> FrameInfo {
    FrameInfo {
        width: width as u32,
        height: height as u32,
        bits_per_sample: bits,
    }
}

pub const SIZES: [(usize, usize); 9] = [
    (1, 1),
    (1, 9),
    (9, 1),
    (2, 2),
    (3, 5),
    (17, 33),
    (64, 64),
    (301, 7),
    (130, 90),
];

/// Every kind at every size and bit depth: (what it is, frame, pixel buffer).
pub fn images() -> Vec<(String, FrameInfo, Vec<u8>)> {
    let mut rng = Rng(0x2545_F491_4F6C_DD1D);
    let mut all = Vec::new();
    for bits in 2..=16u8 {
        for (width, height) in SIZES {
            for kind in KINDS {
                let samples = samples(kind, width, height, bits, &mut rng);
                all.push((
                    format!("{kind} {width}x{height} at {bits} bits"),
                    frame(width, height, bits),
                    pixels(&samples, bits),
                ));
            }
        }
    }
    all
}
