//! Against CharLS, the reference implementation: the same bytes out of the encoder, and
//! its streams, lossless and near-lossless, decoded to the same samples.

mod common;

use charls_ref::{Image, Interleave};
use codecsls::jpegls::FrameInfo;

/// CharLS's stream for an image of one component.
fn charls_encode(frame: FrameInfo, near: i32, pixels: &[u8]) -> Vec<u8> {
    let image = Image {
        width: frame.width,
        height: frame.height,
        bits_per_sample: frame.bits_per_sample,
        components: 1,
    };
    charls_ref::encode(image, near as u8, Interleave::None, pixels)
        .unwrap_or_else(|e| panic!("CharLS does not encode: {e:?}"))
}

/// Most images must have been compared, or a test proved nothing.
fn assert_most(compared: usize, of: usize) {
    assert!(
        compared * 10 >= of * 8,
        "only {compared} of {of} images were compared"
    );
}

#[test]
fn encodes_the_bytes_charls_does() {
    let mut compared = 0;
    for (name, frame, pixels) in common::images() {
        let ours =
            codecsls::jpegls::encode(frame, &pixels).unwrap_or_else(|e| panic!("{name}: {e}"));
        let theirs = charls_encode(frame, 0, &pixels);
        assert!(
            common::same_stream(&ours, &theirs),
            "{name}: {} bytes against CharLS's {}",
            ours.len(),
            theirs.len()
        );
        compared += 1;
    }
    assert_most(compared, common::images().len());
}

#[test]
fn charls_decodes_what_we_encode() {
    for (name, frame, pixels) in common::images() {
        let ours =
            codecsls::jpegls::encode(frame, &pixels).unwrap_or_else(|e| panic!("{name}: {e}"));
        let back = charls_ref::decode(&ours).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(back == pixels, "{name}: CharLS decodes other samples");
    }
}

#[test]
fn decodes_what_charls_encodes_lossless() {
    let mut compared = 0;
    for (name, frame, pixels) in common::images() {
        let stream = charls_encode(frame, 0, &pixels);
        let (got, back) =
            codecsls::jpegls::decode(&stream).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(got, frame, "{name}");
        assert!(back == pixels, "{name}: other samples than were encoded");
        compared += 1;
    }
    assert_most(compared, common::images().len());
}

#[test]
fn decodes_what_charls_encodes_near_lossless() {
    let mut compared = 0;
    for (name, frame, pixels) in common::images() {
        let maxval = (1i32 << frame.bits_per_sample) - 1;
        for near in [1, 2, 3, 7, 30, 255] {
            if near > (maxval / 2).min(255) {
                continue;
            }
            let stream = charls_encode(frame, near, &pixels);
            compared += 1;
            let theirs = charls_ref::decode(&stream).expect("CharLS decodes its own");
            let decoder = codecsls::jpegls::Decoder::new(&stream)
                .unwrap_or_else(|e| panic!("{name} near {near}: {e}"));
            assert_eq!(i32::from(decoder.near()), near, "{name}");
            let ours = decoder
                .decode()
                .unwrap_or_else(|e| panic!("{name} near {near}: {e}"));
            assert!(
                ours == theirs,
                "{name} near {near}: not the samples CharLS decodes"
            );
            // and what both decode is within NEAR of the original
            let wide = frame.bits_per_sample > 8;
            let sample = |buffer: &[u8], i: usize| {
                if wide {
                    i32::from(u16::from_le_bytes([buffer[2 * i], buffer[2 * i + 1]]))
                } else {
                    i32::from(buffer[i])
                }
            };
            for i in 0..(frame.width * frame.height) as usize {
                let difference = (sample(&ours, i) - sample(&pixels, i)).abs();
                assert!(
                    difference <= near,
                    "{name} near {near}: sample {i} off by {difference}"
                );
            }
        }
    }
    assert!(
        compared > 3000,
        "only {compared} near-lossless streams were compared"
    );
}
