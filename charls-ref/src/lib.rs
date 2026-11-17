//! CharLS through its C interface: encode, decode, and which version was built.
//!
//! Pixel buffers are CharLS's own: planes one after the other for a scan per component,
//! the components of a pixel together otherwise.

use std::ffi::c_void;

extern crate link_cplusplus;

#[repr(C)]
struct FrameInfo {
    width: u32,
    height: u32,
    bits_per_sample: i32,
    component_count: i32,
}

#[repr(C)]
struct Encoder {
    _opaque: [u8; 0],
}

#[repr(C)]
struct Decoder {
    _opaque: [u8; 0],
}

// As declared in charls/include/charls (charls_jpegls_encoder.h, charls_jpegls_decoder.h,
// version.h); each returns a charls_jpegls_errc, 0 for success.
unsafe extern "C" {
    fn charls_get_version_number(major: *mut i32, minor: *mut i32, patch: *mut i32);

    fn charls_jpegls_encoder_create() -> *mut Encoder;
    fn charls_jpegls_encoder_destroy(encoder: *const Encoder);
    fn charls_jpegls_encoder_set_frame_info(encoder: *mut Encoder, info: *const FrameInfo) -> i32;
    fn charls_jpegls_encoder_set_near_lossless(encoder: *mut Encoder, near: i32) -> i32;
    fn charls_jpegls_encoder_set_interleave_mode(encoder: *mut Encoder, mode: i32) -> i32;
    fn charls_jpegls_encoder_set_destination_buffer(
        encoder: *mut Encoder,
        buffer: *mut c_void,
        size: usize,
    ) -> i32;
    fn charls_jpegls_encoder_encode_from_buffer(
        encoder: *mut Encoder,
        source: *const c_void,
        size: usize,
        stride: u32,
    ) -> i32;
    fn charls_jpegls_encoder_get_bytes_written(encoder: *const Encoder, written: *mut usize)
        -> i32;

    fn charls_jpegls_decoder_create() -> *mut Decoder;
    fn charls_jpegls_decoder_destroy(decoder: *const Decoder);
    fn charls_jpegls_decoder_set_source_buffer(
        decoder: *mut Decoder,
        source: *const c_void,
        size: usize,
    ) -> i32;
    fn charls_jpegls_decoder_read_header(decoder: *mut Decoder) -> i32;
    fn charls_jpegls_decoder_get_destination_size(
        decoder: *const Decoder,
        stride: u32,
        size: *mut usize,
    ) -> i32;
    fn charls_jpegls_decoder_decode_to_buffer(
        decoder: *mut Decoder,
        destination: *mut c_void,
        size: usize,
        stride: u32,
    ) -> i32;
}

/// An error code of CharLS (`charls_jpegls_errc`), with the call that gave it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Error {
    pub code: i32,
    pub call: &'static str,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CharLS error {} in {}", self.code, self.call)
    }
}

impl std::error::Error for Error {}

fn check(code: i32, call: &'static str) -> Result<(), Error> {
    if code == 0 {
        Ok(())
    } else {
        Err(Error { code, call })
    }
}

/// How a stream orders the components, as CharLS numbers the modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interleave {
    None = 0,
    Line = 1,
    Sample = 2,
}

/// The image `encode` is given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub bits_per_sample: u8,
    pub components: u8,
}

/// The version of CharLS that was built: major, minor, patch.
pub fn version() -> (i32, i32, i32) {
    let (mut major, mut minor, mut patch) = (0, 0, 0);
    // SAFETY: the three pointers are to live integers, which is all the function writes.
    unsafe { charls_get_version_number(&mut major, &mut minor, &mut patch) };
    (major, minor, patch)
}

