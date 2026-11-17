use std::fmt;

/// Why an image could not be encoded or a stream decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// What the caller passed cannot be coded: a dimension or a bit depth out of range, a
    /// buffer of the wrong length.
    InvalidParameter(&'static str),
    /// A sample is larger than the bit depth allows.
    SampleOutOfRange,
    /// The stream ends before the image does.
    Truncated,
    /// The stream is not valid JPEG-LS.
    InvalidData(&'static str),
    /// The stream is valid JPEG-LS that this crate does not read.
    Unsupported(&'static str),
    /// The decoded image would not fit in memory.
    TooLarge,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::InvalidParameter(what) => write!(f, "invalid parameter: {what}"),
            Error::SampleOutOfRange => f.write_str("a sample is larger than the bit depth allows"),
            Error::Truncated => f.write_str("the JPEG-LS stream is truncated"),
            Error::InvalidData(what) => write!(f, "invalid JPEG-LS stream: {what}"),
            Error::Unsupported(what) => write!(f, "unsupported JPEG-LS stream: {what}"),
            Error::TooLarge => f.write_str("the decoded image does not fit in memory"),
        }
    }
}

impl std::error::Error for Error {}
