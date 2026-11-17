//! Images of several components against CharLS, in the three interleave modes: the same
//! bytes out of the encoder, and each decodes the other.

mod common;

use charls::{CharLS, InterleaveMode};
use codecsls::jpegls::{FrameInfo, Interleave};

const MODES: [(Interleave, InterleaveMode); 3] = [
    (Interleave::None, InterleaveMode::None),
    (Interleave::Line, InterleaveMode::Line),
    (Interleave::Sample, InterleaveMode::Sample),
];

/// What CharLS takes and gives for a mode: planes when the stream has a scan per
/// component, pixels otherwise.
fn for_charls(pixels: &[u8], frame: FrameInfo, mode: Interleave) -> Vec<u8> {
    if mode == Interleave::None {
        common::to_planes(pixels, frame)
    } else {
        pixels.to_vec()
    }
}

fn from_charls(decoded: &[u8], frame: FrameInfo, mode: Interleave) -> Vec<u8> {
    if mode == Interleave::None {
        common::from_planes(decoded, frame)
    } else {
        decoded.to_vec()
    }
}

/// CharLS's stream, or nothing where its binding cannot make one: an image that does not
/// compress (error 3, as in `tests/charls.rs`), or two components interleaved, which
/// CharLS does not do.
fn charls_encode(frame: FrameInfo, near: i32, mode: usize, pixels: &[u8]) -> Option<Vec<u8>> {
    if frame.components == 2 && MODES[mode].0 != Interleave::None {
        return None;
    }
    let mut charls = CharLS::default();
    charls
        .set_interleave_mode(MODES[mode].1)
        .expect("CharLS takes the mode");
    let info = charls::FrameInfo {
        width: frame.width,
        height: frame.height,
        bits_per_sample: i32::from(frame.bits_per_sample),
        component_count: i32::from(frame.components),
    };
    match charls.encode(info, near, &for_charls(pixels, frame, MODES[mode].0)) {
        Ok(stream) => Some(stream),
        Err(e) if format!("{e:?}").contains("code: 3") => None,
        Err(e) => panic!("CharLS does not encode: {e:?}"),
    }
}

/// Whether CharLS decodes its own lossless stream to the image it was given.
///
/// It does not always: CharLS 2.4.5 takes a pixel of four components into a run when only
/// its fourth differs from the run by one, so the stream it writes holds another image
/// (4 of the 335 here). Those streams are no reference for the encoder; this crate's own
/// stream of such an image is exact, and CharLS decodes it exactly
/// (`charls_decodes_what_we_encode`).
fn charls_is_exact(stream: &[u8], frame: FrameInfo, mode: Interleave, pixels: &[u8]) -> bool {
    let back = CharLS::default()
        .decode(stream)
        .expect("CharLS decodes its own");
    let exact = from_charls(&back, frame, mode) == pixels;
    assert!(
        exact || (frame.components == 4 && mode == Interleave::Sample),
        "CharLS loses samples where it was not known to"
    );
    exact
}

#[test]
fn encodes_the_bytes_charls_does() {
    let mut compared = [0; 3];
    for (name, frame, pixels) in common::colour_images() {
        for (mode, (ours, _)) in MODES.iter().enumerate() {
            let stream = codecsls::jpegls::encode_interleaved(frame, *ours, &pixels)
                .unwrap_or_else(|e| panic!("{name} {ours:?}: {e}"));
            let Some(theirs) = charls_encode(frame, 0, mode, &pixels) else {
                continue;
            };
            if !charls_is_exact(&theirs, frame, *ours, &pixels) {
                continue;
            }
            assert!(
                stream == theirs,
                "{name} {ours:?}: {} bytes against CharLS's {}",
                stream.len(),
                theirs.len()
            );
            compared[mode] += 1;
        }
    }
    assert!(
        compared.iter().all(|&n| n > 300),
        "compared {compared:?} images by mode"
    );
}

#[test]
fn charls_decodes_what_we_encode() {
    for (name, frame, pixels) in common::colour_images() {
        for (ours, _) in MODES {
            if frame.components == 2 && ours != Interleave::None {
                continue;
            }
            let stream =
                codecsls::jpegls::encode_interleaved(frame, ours, &pixels).expect("encodes");
            let back = CharLS::default()
                .decode(&stream)
                .unwrap_or_else(|e| panic!("{name} {ours:?}: {e:?}"));
            assert!(
                from_charls(&back, frame, ours) == pixels,
                "{name} {ours:?}: other samples"
            );
        }
    }
}

#[test]
fn every_image_comes_back_in_every_mode() {
    // two components interleaved too, which CharLS does not check
    for (name, frame, pixels) in common::colour_images() {
        for (ours, _) in MODES {
            let stream =
                codecsls::jpegls::encode_interleaved(frame, ours, &pixels).expect("encodes");
            let decoder = codecsls::jpegls::Decoder::new(&stream).expect("headers");
            assert_eq!(
                (decoder.frame(), decoder.interleave()),
                (frame, ours),
                "{name}"
            );
            assert!(
                decoder.decode().expect("decodes") == pixels,
                "{name} {ours:?}"
            );
        }
        let default = codecsls::jpegls::encode(frame, &pixels).expect("encodes");
        assert_eq!(
            codecsls::jpegls::Decoder::new(&default)
                .expect("headers")
                .interleave(),
            Interleave::Line
        );
    }
}

#[test]
fn decodes_what_charls_encodes_lossless_and_near_lossless() {
    let mut compared = [0; 3];
    for (name, frame, pixels) in common::colour_images() {
        let maxval = (1i32 << frame.bits_per_sample) - 1;
        for near in [0, 1, 3, 30] {
            if near > (maxval / 2).min(255) {
                continue;
            }
            for (mode, (ours, _)) in MODES.iter().enumerate() {
                let Some(stream) = charls_encode(frame, near, mode, &pixels) else {
                    continue;
                };
                let theirs = CharLS::default()
                    .decode(&stream)
                    .expect("CharLS decodes its own");
                let theirs = from_charls(&theirs, frame, *ours);
                let decoder = codecsls::jpegls::Decoder::new(&stream)
                    .unwrap_or_else(|e| panic!("{name} {ours:?} near {near}: {e}"));
                assert_eq!(
                    (
                        decoder.frame(),
                        decoder.interleave(),
                        i32::from(decoder.near())
                    ),
                    (frame, *ours, near),
                    "{name}"
                );
                let back = decoder
                    .decode()
                    .unwrap_or_else(|e| panic!("{name} {ours:?} near {near}: {e}"));
                assert!(
                    back == theirs,
                    "{name} {ours:?} near {near}: not what CharLS decodes"
                );
                if near == 0 && charls_is_exact(&stream, frame, *ours, &pixels) {
                    assert!(back == pixels, "{name} {ours:?}: not what was encoded");
                }
                compared[mode] += 1;
            }
        }
    }
    assert!(
        compared.iter().all(|&n| n > 1000),
        "compared {compared:?} streams by mode"
    );
}
