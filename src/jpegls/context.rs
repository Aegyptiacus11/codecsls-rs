//! The adaptive state of a scan: one context per quantized gradient triple (T.87 A.6), two
//! for the samples that interrupt a run (A.7.2).

use super::params::{Params, Quantizer};

/// What every sample of a scan is coded with, and none changes.
pub(crate) struct Coding {
    pub p: Params,
    pub quantizer: Quantizer,
    /// Zeros before the escape of a regular code.
    pub escape: u32,
    /// 2 NEAR + 1: what a coded error is worth in sample values.
    pub step: i32,
    wrap: i32,
    /// MAXVAL when the scan is lossless and RANGE a power of two, as it is unless an LSE
    /// segment set another MAXVAL: bringing a value back into range is then keeping its
    /// low bits. Zero otherwise.
    low_bits: i32,
}

impl Coding {
    pub fn new(p: Params) -> Coding {
        let step = 2 * p.near + 1;
        Coding {
            p,
            quantizer: Quantizer::new(&p),
            escape: p.limit - p.qbpp - 1,
            step,
            wrap: p.range * step,
            low_bits: if p.near == 0 && (p.maxval + 1).count_ones() == 1 {
                p.maxval
            } else {
                0
            },
        }
    }

    /// A prediction plus an error, brought back into the sample range (T.87 A.4.5, A.8).
    #[inline(always)]
    pub fn reconstruct(&self, value: i32) -> u16 {
        if self.low_bits != 0 {
            return (value & self.low_bits) as u16;
        }
        let value = if value < -self.p.near {
            value + self.wrap
        } else if value > self.p.maxval + self.p.near {
            value - self.wrap
        } else {
            value
        };
        value.clamp(0, self.p.maxval) as u16
    }
}

/// What a scan learns as it goes. The components of an interleaved scan share it.
pub(crate) struct Contexts {
    pub regular: [Regular; 365],
    pub run: [Run; 2],
}

impl Contexts {
    pub fn new(p: &Params) -> Contexts {
        let a = p.initial_a();
        Contexts {
            regular: [Regular::new(a); 365],
            run: [Run::new(a, 0), Run::new(a, 1)],
        }
    }
}

/// A component's current line and the one above it. A line holds its samples at
/// 1..=width; 0 and width + 1 are the neighbours the standard defines past the two edges
/// (T.87 A.2.1). The line above the first is zeros.
pub(crate) struct Lines {
    pub above: Vec<u16>,
    pub line: Vec<u16>,
}

impl Lines {
    pub fn new(width: usize) -> Lines {
        Lines {
            above: vec![0; width + 2],
            line: vec![0; width + 2],
        }
    }

    pub fn width(&self) -> usize {
        self.line.len() - 2
    }

    /// Before a line is coded: its left neighbour is the first sample above, and the
    /// sample past the end of the line above repeats its last.
    #[inline]
    pub fn edges(&mut self) {
        let width = self.width();
        self.line[0] = self.above[1];
        self.above[width + 1] = self.above[width];
    }

    /// The quantized gradients around the sample at `x`, as one signed number.
    #[inline(always)]
    pub fn context(&self, quantizer: &Quantizer, x: usize) -> i32 {
        let ra = i32::from(self.line[x - 1]);
        let rb = i32::from(self.above[x]);
        let rc = i32::from(self.above[x - 1]);
        let rd = i32::from(self.above[x + 1]);
        quantizer.context(rd - rb, rb - rc, rc - ra)
    }

    /// The prediction for the sample at `x` (T.87 A.4.1), before the context's correction.
    #[inline(always)]
    pub fn predict(&self, x: usize) -> i32 {
        predict(
            i32::from(self.line[x - 1]),
            i32::from(self.above[x]),
            i32::from(self.above[x - 1]),
        )
    }

    /// The line just coded becomes the line above.
    #[inline]
    pub fn advance(&mut self) {
        std::mem::swap(&mut self.above, &mut self.line);
    }
}

/// The Golomb parameter: the smallest k with `n << k >= a` (T.87 A.5.1), from the bit
/// lengths of the two instead of a loop.
#[inline(always)]
pub(crate) fn golomb_k(a: i32, n: i32) -> u32 {
    debug_assert!(a >= 0 && n >= 1);
    let (a, n) = (a as u32, n as u32);
    let k = n.leading_zeros().saturating_sub(a.leading_zeros());
    k + u32::from((n << k) < a)
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Regular {
    pub a: i32,
    pub b: i32,
    pub c: i32,
    pub n: i32,
}

impl Regular {
    pub fn new(a: i32) -> Regular {
        Regular {
            a,
            b: 0,
            c: 0,
            n: 1,
        }
    }

    #[inline(always)]
    pub fn k(&self) -> u32 {
        golomb_k(self.a, self.n)
    }

    /// All ones when the error is mapped the other way round (T.87 A.5.2): lossless
    /// coding, k of zero, and a context that leans negative.
    #[inline(always)]
    pub fn flip(&self, k: u32, lossless: bool) -> i32 {
        -i32::from(lossless & (k == 0) & (2 * self.b <= -self.n))
    }

    /// After coding the error `e` (T.87 A.6.1, A.6.2); `step` is 2 NEAR + 1.
    #[inline(always)]
    pub fn update(&mut self, e: i32, step: i32, reset: i32) {
        self.b += e * step;
        self.a += e.abs();
        if self.n == reset {
            self.a >>= 1;
            // the standard's rounding of a negative B is the arithmetic shift
            self.b >>= 1;
            self.n >>= 1;
        }
        self.n += 1;
        if self.b <= -self.n {
            self.b += self.n;
            if self.c > -128 {
                self.c -= 1;
            }
            if self.b <= -self.n {
                self.b = 1 - self.n;
            }
        } else if self.b > 0 {
            self.b -= self.n;
            if self.c < 127 {
                self.c += 1;
            }
            if self.b > 0 {
                self.b = 0;
            }
        }
    }
}

/// The context of a sample that ends a run; `ri_type` 1 when the two neighbours agree.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Run {
    pub a: i32,
    pub n: i32,
    pub nn: i32,
    pub ri_type: i32,
}

