//! Test images: made up, of every kind that takes a different path through the codec.
#![allow(dead_code)]

use codecsls::jpegls::FrameInfo;

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
    colour_frame(width, height, bits, 1)
}

pub fn colour_frame(width: usize, height: usize, bits: u8, components: u8) -> FrameInfo {
    FrameInfo {
        width: width as u32,
        height: height as u32,
        bits_per_sample: bits,
        components,
    }
}

/// A pixel buffer of several components: each its own image of `kind`, or all the same
/// one for "gray", where whole pixels repeat.
pub fn colour(
    kind: &str,
    width: usize,
    height: usize,
    bits: u8,
    components: usize,
    rng: &mut Rng,
) -> Vec<u8> {
    let planes: Vec<Vec<u16>> = match kind {
        "gray" => vec![samples("sparse", width, height, bits, rng); components],
        // runs of whole pixels broken by a pixel that differs in one component only
        "tint" => {
            let base = samples("flat", width, height, bits, rng);
            (0..components)
                .map(|c| {
                    let mut plane = base.clone();
                    for sample in plane.iter_mut().skip(c * 3).step_by(11 + c) {
                        *sample ^= 1;
                    }
                    plane
                })
                .collect()
        }
        _ => (0..components)
            .map(|_| samples(kind, width, height, bits, rng))
            .collect(),
    };
    let interleaved: Vec<u16> = (0..width * height)
        .flat_map(|i| planes.iter().map(move |plane| plane[i]))
        .collect();
    pixels(&interleaved, bits)
}

/// The same pixels with each component on its own, one after the other, and back.
pub fn to_planes(pixels: &[u8], frame: FrameInfo) -> Vec<u8> {
    let (n, size) = (usize::from(frame.components), frame.bytes_per_sample());
    let mut planes = Vec::with_capacity(pixels.len());
    for component in 0..n {
        for pixel in pixels.chunks_exact(n * size) {
            planes.extend(&pixel[component * size..][..size]);
        }
    }
    planes
}

pub fn from_planes(planes: &[u8], frame: FrameInfo) -> Vec<u8> {
    let (n, size) = (usize::from(frame.components), frame.bytes_per_sample());
    let plane_len = planes.len() / n;
    let mut pixels = Vec::with_capacity(planes.len());
    for at in (0..plane_len).step_by(size) {
        for component in 0..n {
            pixels.extend(&planes[component * plane_len + at..][..size]);
        }
    }
    pixels
}

pub const COLOUR_KINDS: [&str; 8] = [
    "noise", "flat", "ramp", "walk", "sparse", "stripes", "gray", "tint",
];

/// Every colour kind at some sizes and bit depths, of 2, 3 and 4 components.
pub fn colour_images() -> Vec<(String, FrameInfo, Vec<u8>)> {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut all = Vec::new();
    for bits in [2, 5, 8, 10, 12, 16] {
        for (width, height) in [(1, 1), (1, 9), (9, 1), (3, 5), (17, 33), (64, 48), (301, 7)] {
            for kind in COLOUR_KINDS {
                for components in 2..=4u8 {
                    all.push((
                        format!("{kind} {width}x{height} at {bits} bits, {components} components"),
                        colour_frame(width, height, bits, components),
                        colour(kind, width, height, bits, usize::from(components), &mut rng),
                    ));
                }
            }
        }
    }
    all
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
