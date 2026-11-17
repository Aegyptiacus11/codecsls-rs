//! What is refused, and that nothing a stream holds makes the decoder panic.

mod common;

use common::{frame, pixels, samples, Rng};
use jpegls::{decode, encode, Decoder, Error, FrameInfo};

fn stream(kind: &str, width: usize, height: usize, bits: u8) -> (Vec<u8>, Vec<u8>) {
    let image = pixels(&samples(kind, width, height, bits, &mut Rng(23)), bits);
    (
        encode(frame(width, height, bits), &image).expect("encodes"),
        image,
    )
}

/// The stream with the segment `marker, body` put before the first segment that starts
/// with `before`.
fn with_segment(stream: &[u8], before: u8, marker: u8, body: &[u8]) -> Vec<u8> {
    let at = stream
        .windows(2)
        .position(|w| w == [0xFF, before])
        .expect("the marker is there");
    let mut out = stream[..at].to_vec();
    out.extend([0xFF, marker]);
    out.extend(((body.len() + 2) as u16).to_be_bytes());
    out.extend(body);
    out.extend(&stream[at..]);
    out
}

#[test]
fn encode_refuses_frames_that_cannot_be_coded() {
    let refused = |width, height, bits, len: usize| {
        encode(
            FrameInfo {
                width,
                height,
                bits_per_sample: bits,
            },
            &vec![0; len],
        )
    };
    assert!(matches!(
        refused(0, 4, 8, 0),
        Err(Error::InvalidParameter(_))
    ));
    assert!(matches!(
        refused(4, 0, 8, 0),
        Err(Error::InvalidParameter(_))
    ));
    assert!(matches!(
        refused(65_536, 1, 8, 65_536),
        Err(Error::InvalidParameter(_))
    ));
    assert!(matches!(
        refused(4, 4, 1, 16),
        Err(Error::InvalidParameter(_))
    ));
    assert!(matches!(
        refused(4, 4, 17, 32),
        Err(Error::InvalidParameter(_))
    ));
    // the buffer is not width x height samples
    assert!(matches!(
        refused(4, 4, 8, 15),
        Err(Error::InvalidParameter(_))
    ));
    assert!(matches!(
        refused(4, 4, 8, 32),
        Err(Error::InvalidParameter(_))
    ));
    assert!(matches!(
        refused(4, 4, 12, 16),
        Err(Error::InvalidParameter(_))
    ));
    assert!(refused(4, 4, 12, 32).is_ok());
}

#[test]
fn encode_refuses_samples_above_the_bit_depth() {
    let mut image = vec![0u8; 16];
    image[9] = 32;
    assert_eq!(encode(frame(4, 4, 5), &image), Err(Error::SampleOutOfRange));
    image[9] = 31;
    assert!(encode(frame(4, 4, 5), &image).is_ok());

    let mut image = vec![0u8; 32];
    image[31] = 0x10; // 4096 in the last sample
    assert_eq!(
        encode(frame(4, 4, 12), &image),
        Err(Error::SampleOutOfRange)
    );
    image[31] = 0x0F;
    assert!(encode(frame(4, 4, 12), &image).is_ok());
}

#[test]
fn decode_refuses_what_is_not_a_jpeg_ls_stream() {
    assert_eq!(decode(&[]), Err(Error::Truncated));
    assert_eq!(decode(&[0xFF]), Err(Error::Truncated));
    assert_eq!(decode(&[0xFF, 0xD8]), Err(Error::Truncated));
    assert!(matches!(
        decode(b"DICM not an image"),
        Err(Error::InvalidData(_))
    ));
    assert!(matches!(
        decode(&[0xFF, 0xD8, 0x00, 0x00]),
        Err(Error::InvalidData(_))
    ));
    assert!(matches!(
        decode(&[0xFF, 0xD8, 0xFF, 0xD9]),
        Err(Error::InvalidData(_))
    ));
    assert!(matches!(
        decode(&[0xFF, 0xD8, 0xFF, 0xD8]),
        Err(Error::InvalidData(_))
    ));
    // a baseline JPEG: its frame header is not the JPEG-LS one
    let jpeg = [0xFF, 0xD8, 0xFF, 0xC0, 0, 11, 8, 0, 4, 0, 4, 1, 1, 0x11, 0];
    assert!(matches!(decode(&jpeg), Err(Error::Unsupported(_))));
    // a scan with no frame header before it
    let scan_first = [0xFF, 0xD8, 0xFF, 0xDA, 0, 8, 1, 1, 0, 0, 0, 0];
    assert!(matches!(decode(&scan_first), Err(Error::InvalidData(_))));
}

