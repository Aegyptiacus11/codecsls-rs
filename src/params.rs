//! The coding parameters of a scan (T.87 A.1, C.2.4.1.1) and the gradient quantizer (A.3.3).

use crate::Error;

/// Run-length code orders (T.87 A.2.1, table J).
pub(crate) const J: [u32; 32] = [
    0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 9, 10, 11, 12, 13,
    14, 15,
];

pub(crate) const DEFAULT_RESET: i32 = 64;

/// What an LSE segment of type 1 carries; zero means "the default".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Presets {
    pub maxval: i32,
    pub t1: i32,
    pub t2: i32,
    pub t3: i32,
    pub reset: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Params {
    pub maxval: i32,
    pub near: i32,
    pub range: i32,
    pub qbpp: u32,
    pub limit: u32,
    pub t1: i32,
    pub t2: i32,
    pub t3: i32,
    pub reset: i32,
}

/// ceil(log2(n)) for n >= 1.
fn ceil_log2(n: i32) -> u32 {
    debug_assert!(n >= 1);
    32 - ((n - 1) as u32).leading_zeros()
}

/// CLAMP of T.87 C.2.4.1.1.1: out of [low, maxval] gives low.
fn clamp(i: i32, low: i32, maxval: i32) -> i32 {
    if i > maxval || i < low {
        low
    } else {
        i
    }
}

/// The default thresholds T1, T2, T3 for MAXVAL and NEAR (T.87 C.2.4.1.1.1).
pub(crate) fn default_thresholds(maxval: i32, near: i32) -> (i32, i32, i32) {
    const BASIC: (i32, i32, i32) = (3, 7, 21);
    if maxval >= 128 {
        let factor = (maxval.min(4095) + 128) / 256;
        let t1 = clamp(factor * (BASIC.0 - 2) + 2 + 3 * near, near + 1, maxval);
        let t2 = clamp(factor * (BASIC.1 - 3) + 3 + 5 * near, t1, maxval);
        let t3 = clamp(factor * (BASIC.2 - 4) + 4 + 7 * near, t2, maxval);
        (t1, t2, t3)
    } else {
        let factor = 256 / (maxval + 1);
        let t1 = clamp((BASIC.0 / factor + 3 * near).max(2), near + 1, maxval);
        let t2 = clamp((BASIC.1 / factor + 5 * near).max(3), t1, maxval);
        let t3 = clamp((BASIC.2 / factor + 7 * near).max(4), t2, maxval);
        (t1, t2, t3)
    }
}

impl Params {
    /// The parameters of a scan of `bits` per sample coded with `near`, with what an LSE
    /// segment set. Refuses what the standard does, and a RESET above 255: the counters
    /// here are sized for the values that are used.
    pub fn new(bits: u8, near: i32, presets: Presets) -> Result<Params, Error> {
        debug_assert!((2..=16).contains(&bits));
        let full = (1i32 << bits) - 1;
        let maxval = match presets.maxval {
            0 => full,
            m if (1..=full).contains(&m) => m,
            _ => {
                return Err(Error::InvalidData(
                    "MAXVAL larger than the sample precision",
                ))
            }
        };
        if !(0..=(maxval / 2).min(255)).contains(&near) {
            return Err(Error::InvalidData("NEAR out of range"));
        }
        let (d1, d2, d3) = default_thresholds(maxval, near);
        let pick = |given: i32, default: i32| if given == 0 { default } else { given };
        let (t1, t2, t3) = (
            pick(presets.t1, d1),
            pick(presets.t2, d2),
            pick(presets.t3, d3),
        );
        if !(near < t1 && t1 <= t2 && t2 <= t3 && t3 <= maxval) {
            return Err(Error::InvalidData("thresholds out of order"));
        }
        let reset = pick(presets.reset, DEFAULT_RESET);
        if reset < 3 || reset > maxval.max(255) {
            return Err(Error::InvalidData("RESET out of range"));
        }
        if reset > 255 {
            return Err(Error::Unsupported("RESET above 255"));
        }
        let range = (maxval + 2 * near) / (2 * near + 1) + 1;
        let bpp = ceil_log2(maxval + 1).max(2);
        Ok(Params {
            maxval,
            near,
            range,
            qbpp: ceil_log2(range),
            limit: 2 * (bpp + bpp.max(8)),
            t1,
            t2,
            t3,
            reset,
        })
    }

    /// What A[Q] starts at (T.87 A.2.1).
    pub fn initial_a(&self) -> i32 {
        ((self.range + 32) / 64).max(2)
    }
}

/// The gradient quantizer of T.87 A.3.3 as a table over [-T3, T3]: a gradient beyond T3
/// quantizes as T3 does, so the table stays a few hundred bytes whatever the bit depth.
pub(crate) struct Quantizer {
    table: Vec<i8>,
    t3: i32,
}

impl Quantizer {
    pub fn new(p: &Params) -> Quantizer {
        let quantize = |d: i32| -> i8 {
            if d <= -p.t3 {
                -4
            } else if d <= -p.t2 {
                -3
            } else if d <= -p.t1 {
                -2
            } else if d < -p.near {
                -1
            } else if d <= p.near {
                0
            } else if d < p.t1 {
                1
            } else if d < p.t2 {
                2
            } else if d < p.t3 {
                3
            } else {
                4
            }
        };
        Quantizer {
            table: (-p.t3..=p.t3).map(quantize).collect(),
            t3: p.t3,
        }
    }

