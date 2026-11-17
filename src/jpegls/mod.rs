//! JPEG-LS (ITU-T T.87 | ISO/IEC 14495-1) for images of one to four components, 2 to 16
//! bits per sample: a lossless encoder, and a decoder of lossless and near-lossless
//! streams.
//!
//! Samples are bytes when the image has 8 bits per sample or fewer, and 16-bit
//! little-endian words above. Pixels follow each other row after row, and the components
//! of a pixel follow each other (RGBRGB...), however the stream interleaves them.
mod bits;
mod context;
mod decode;
mod encode;
mod error;
mod many;
mod marker;
mod params;

pub use decode::{decode, Decoder};
pub use encode::{encode, encode_interleaved};
pub use error::Error;
pub use many::{decode_many, encode_many};

/// The size of an image and the precision of its samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameInfo {
    /// Pixels per row, 1 to 65535.
    pub width: u32,
    /// Rows, 1 to 65535.
    pub height: u32,
    /// Bits per sample, 2 to 16.
    pub bits_per_sample: u8,
    /// Samples per pixel, 1 to 4: 1 for grayscale, 3 for RGB.
    pub components: u8,
}

/// How a stream orders the samples of an image of several components (T.87 annex B).
/// Pixel buffers are the same whichever it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interleave {
    /// A scan per component: all of the first, then all of the second.
    None,
    /// One scan: a line of each component in turn.
    Line,
    /// One scan: a pixel at a time. The one that codes a run of equal pixels as a run.
    Sample,
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

    /// Bytes a row takes in a pixel buffer.
    pub fn row_len(&self) -> usize {
        self.width as usize * usize::from(self.components) * self.bytes_per_sample()
    }

    /// Bytes the image takes in a pixel buffer.
    pub fn byte_len(&self) -> usize {
        // at most 65535 * 65535 * 8, which fits 64 bits; saturates on 32
        self.row_len().saturating_mul(self.height as usize)
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
        if !(1..=4).contains(&self.components) {
            return Err(Error::InvalidParameter("components go from 1 to 4"));
        }
        Ok(())
    }
}
