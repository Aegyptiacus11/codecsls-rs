//! The conformance streams of T.87 annex E, which this repository does not hold: they
//! come with no licence. CharLS's repository has them under `test/conformance`.
//!
//!     CODECSLS_CONFORMANCE=<that folder> cargo test --test conformance -- --ignored

use codecsls::jpegls::{decode, encode_interleaved, Decoder, Error, FrameInfo, Interleave};

fn file(name: &str) -> Vec<u8> {
    let dir = std::env::var("CODECSLS_CONFORMANCE")
        .expect("CODECSLS_CONFORMANCE names the folder of the conformance files");
    let path = std::path::Path::new(&dir).join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// A PGM or PPM file as a frame and a pixel buffer (16-bit samples are big-endian in the
/// file, little-endian in the buffer).
fn pnm(name: &str) -> (FrameInfo, Vec<u8>) {
    let data = file(name);
    let mut fields = Vec::new();
    let mut pos = 0;
    while fields.len() < 4 {
        let start = pos;
        while !data[pos].is_ascii_whitespace() {
            pos += 1;
        }
        fields.push(
            std::str::from_utf8(&data[start..pos])
                .expect("header")
                .to_string(),
        );
        pos += 1;
    }
    let components = match fields[0].as_str() {
        "P5" => 1,
        "P6" => 3,
        other => panic!("{name}: {other} is not PGM or PPM"),
    };
    let number = |i: usize| {
        fields[i]
            .parse::<u32>()
            .unwrap_or_else(|e| panic!("{name}: {e}"))
    };
    let (width, height, maxval) = (number(1), number(2), number(3));
    let bits = (32 - maxval.leading_zeros()) as u8;
    let frame = FrameInfo {
        width,
        height,
        bits_per_sample: bits,
        components,
    };
    let mut pixels = data[pos..].to_vec();
    assert_eq!(pixels.len(), frame.byte_len(), "{name}: pixel data length");
    if bits > 8 {
        pixels
            .as_chunks_mut::<2>()
            .0
            .iter_mut()
            .for_each(|sample| sample.swap(0, 1));
    }
    (frame, pixels)
}

fn largest_difference(a: &[u8], b: &[u8], frame: FrameInfo) -> i32 {
    let sample = |buffer: &[u8], i: usize| {
        if frame.bits_per_sample > 8 {
            i32::from(u16::from_le_bytes([buffer[2 * i], buffer[2 * i + 1]]))
        } else {
            i32::from(buffer[i])
        }
    };
    let count = a.len() / frame.bytes_per_sample();
    (0..count)
        .map(|i| (sample(a, i) - sample(b, i)).abs())
        .max()
        .expect("samples")
}

const COLOUR: [(&str, Interleave); 3] = [
    ("t8c0", Interleave::None),
    ("t8c1", Interleave::Line),
    ("t8c2", Interleave::Sample),
];

#[test]
#[ignore = "needs CODECSLS_CONFORMANCE"]
fn lossless_streams_decode_to_the_test_images() {
    let (frame, image) = pnm("test8.ppm");
    for (name, mode) in COLOUR {
        let stream = file(&format!("{name}e0.jls"));
        let decoder = Decoder::new(&stream).unwrap_or_else(|e| panic!("{name}e0: {e}"));
        assert_eq!(
            (decoder.frame(), decoder.interleave(), decoder.near()),
            (frame, mode, 0)
        );
        assert!(decoder.decode().expect("decodes") == image, "{name}e0");
    }
    // T1 = T2 = T3 = 9 and RESET = 31, in an LSE segment. The set's notes call it TEST8
    // interleaved by line; the stream is one component of 128 x 128, the blue one at
    // half the resolution.
    assert!(
        decode(&file("t8nde0.jls")).expect("decodes") == pnm("test8bs2.pgm"),
        "t8nde0"
    );

    // 12 bits in 16
    let (frame, image) = pnm("test16.pgm");
    assert_eq!(frame.bits_per_sample, 12);
    assert!(
        decode(&file("t16e0.jls")).expect("decodes") == (frame, image),
        "t16e0"
    );
}

#[test]
#[ignore = "needs CODECSLS_CONFORMANCE"]
fn near_lossless_streams_decode_within_three_of_the_test_images() {
    let (frame, image) = pnm("test8.ppm");
    for name in ["t8c0e3", "t8c1e3", "t8c2e3", "t8nde3"] {
        let (frame, image) = if name == "t8nde3" {
            pnm("test8bs2.pgm")
        } else {
            (frame, image.clone())
        };
        let stream = file(&format!("{name}.jls"));
        let decoder = Decoder::new(&stream).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!((decoder.frame(), decoder.near()), (frame, 3), "{name}");
        let back = decoder.decode().unwrap_or_else(|e| panic!("{name}: {e}"));
        let difference = largest_difference(&back, &image, frame);
        assert!(
            (1..=3).contains(&difference),
            "{name}: off by up to {difference}"
        );
    }
    let (frame, image) = pnm("test16.pgm");
    let (got, back) = decode(&file("t16e3.jls")).expect("decodes");
    assert_eq!(got, frame);
    assert!((1..=3).contains(&largest_difference(&back, &image, frame)));
    // and it is the decoded image the set gives for this stream
    assert!(
        back == pnm("t16e3.pgm").1,
        "t16e3 is not the reference's decoded image"
    );
}

#[test]
#[ignore = "needs CODECSLS_CONFORMANCE"]
fn subsampled_streams_are_refused() {
    for name in ["t8sse0.jls", "t8sse3.jls"] {
        assert_eq!(
            decode(&file(name)),
            Err(Error::Unsupported("subsampled components"))
        );
    }
}

#[test]
#[ignore = "needs CODECSLS_CONFORMANCE"]
fn the_encoder_writes_the_coded_data_of_the_reference_streams() {
    // headers may be laid out another way; the coded samples of each scan may not
    let coded = |stream: &[u8]| -> Vec<Vec<u8>> {
        let starts: Vec<usize> = (0..stream.len() - 1)
            .filter(|&i| stream[i..i + 2] == [0xFF, 0xDA])
            .collect();
        starts
            .iter()
            .map(|&sos| {
                let from =
                    sos + 2 + usize::from(u16::from_be_bytes([stream[sos + 2], stream[sos + 3]]));
                let len = (from..stream.len() - 1)
                    .position(|i| stream[i] == 0xFF && stream[i + 1] >= 0x80)
                    .expect("a marker ends the scan");
                stream[from..from + len].to_vec()
            })
            .collect()
    };
    let (frame, image) = pnm("test8.ppm");
    for (name, mode) in COLOUR {
        let ours = encode_interleaved(frame, mode, &image).expect("encodes");
        assert!(
            coded(&ours) == coded(&file(&format!("{name}e0.jls"))),
            "{name}e0"
        );
    }
    let (frame, image) = pnm("test16.pgm");
    let ours = encode_interleaved(frame, Interleave::None, &image).expect("encodes");
    assert!(coded(&ours) == coded(&file("t16e0.jls")), "t16e0");
}
