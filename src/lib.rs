//! JPEG-LS (ITU-T T.87 | ISO/IEC 14495-1) for images of one component, 2 to 16 bits per
//! sample: a lossless encoder, and a decoder of lossless and near-lossless streams.
//!
//! Samples are bytes when the image has 8 bits per sample or fewer, and 16-bit
//! little-endian words above, row after row: what DICOM calls native pixel data.
#![forbid(unsafe_code)]

mod bits;
mod context;
mod error;
mod marker;
mod params;

pub use error::Error;

/// The size of an image and the precision of its samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameInfo {
    /// Samples per row, 1 to 65535.
    pub width: u32,
    /// Rows, 1 to 65535.
    pub height: u32,
    /// Bits per sample, 2 to 16.
    pub bits_per_sample: u8,
}

impl FrameInfo {
    /// Bytes a sample takes in a pixel buffer: 1 up to 8 bits, 2 above.
    pub fn bytes_per_sample(&self) -> usize {
        if self.bits_per_sample <= 8 {
            1
        } else {
            2
        }
    }

    /// Bytes the image takes in a pixel buffer.
    pub fn byte_len(&self) -> usize {
        // at most 65535 * 65535 * 2, which fits 64 bits; saturates on 32
        (self.width as usize)
            .saturating_mul(self.height as usize)
            .saturating_mul(self.bytes_per_sample())
    }

    pub(crate) fn validate(&self) -> Result<(), Error> {
        if !(1..=65_535).contains(&self.width) || !(1..=65_535).contains(&self.height) {
            return Err(Error::InvalidParameter(
                "width and height go from 1 to 65535",
            ));
        }
        if !(2..=16).contains(&self.bits_per_sample) {
            return Err(Error::InvalidParameter("bits per sample go from 2 to 16"));
        }
        Ok(())
    }
}