impl Run {
    pub fn new(a: i32, ri_type: i32) -> Run {
        Run {
            a,
            n: 1,
            nn: 0,
            ri_type,
        }
    }

    #[inline]
    pub fn k(&self) -> u32 {
        let temp = if self.ri_type == 1 {
            self.a + (self.n >> 1)
        } else {
            self.a
        };
        golomb_k(temp, self.n)
    }

    /// Whether a positive error takes the odd codes (T.87 A.7.2.1, the `map` of code
    /// segment A.21 for errors above zero; errors below zero take the other parity).
    #[inline]
    pub fn positive_is_odd(&self, k: u32) -> bool {
        k == 0 && 2 * self.nn < self.n
    }

    /// After coding the error `e` as the mapped value `em` (T.87 A.7.2.2).
    #[inline]
    pub fn update(&mut self, e: i32, em: i32, reset: i32) {
        if e < 0 {
            self.nn += 1;
        }
        self.a += (em + 1 - self.ri_type) >> 1;
        if self.n == reset {
            self.a >>= 1;
            self.n >>= 1;
            self.nn >>= 1;
        }
        self.n += 1;
    }
}

/// The predictor of T.87 A.4.1: the median of a, b and a + b - c.
#[inline(always)]
pub(crate) fn predict(a: i32, b: i32, c: i32) -> i32 {
    (a + b - c).clamp(a.min(b), a.max(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn golomb_k_is_the_loop_of_the_standard() {
        let by_loop = |a: i32, n: i32| {
            let mut k = 0;
            while (i64::from(n) << k) < i64::from(a) {
                k += 1;
            }
            k
        };
        for n in 1..=256 {
            for a in (0..5000).chain([65_535, 1 << 20, (1 << 25) - 1, 1 << 25, 33_000_000]) {
                assert_eq!(golomb_k(a, n), by_loop(a, n), "a {a} n {n}");
            }
        }
    }

    #[test]
    fn predictor_is_the_edge_detector_of_the_standard() {
        let by_cases = |a: i32, b: i32, c: i32| {
            if c >= a.max(b) {
                a.min(b)
            } else if c <= a.min(b) {
                a.max(b)
            } else {
                a + b - c
            }
        };
        for a in 0..12 {
            for b in 0..12 {
                for c in 0..12 {
                    assert_eq!(predict(a, b, c), by_cases(a, b, c), "{a} {b} {c}");
                }
            }
        }
    }

    #[test]
    fn regular_update_halves_at_reset_and_corrects_the_bias() {
        let mut ctx = Regular::new(4);
        ctx.update(5, 1, 64);
        // B 5 > 0: C goes up, B -= N (2) leaves 3, still above zero, so 0
        assert_eq!((ctx.a, ctx.b, ctx.c, ctx.n), (9, 0, 1, 2));
        ctx.update(-7, 1, 64);
        // B -7 <= -N (3): C goes down, B += 3 leaves -4, still <= -3, so 1 - N
        assert_eq!((ctx.a, ctx.b, ctx.c, ctx.n), (16, -2, 0, 3));

        let mut ctx = Regular {
            a: 100,
            b: -3,
            c: 0,
            n: 64,
        };
        ctx.update(0, 1, 64);
        // halved: A 50, B floor(-3 / 2) = -2, N 32 then 33
        assert_eq!((ctx.a, ctx.b, ctx.n), (50, -2, 33));

        let mut ctx = Regular {
            a: 4,
            b: 0,
            c: 127,
            n: 1,
        };
        ctx.update(9, 1, 64);
        assert_eq!(ctx.c, 127, "C stops at 127");
        let mut ctx = Regular {
            a: 4,
            b: 0,
            c: -128,
            n: 1,
        };
        ctx.update(-9, 1, 64);
        assert_eq!(ctx.c, -128, "C stops at -128");
    }

    #[test]
    fn run_update_counts_negative_errors() {
        let mut ctx = Run::new(4, 0);
        ctx.update(-2, 3, 64);
        assert_eq!((ctx.a, ctx.n, ctx.nn), (6, 2, 1));
        let mut ctx = Run {
            a: 9,
            n: 64,
            nn: 5,
            ri_type: 1,
        };
        ctx.update(1, 1, 64);
        // A 9 + (1 + 1 - 1 >> 1) = 9, then halved with N and Nn
        assert_eq!((ctx.a, ctx.n, ctx.nn), (4, 33, 2));
        assert_eq!(
            Run {
                a: 4,
                n: 3,
                nn: 0,
                ri_type: 1
            }
            .k(),
            1,
            "A + (N >> 1) = 5 > 3"
        );
        assert_eq!(
            Run {
                a: 4,
                n: 3,
                nn: 0,
                ri_type: 0
            }
            .k(),
            1
        );
        assert_eq!(
            Run {
                a: 0,
                n: 3,
                nn: 0,
                ri_type: 0
            }
            .k(),
            0
        );
    }
}
