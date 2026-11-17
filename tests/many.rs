//! Batches on several threads give what one image at a time gives, errors included.

mod common;

use codecsls::jpegls::{
    decode, decode_many, decode_many_with_threads, encode, encode_many, encode_many_with_threads,
    Error, MANY_THREADS,
};
use common::{colour, colour_frame, frame, pixels, samples, Rng};

const THREADS: [usize; 5] = [1, 2, 3, 8, 200];

#[test]
fn a_batch_is_each_image_on_its_own() {
    let mut rng = Rng(41);
    let gray = frame(64, 48, 16);
    let images: Vec<Vec<u8>> = (0..37)
        .map(|i| {
            pixels(
                &samples(common::KINDS[i % common::KINDS.len()], 64, 48, 16, &mut rng),
                16,
            )
        })
        .collect();
    let streams: Vec<Vec<u8>> = images
        .iter()
        .map(|i| encode(gray, i).expect("encodes"))
        .collect();
    for threads in THREADS {
        assert!(
            encode_many_with_threads(gray, &images, threads).expect("encodes") == streams,
            "{threads} threads"
        );
        let back = decode_many_with_threads(&streams, threads).expect("decodes");
        assert_eq!(back.len(), images.len());
        assert!(back
            .iter()
            .zip(&images)
            .all(|((f, p), image)| *f == gray && p == image));
    }
    // slices of slices as well as vectors of vectors
    let borrowed: Vec<&[u8]> = images.iter().map(Vec::as_slice).collect();
    assert!(encode_many_with_threads(gray, &borrowed, 4).expect("encodes") == streams);
}

#[test]
fn colour_goes_through_a_batch_too() {
    let mut rng = Rng(43);
    let rgb = colour_frame(31, 17, 8, 3);
    let images: Vec<Vec<u8>> = (0..9)
        .map(|_| colour("walk", 31, 17, 8, 3, &mut rng))
        .collect();
    let streams = encode_many_with_threads(rgb, &images, 4).expect("encodes");
    assert!(streams
        .iter()
        .zip(&images)
        .all(|(s, i)| *s == encode(rgb, i).expect("encodes")));
    assert!(decode_many_with_threads(&streams, 4)
        .expect("decodes")
        .into_iter()
        .map(|(_, p)| p)
        .eq(images));
}

#[test]
fn streams_of_a_batch_need_not_be_alike() {
    let mut rng = Rng(47);
    let mut expected = Vec::new();
    let mut streams = Vec::new();
    for (i, (width, height)) in common::SIZES.iter().copied().enumerate() {
        let bits = [8, 12, 16][i % 3];
        let image = pixels(&samples("walk", width, height, bits, &mut rng), bits);
        streams.push(encode(frame(width, height, bits), &image).expect("encodes"));
        expected.push((frame(width, height, bits), image));
    }
    for threads in THREADS {
        assert!(
            decode_many_with_threads(&streams, threads).expect("decodes") == expected,
            "{threads} threads"
        );
    }
}

#[test]
fn the_error_of_a_batch_is_that_of_its_first_bad_image() {
    let gray = frame(8, 8, 5);
    let mut images = vec![vec![0u8; 64]; 40];
    images[23] = vec![0; 63]; // not the size of the frame
    images[31][5] = 32; // a sample above 5 bits
    for threads in THREADS {
        let result = encode_many_with_threads(gray, &images, threads);
        assert!(
            matches!(result, Err(Error::InvalidParameter(_))),
            "{threads} threads: {result:?}"
        );
    }
    images[23] = vec![0; 64];
    for threads in THREADS {
        assert_eq!(
            encode_many_with_threads(gray, &images, threads),
            Err(Error::SampleOutOfRange)
        );
    }
    images[31][5] = 31;
    assert_eq!(
        encode_many_with_threads(gray, &images, 8)
            .expect("encodes")
            .len(),
        40
    );

    // a truncated stream before one that is no stream at all
    let good = encode(gray, &images[0]).expect("encodes");
    let mut streams = vec![good.clone(); 30];
    streams[11] = good[..good.len() - 3].to_vec();
    streams[19] = b"not JPEG-LS".to_vec();
    for threads in THREADS {
        assert_eq!(
            decode_many_with_threads(&streams, threads),
            Err(Error::Truncated),
            "{threads} threads"
        );
    }
    assert_eq!(
        decode(&streams[19]).map(|_| ()),
        Err(Error::InvalidData("no start of image"))
    );
}

#[test]
fn a_batch_wants_a_thread_and_takes_no_image() {
    let gray = frame(4, 4, 8);
    let none: Vec<Vec<u8>> = Vec::new();
    assert!(matches!(
        encode_many_with_threads(gray, &[vec![0u8; 16]], 0),
        Err(Error::InvalidParameter(_))
    ));
    assert!(matches!(
        decode_many_with_threads(&none, 0),
        Err(Error::InvalidParameter(_))
    ));
    assert_eq!(encode_many_with_threads(gray, &none, 4), Ok(vec![]));
    assert_eq!(decode_many_with_threads(&none, 4), Ok(vec![]));
}

#[test]
fn the_default_is_two_threads() {
    assert_eq!(MANY_THREADS, 2);
    let mut rng = Rng(53);
    let gray = frame(40, 30, 12);
    let images: Vec<Vec<u8>> = (0..11)
        .map(|_| pixels(&samples("walk", 40, 30, 12, &mut rng), 12))
        .collect();
    let streams = encode_many(gray, &images).expect("encodes");
    assert!(streams == encode_many_with_threads(gray, &images, 1).expect("encodes"));
    assert!(decode_many(&streams)
        .expect("decodes")
        .into_iter()
        .map(|(_, p)| p)
        .eq(images));
    // and its errors are those of the explicit form
    assert_eq!(
        encode_many(gray, &[vec![0u8; 7]]),
        Err(Error::InvalidParameter(
            "the pixel buffer is not width x height pixels"
        ))
    );
    assert_eq!(
        decode_many(&[b"no".to_vec()]),
        Err(Error::InvalidData("no start of image"))
    );
}
