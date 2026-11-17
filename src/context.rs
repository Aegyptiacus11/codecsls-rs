//! The adaptive state of a scan: one context per quantized gradient triple (T.87 A.6), two
//! for the samples that interrupt a run (A.7.2).

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
