//! JPEG-LS lossless on real CT slices: this crate, CharLS and two other pure-Rust codecs.
//!
//!     cargo run --release --manifest-path bench/Cargo.toml -- <folder of DICOM slices> [12]
//!
//! Every encoder's output is decoded by every decoder and compared with the source.
//! Exits 1 when any pair fails or differs.
use std::time::{Duration, Instant};

use dicom_dictionary_std::tags;

struct Slice {
    cols: u32,
    rows: u32,
    bits: u8,
    native: Vec<u8>, // little-endian 16-bit samples, as in the file
}

const NAMES: [&str; 4] = ["jpegls (this crate)", "charls", "pure_jpegls", "dicom-toolkit-codec"];

fn encode(codec: usize, s: &Slice) -> Result<Vec<u8>, String> {
    match codec {
        0 => jpegls::encode(
            jpegls::FrameInfo { width: s.cols, height: s.rows, bits_per_sample: s.bits },
            &s.native,
        )
        .map_err(|e| e.to_string()),
        1 => charls::CharLS::default()
            .encode(
                charls::FrameInfo {
                    width: s.cols,
                    height: s.rows,
                    bits_per_sample: i32::from(s.bits),
                    component_count: 1,
                },
                0,
                &s.native,
            )
            .map_err(|e| e.to_string()),
        2 => {
            let px: Vec<u16> = s.native.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            let mut opts = pure_jpegls::EncodeOptions::default();
            opts.precision = Some(s.bits);
            let mut out = Vec::new();
            pure_jpegls::encode_with_options(&px, s.cols, s.rows, &opts, &mut out).map_err(|e| e.to_string())?;
            Ok(out)
        }
        _ => dicom_toolkit_codec::jpeg_ls::JpegLsCodec::encode_frame(&s.native, s.cols, s.rows, s.bits, 1, 0)
            .map_err(|e| e.to_string()),
    }
}

fn decode(codec: usize, s: &Slice, bytes: &[u8]) -> Result<Vec<u8>, String> {
    match codec {
        0 => jpegls::decode(bytes).map(|(_, pixels)| pixels).map_err(|e| e.to_string()),
        1 => charls::CharLS::default().decode(bytes).map_err(|e| e.to_string()),
        2 => {
            let (px, _, _) = pure_jpegls::decode(bytes, s.cols, s.rows).map_err(|e| e.to_string())?;
            Ok(px.iter().flat_map(|v| v.to_le_bytes()).collect())
        }
        _ => dicom_toolkit_codec::jpeg_ls::JpegLsCodec::decode_frame(bytes)
            .map(|f| f.pixels)
            .map_err(|e| e.to_string()),
    }
}

fn load(dir: &str) -> Vec<Slice> {
    let mut paths: Vec<_> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{dir}: {e}"))
        .map(|e| e.expect("directory entry").path())
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "{dir}: no files");
    paths
        .iter()
        .map(|p| {
            let o = dicom_object::open_file(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
            let int = |t| -> u32 { o.element(t).expect("element").to_int().expect("integer") };
            let (rows, cols) = (int(tags::ROWS), int(tags::COLUMNS));
            assert_eq!(int(tags::BITS_ALLOCATED), 16, "{}: not 16 bits allocated", p.display());
            assert_eq!(int(tags::SAMPLES_PER_PIXEL), 1, "{}: not grayscale", p.display());
            let ts = o.meta().transfer_syntax().trim_end_matches('\0').to_string();
            assert!(
                ts == "1.2.840.10008.1.2" || ts == "1.2.840.10008.1.2.1",
                "{}: transfer syntax {ts} is not uncompressed little endian",
                p.display()
            );
            let native = o.element(tags::PIXEL_DATA).expect("pixel data").to_bytes().expect("bytes").into_owned();
            assert_eq!(native.len(), (rows * cols * 2) as usize, "{}: pixel data length", p.display());
            // as pacs-rs's coded_bits: Bits Stored when every value fits, else 16
            let stored = int(tags::BITS_STORED);
            let fits = stored < 16
                && native.chunks_exact(2).all(|c| u16::from_le_bytes([c[0], c[1]]) >> stored == 0);
            Slice { cols, rows, bits: if fits { stored as u8 } else { 16 }, native }
        })
        .collect()
}