#[test]
fn decode_refuses_the_streams_it_does_not_read() {
    let (stream, _) = stream("walk", 8, 8, 8);
    let sof = stream
        .windows(2)
        .position(|w| w == [0xFF, 0xF7])
        .expect("frame header");
    let sos = stream
        .windows(2)
        .position(|w| w == [0xFF, 0xDA])
        .expect("scan header");

    // three components
    let mut colour = stream[..sof].to_vec();
    colour.extend([
        0xFF, 0xF7, 0, 17, 8, 0, 8, 0, 8, 3, 1, 0x11, 0, 2, 0x11, 0, 3, 0x11, 0,
    ]);
    colour.extend(&stream[sos..]);
    assert_eq!(
        decode(&colour),
        Err(Error::Unsupported("more than one component"))
    );

    // a restart interval of 4 lines
    let restart = with_segment(&stream, 0xDA, 0xDD, &[0, 4]);
    assert_eq!(
        decode(&restart),
        Err(Error::Unsupported("restart intervals"))
    );
    // and one of zero, which is none
    assert!(decode(&with_segment(&stream, 0xDA, 0xDD, &[0, 0])).is_ok());

    // a mapping table, a point transform, an interleave mode that does not exist
    for (at, value, want) in [
        (sos + 6, 1, Error::Unsupported("a mapping table")),
        (sos + 9, 1, Error::Unsupported("a point transform")),
        (sos + 8, 3, Error::InvalidData("interleave mode")),
    ] {
        let mut changed = stream.clone();
        changed[at] = value;
        assert_eq!(decode(&changed), Err(want));
    }

    // a frame of 1 or 17 bits, of no width, of a height left for later
    for (at, value) in [(sof + 4, 1), (sof + 4, 17), (sof + 8, 0)] {
        let mut changed = stream.clone();
        changed[at] = value;
        assert!(
            matches!(decode(&changed), Err(Error::InvalidData(_))),
            "byte {at} = {value}"
        );
    }
    let mut changed = stream.clone();
    changed[sof + 6] = 0;
    assert!(matches!(decode(&changed), Err(Error::Unsupported(_))));

    // a second frame header
    let twice = with_segment(&stream, 0xDA, 0xF7, &[8, 0, 8, 0, 8, 1, 1, 0x11, 0]);
    assert_eq!(
        decode(&twice),
        Err(Error::InvalidData("a second frame header"))
    );
}

#[test]
fn coding_parameters_in_the_stream_are_obeyed_and_checked() {
    let (stream, image) = stream("walk", 32, 32, 8);
    let lse = |maxval: u16, t1: u16, t2: u16, t3: u16, reset: u16| {
        let mut body = vec![1];
        for value in [maxval, t1, t2, t3, reset] {
            body.extend(value.to_be_bytes());
        }
        with_segment(&stream, 0xDA, 0xF8, &body)
    };
    // the defaults written out, and zeros that mean the defaults: the same image
    assert!(decode(&lse(255, 3, 7, 21, 64)).expect("decodes").1 == image);
    assert!(decode(&lse(0, 0, 0, 0, 0)).expect("decodes").1 == image);
    // other thresholds than the encoder used: another image or an error, never a panic
    assert!(decode(&lse(255, 4, 9, 30, 64)).map_or(true, |(_, back)| back != image));
    assert!(decode(&lse(255, 3, 7, 21, 30)).map_or(true, |(_, back)| back != image));
    // parameters the standard does not allow
    for bad in [
        lse(256, 0, 0, 0, 0),
        lse(0, 9, 7, 21, 0),
        lse(0, 3, 7, 300, 0),
        lse(0, 0, 0, 0, 2),
    ] {
        assert!(matches!(decode(&bad), Err(Error::InvalidData(_))));
    }
    // NEAR above half of MAXVAL
    let sos = stream
        .windows(2)
        .position(|w| w == [0xFF, 0xDA])
        .expect("scan header");
    let mut changed = stream.clone();
    changed[sos + 7] = 128;
    assert_eq!(
        decode(&changed),
        Err(Error::InvalidData("NEAR out of range"))
    );
    // segments the decoder has no use for are skipped: a comment, application data
    let commented = with_segment(&stream, 0xF7, 0xFE, b"made somewhere else");
    assert!(
        decode(&with_segment(&commented, 0xDA, 0xE8, &[0; 40]))
            .expect("decodes")
            .1
            == image
    );
}

