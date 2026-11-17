//! Lossless codecs for medical images, in safe Rust with no dependencies. One so far:
//!
//! - [`jpegls`]: JPEG-LS (ITU-T T.87 | ISO/IEC 14495-1).
#![forbid(unsafe_code)]

pub mod jpegls;

/// The README's examples, compiled and run with the tests.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct Readme;