fn main() {
    let dir = std::env::args().nth(1).expect("usage: jls-bench <folder of DICOM slices>");
    let mut slices = load(&dir);
    // second argument "12": the same slices as unsigned 12-bit (value + 1024, clamped), to see
    // whether a result depends on the 16-bit depth
    match std::env::args().nth(2).as_deref() {
        None => {}
        Some("12") => {
            for s in &mut slices {
                for c in s.native.chunks_exact_mut(2) {
                    let v = (i32::from(i16::from_le_bytes([c[0], c[1]])) + 1024).clamp(0, 4095) as u16;
                    c.copy_from_slice(&v.to_le_bytes());
                }
                s.bits = 12;
            }
        }
        Some(other) => panic!("unknown mode {other}"),
    }
    let raw: usize = slices.iter().map(|s| s.native.len()).sum();
    let bits: std::collections::BTreeSet<u8> = slices.iter().map(|s| s.bits).collect();
    println!("{} slices, {} x {}, coded bits {:?}, {:.1} MB of pixels", slices.len(), slices[0].cols, slices[0].rows, bits, raw as f64 / 1e6);

    const PASSES: usize = 5;
    let mut failed = false;
    let mut encoded: Vec<Option<Vec<Vec<u8>>>> = Vec::new();
    for e in 0..4 {
        let mut best = Duration::MAX;
        let mut out = None;
        for _ in 0..PASSES {
            let t = Instant::now();
            let r: Result<Vec<Vec<u8>>, String> = slices.iter().map(|s| encode(e, s)).collect();
            let dt = t.elapsed();
            match r {
                Ok(v) => {
                    best = best.min(dt);
                    out = Some(v);
                }
                Err(err) => {
                    println!("ENCODE {:<20} FAILED: {err}", NAMES[e]);
                    failed = true;
                    out = None;
                    break;
                }
            }
        }
        if let Some(v) = &out {
            let size: usize = v.iter().map(Vec::len).sum();
            println!(
                "ENCODE {:<20} {:.2} MB  ratio 1/{:.3}  {:.2} ms/slice (best of {PASSES})",
                NAMES[e], size as f64 / 1e6, raw as f64 / size as f64, best.as_secs_f64() * 1e3 / slices.len() as f64
            );
        }
        encoded.push(out);
    }
    for other in [0, 2] {
        if let (Some(a), Some(b)) = (&encoded[other], &encoded[1]) {
            let same = a.iter().zip(b).filter(|(x, y)| x == y).count();
            println!("{} and charls streams byte-identical: {same}/{}", NAMES[other], a.len());
        }
    }
    for d in 0..4 {
        for e in 0..4 {
            let Some(streams) = &encoded[e] else { continue };
            let mut best = Duration::MAX;
            let (mut exact, mut differ, mut errors) = (0usize, 0usize, 0usize);
            let mut first_error = String::new();
            for pass in 0..PASSES {
                let t = Instant::now();
                let outs: Vec<Result<Vec<u8>, String>> = slices.iter().zip(streams).map(|(s, b)| decode(d, s, b)).collect();
                best = best.min(t.elapsed());
                if pass == 0 {
                    for (s, o) in slices.iter().zip(&outs) {
                        match o {
                            Ok(px) if *px == s.native => exact += 1,
                            Ok(_) => differ += 1,
                            Err(err) => {
                                errors += 1;
                                if first_error.is_empty() {
                                    first_error = err.clone();
                                }
                            }
                        }
                    }
                }
            }
            failed |= differ + errors > 0;
            println!(
                "DECODE {:<20} of {:<20} exact {exact}/{}  differ {differ}  errors {errors}  {:.2} ms/slice {first_error}",
                NAMES[d], NAMES[e], slices.len(), best.as_secs_f64() * 1e3 / slices.len() as f64
            );
        }
    }
    if failed {
        std::process::exit(1);
    }
}
