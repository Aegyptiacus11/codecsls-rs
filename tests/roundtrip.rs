//! What is encoded decodes to the same samples, at the sizes and depths the limits are at.

mod common;

use common::{frame, pixels, samples, Rng};

#[test]
fn every_image_comes_back() {
    for (name, frame, pixels) in common::images() {
        let stream =
            codecsls::jpegls::encode(frame, &pixels).unwrap_or_else(|e| panic!("{name}: {e}"));
        let (got, back) =
            codecsls::jpegls::decode(&stream).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(got, frame, "{name}");
        assert!(back == pixels, "{name}: other samples than were encoded");
    }
}

#[test]
fn the_widest_and_the_tallest_images_come_back() {
    let mut rng = Rng(7);
    for (width, height) in [(65_535, 1), (1, 65_535), (65_535, 3)] {
        // flat: one run across the whole line, past the longest run code (2^15)
        for kind in ["flat", "sparse", "walk"] {
            for bits in [8, 16] {
                let image = pixels(&samples(kind, width, height, bits, &mut rng), bits);
                let stream =
                    codecsls::jpegls::encode(frame(width, height, bits), &image).expect("encodes");
                let (_, back) = codecsls::jpegls::decode(&stream).expect("decodes");
                assert!(back == image, "{kind} {width}x{height} at {bits} bits");
            }
        }
    }
}

#[test]
fn a_slice_sized_image_comes_back() {
    let mut rng = Rng(11);
    for kind in common::KINDS {
        let image = pixels(&samples(kind, 512, 512, 16, &mut rng), 16);
        let stream = codecsls::jpegls::encode(frame(512, 512, 16), &image).expect("encodes");
        let (_, back) = codecsls::jpegls::decode(&stream).expect("decodes");
        assert!(back == image, "{kind}");
    }
}

#[test]
fn noise_grows_and_still_comes_back() {
    // an image that does not compress is larger coded than raw; nothing is lost
    let mut rng = Rng(13);
    let image = pixels(&samples("noise", 200, 200, 16, &mut rng), 16);
    let stream = codecsls::jpegls::encode(frame(200, 200, 16), &image).expect("encodes");
    assert!(stream.len() > image.len());
    assert!(codecsls::jpegls::decode(&stream).expect("decodes").1 == image);
}

#[test]
fn decode_into_fills_the_callers_buffer() {
    let mut rng = Rng(17);
    let image = pixels(&samples("walk", 40, 30, 12, &mut rng), 12);
    let stream = codecsls::jpegls::encode(frame(40, 30, 12), &image).expect("encodes");
    let decoder = codecsls::jpegls::Decoder::new(&stream).expect("headers");
    assert_eq!(decoder.frame(), frame(40, 30, 12));
    assert_eq!(decoder.near(), 0);
    let mut buffer = vec![0xAA; decoder.frame().byte_len()];
    decoder.decode_into(&mut buffer).expect("decodes");
    assert!(buffer == image);
}