/// CharLS's stream for `pixels`, with `near` 0 for lossless.
pub fn encode(
    image: Image,
    near: u8,
    interleave: Interleave,
    pixels: &[u8],
) -> Result<Vec<u8>, Error> {
    let info = FrameInfo {
        width: image.width,
        height: image.height,
        bits_per_sample: i32::from(image.bits_per_sample),
        component_count: i32::from(image.components),
    };
    // CharLS 2.4.5 does not check that the pixels fill the frame (it takes a buffer one
    // byte short without an error), so it is checked here.
    let bytes_per_sample = if image.bits_per_sample > 8 { 2 } else { 1 };
    let expected = image.width as usize
        * image.height as usize
        * usize::from(image.components)
        * bytes_per_sample;
    if pixels.len() != expected {
        return Err(Error {
            code: -1,
            call: "the pixel buffer is not the size of the frame",
        });
    }
    // A sample takes at most 64 bits coded (the limit at 16 bits), stuffing adds a
    // seventh, and the headers are under a hundred bytes: five times the pixels holds it.
    let mut stream = vec![0u8; pixels.len() * 5 + 1024];
    let mut written = 0;
    // SAFETY: the encoder is created here and destroyed before returning; `info`,
    // `stream` and `pixels` outlive every call that reads or writes them; `pixels` holds
    // exactly the frame CharLS is told to read, as checked above, and `stream` is passed
    // with its true length, which CharLS does check.
    unsafe {
        let encoder = charls_jpegls_encoder_create();
        assert!(!encoder.is_null(), "CharLS could not allocate an encoder");
        let result = (|| {
            check(
                charls_jpegls_encoder_set_frame_info(encoder, &info),
                "set_frame_info",
            )?;
            check(
                charls_jpegls_encoder_set_near_lossless(encoder, i32::from(near)),
                "set_near_lossless",
            )?;
            check(
                charls_jpegls_encoder_set_interleave_mode(encoder, interleave as i32),
                "set_interleave_mode",
            )?;
            check(
                charls_jpegls_encoder_set_destination_buffer(
                    encoder,
                    stream.as_mut_ptr().cast(),
                    stream.len(),
                ),
                "set_destination_buffer",
            )?;
            check(
                charls_jpegls_encoder_encode_from_buffer(
                    encoder,
                    pixels.as_ptr().cast(),
                    pixels.len(),
                    0,
                ),
                "encode_from_buffer",
            )?;
            check(
                charls_jpegls_encoder_get_bytes_written(encoder, &mut written),
                "get_bytes_written",
            )
        })();
        charls_jpegls_encoder_destroy(encoder);
        result?;
    }
    stream.truncate(written);
    Ok(stream)
}

/// The pixels CharLS decodes from `stream`.
pub fn decode(stream: &[u8]) -> Result<Vec<u8>, Error> {
    let mut pixels = Vec::new();
    // SAFETY: the decoder is created here and destroyed before returning; `stream`
    // outlives it; `pixels` is resized to the size CharLS asks for before CharLS writes
    // that many bytes into it.
    unsafe {
        let decoder = charls_jpegls_decoder_create();
        assert!(!decoder.is_null(), "CharLS could not allocate a decoder");
        let result = (|| {
            check(
                charls_jpegls_decoder_set_source_buffer(
                    decoder,
                    stream.as_ptr().cast(),
                    stream.len(),
                ),
                "set_source_buffer",
            )?;
            check(charls_jpegls_decoder_read_header(decoder), "read_header")?;
            let mut size = 0;
            check(
                charls_jpegls_decoder_get_destination_size(decoder, 0, &mut size),
                "get_destination_size",
            )?;
            pixels.resize(size, 0u8);
            check(
                charls_jpegls_decoder_decode_to_buffer(
                    decoder,
                    pixels.as_mut_ptr().cast(),
                    size,
                    0,
                ),
                "decode_to_buffer",
            )
        })();
        charls_jpegls_decoder_destroy(decoder);
        result?;
    }
    Ok(pixels)
}

#[cfg(test)]
mod tests {
    use super::*;

    const GRAY: Image = Image {
        width: 4,
        height: 2,
        bits_per_sample: 8,
        components: 1,
    };

    #[test]
    fn the_pinned_release_is_what_was_built() {
        assert_eq!(version(), (2, 4, 5));
    }

    #[test]
    fn an_image_comes_back() {
        let pixels = [1, 2, 3, 4, 250, 251, 252, 253];
        let stream = encode(GRAY, 0, Interleave::None, &pixels).expect("encodes");
        assert_eq!(&stream[..2], [0xFF, 0xD8]);
        assert_eq!(decode(&stream).expect("decodes"), pixels);
    }

    #[test]
    fn errors_come_back_as_errors() {
        // a pixel buffer shorter than the frame, a stream that is not one
        let short = encode(GRAY, 0, Interleave::None, &[0; 7]);
        assert!(matches!(short, Err(Error { code: -1, .. })), "{short:?}");
        assert!(encode(GRAY, 0, Interleave::None, &[0; 9]).is_err());
        // 12 bits take two bytes a sample
        let wide = Image {
            bits_per_sample: 12,
            ..GRAY
        };
        assert!(encode(wide, 0, Interleave::None, &[0; 8]).is_err());
        assert!(encode(wide, 0, Interleave::None, &[0; 16]).is_ok());
        let not_a_stream = decode(b"not JPEG-LS");
        assert!(
            matches!(
                not_a_stream,
                Err(Error {
                    call: "read_header",
                    ..
                })
            ),
            "{not_a_stream:?}"
        );
    }
}