#[test]
fn every_truncation_is_an_error() {
    for (kind, width, height, bits) in [
        ("walk", 24, 9, 8),
        ("sparse", 31, 6, 16),
        ("flat", 40, 4, 12),
    ] {
        let (stream, _) = stream(kind, width, height, bits);
        for len in 0..stream.len() {
            let result = decode(&stream[..len]);
            assert!(
                result.is_err(),
                "{kind}: the first {len} of {} bytes decode",
                stream.len()
            );
        }
        // without its end of image, and with something else in its place
        let cut = &stream[..stream.len() - 2];
        assert_eq!(decode(cut), Err(Error::Truncated));
        let mut other = cut.to_vec();
        other.extend([0xFF, 0xDA]);
        assert!(matches!(decode(&other), Err(Error::InvalidData(_))));
    }
}

#[test]
fn what_follows_the_end_of_image_is_ignored() {
    // DICOM pads a fragment to an even length
    let (mut stream, image) = stream("walk", 24, 9, 8);
    stream.push(0);
    assert!(decode(&stream).expect("decodes").1 == image);
}

#[test]
fn decode_into_refuses_a_buffer_of_another_size() {
    let (stream, _) = stream("walk", 8, 8, 16);
    let decoder = Decoder::new(&stream).expect("headers");
    for len in [0, 127, 129, 64] {
        let mut buffer = vec![0; len];
        assert!(matches!(
            decoder.decode_into(&mut buffer),
            Err(Error::InvalidParameter(_))
        ));
    }
}

#[test]
fn a_stream_that_claims_a_huge_image_fails_without_decoding_it() {
    // 65535 x 65535 at 16 bits and a few bytes of data: the size is there to see before
    // any memory is asked for, and decoding stops where the data does
    let (stream, _) = stream("flat", 8, 8, 16);
    let sof = stream
        .windows(2)
        .position(|w| w == [0xFF, 0xF7])
        .expect("frame header");
    let mut huge = stream.clone();
    huge[sof + 5..sof + 9].copy_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF]);
    let decoder = Decoder::new(&huge).expect("headers");
    assert_eq!(decoder.frame().byte_len(), 65_535 * 65_535 * 2);
    let mut small = vec![0; 128];
    assert!(matches!(
        decoder.decode_into(&mut small),
        Err(Error::InvalidParameter(_))
    ));
    // the buffer `decode` makes grows row by row: 17 GB are never asked for
    assert_eq!(decoder.decode(), Err(Error::Truncated));
    assert_eq!(decode(&huge), Err(Error::Truncated));
}

#[test]
fn no_damaged_stream_makes_the_decoder_panic() {
    // every byte of small streams set to values that matter, then random damage to
    // larger ones: an error or other samples, and no panic (overflow checks are on here)
    for (kind, width, height, bits) in [
        ("walk", 12, 5, 8),
        ("sparse", 14, 4, 16),
        ("stripes", 20, 3, 2),
        ("extremes", 9, 4, 12),
    ] {
        let (stream, _) = stream(kind, width, height, bits);
        for at in 0..stream.len() {
            for value in [
                0x00,
                0x01,
                0x7F,
                0x80,
                0xFE,
                0xFF,
                stream[at] ^ 0x10,
                stream[at] ^ 0x01,
            ] {
                let mut damaged = stream.clone();
                damaged[at] = value;
                let _ = decode(&damaged);
            }
        }
    }
    let mut rng = Rng(29);
    for round in 0..3000 {
        let kind = common::KINDS[round % common::KINDS.len()];
        let bits = [2, 5, 8, 10, 12, 16][round % 6];
        let (width, height) = (1 + rng.below(70) as usize, 1 + rng.below(30) as usize);
        let image = pixels(&samples(kind, width, height, bits, &mut rng), bits);
        let mut damaged = encode(frame(width, height, bits), &image).expect("encodes");
        for _ in 0..1 + rng.below(4) {
            let at = rng.below(damaged.len() as u64) as usize;
            match rng.below(4) {
                0 => damaged[at] = rng.below(256) as u8,
                1 => damaged[at] ^= 1 << rng.below(8),
                2 => {
                    damaged.remove(at);
                }
                _ => damaged.insert(at, rng.below(256) as u8),
            }
        }
        // a claimed size the damage made huge is refused here, as a caller would
        if let Ok(decoder) = Decoder::new(&damaged) {
            if decoder.frame().byte_len() <= 1 << 22 {
                let _ = decoder.decode();
            }
        }
    }
}