    #[inline(always)]
    pub fn q(&self, d: i32) -> i32 {
        i32::from(self.table[(d.clamp(-self.t3, self.t3) + self.t3) as usize])
    }

    /// The three gradients as one signed number: its magnitude is the context, its sign
    /// the sign of the first gradient that is not zero (T.87 A.3.4).
    #[inline(always)]
    pub fn context(&self, d1: i32, d2: i32, d3: i32) -> i32 {
        81 * self.q(d1) + 9 * self.q(d2) + self.q(d3)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(bits: u8, near: i32) -> Params {
        Params::new(bits, near, Presets::default()).expect("default parameters")
    }

    #[test]
    fn default_thresholds_of_the_standard() {
        // T.87 C.2.4.1.1.1: 3, 7, 21 at 8 bits; 18, 67, 276 at 12 bits and above, where
        // FACTOR stops growing.
        assert_eq!(default_thresholds(255, 0), (3, 7, 21));
        assert_eq!(default_thresholds(4095, 0), (18, 67, 276));
        assert_eq!(default_thresholds(65535, 0), (18, 67, 276));
        assert_eq!(default_thresholds(255, 3), (12, 22, 42));
        // below 128 the thresholds shrink, and a value past MAXVAL falls back
        assert_eq!(default_thresholds(3, 0), (2, 3, 3));
        assert_eq!(default_thresholds(1, 0), (1, 1, 1));
    }

    #[test]
    fn derived_values() {
        let p = params(8, 0);
        assert_eq!((p.range, p.qbpp, p.limit, p.initial_a()), (256, 8, 32, 4));
        let p = params(16, 0);
        assert_eq!(
            (p.range, p.qbpp, p.limit, p.initial_a()),
            (65536, 16, 64, 1024)
        );
        let p = params(12, 0);
        assert_eq!((p.range, p.qbpp, p.limit), (4096, 12, 48));
        let p = params(8, 3);
        assert_eq!((p.range, p.qbpp), (38, 6));
        let p = params(2, 0);
        assert_eq!((p.range, p.qbpp, p.limit, p.initial_a()), (4, 2, 20, 2));
    }

    #[test]
    fn presets_replace_defaults_one_by_one() {
        let given = Presets {
            maxval: 0,
            t1: 5,
            t2: 0,
            t3: 30,
            reset: 31,
        };
        let p = Params::new(8, 0, given).expect("valid presets");
        assert_eq!((p.maxval, p.t1, p.t2, p.t3, p.reset), (255, 5, 7, 30, 31));
        let p = Params::new(
            12,
            0,
            Presets {
                maxval: 1000,
                ..Presets::default()
            },
        )
        .expect("valid");
        assert_eq!((p.maxval, p.range, p.qbpp, p.limit), (1000, 1001, 10, 40));
    }

    #[test]
    fn refuses_what_cannot_be_coded() {
        let with = |presets| Params::new(8, 0, presets);
        let d = Presets::default();
        assert!(matches!(
            with(Presets { maxval: 256, ..d }),
            Err(Error::InvalidData(_))
        ));
        assert!(matches!(
            with(Presets { t1: 8, ..d }),
            Err(Error::InvalidData(_))
        ));
        assert!(matches!(
            with(Presets { t3: 256, ..d }),
            Err(Error::InvalidData(_))
        ));
        assert!(matches!(
            with(Presets { reset: 2, ..d }),
            Err(Error::InvalidData(_))
        ));
        assert!(matches!(
            with(Presets { reset: 256, ..d }),
            Err(Error::InvalidData(_))
        ));
        assert!(matches!(
            Params::new(16, 0, Presets { reset: 256, ..d }),
            Err(Error::Unsupported(_))
        ));
        assert!(matches!(Params::new(8, 128, d), Err(Error::InvalidData(_))));
        assert!(matches!(Params::new(8, -1, d), Err(Error::InvalidData(_))));
        assert!(Params::new(8, 127, d).is_ok());
    }

    #[test]
    fn quantizer_follows_the_thresholds() {
        let q = Quantizer::new(&params(8, 0));
        let expected = [
            (-300, -4),
            (-21, -4),
            (-20, -3),
            (-7, -3),
            (-6, -2),
            (-3, -2),
            (-2, -1),
            (-1, -1),
            (0, 0),
            (1, 1),
            (2, 1),
            (3, 2),
            (6, 2),
            (7, 3),
            (20, 3),
            (21, 4),
            (300, 4),
        ];
        for (d, want) in expected {
            assert_eq!(q.q(d), want, "gradient {d}");
        }
        let q = Quantizer::new(&params(8, 2));
        assert_eq!((q.q(-3), q.q(-2), q.q(2), q.q(3)), (-1, 0, 0, 1));
    }

    #[test]
    fn context_sign_is_the_first_gradient_that_is_not_zero() {
        let q = Quantizer::new(&params(8, 0));
        assert_eq!(q.context(0, 0, 0), 0);
        assert_eq!(q.context(21, 21, 21), 364);
        assert_eq!(q.context(-21, -21, -21), -364);
        assert!(q.context(0, -1, 21) < 0);
        assert!(q.context(1, -21, -21) > 0);
    }
}
