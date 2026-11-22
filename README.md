# codecsls

[![crates.io](https://img.shields.io/crates/v/codecsls)](https://crates.io/crates/codecsls) [![CI](https://github.com/Aegyptiacus11/codecsls-rs/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/Aegyptiacus11/codecsls-rs/actions/workflows/ci.yml) [![Minimum Rust version](https://img.shields.io/badge/MSRV-1.88-blue)](Cargo.toml) [![Dependencies](https://deps.rs/crate/codecsls/latest/status.svg)](https://deps.rs/crate/codecsls) [![docs.rs](https://img.shields.io/docsrs/codecsls)](https://docs.rs/codecsls) [![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](#licence)

Lossless image codecs, in safe Rust with no dependencies. One so far:
JPEG-LS (ITU-T T.87 | ISO/IEC 14495-1), as `codecsls::jpegls`.

## JPEG-LS

- **Encoder**: lossless.
- **Decoder**: lossless and near-lossless.
- **Images**: 1 to 4 components (grayscale, RGB), 2 to 16 bits per sample. JPEG-LS itself
  stops at 16 bits.
- **Colour**: a scan per component, or the components interleaved by line or by sample.
- **Samples**: one byte each up to 8 bits per sample, two in little-endian order above; the
  components of a pixel together (RGBRGB...), row after row, whatever the stream's order.

```rust
use codecsls::jpegls::{decode, encode, FrameInfo};

let frame = FrameInfo { width: 512, height: 512, bits_per_sample: 16, components: 1 };
let pixels = vec![0u8; frame.byte_len()];
let stream = encode(frame, &pixels)?;
let (same_frame, same_pixels) = decode(&stream)?;
assert_eq!((same_frame, same_pixels), (frame, pixels));
# Ok::<(), codecsls::jpegls::Error>(())
```

The size of an image comes from its stream. A caller that knows what to expect reads the
headers first and compares; colour picks its interleave mode with `encode_interleaved`:

```rust
use codecsls::jpegls::{encode_interleaved, Decoder, FrameInfo, Interleave};

let frame = FrameInfo { width: 4, height: 4, bits_per_sample: 8, components: 3 };
let rgb = vec![0u8; frame.byte_len()];
let stream = encode_interleaved(frame, Interleave::Sample, &rgb)?;
let decoder = Decoder::new(&stream)?;
assert_eq!((decoder.frame(), decoder.interleave()), (frame, Interleave::Sample));
assert_eq!(decoder.decode()?, rgb);
# Ok::<(), codecsls::jpegls::Error>(())
```

Many images at once, an image per thread. Two threads unless the caller gives a number,
since not every machine has more; 1 is the calling thread alone. Results keep the order
of the images, and the error of a batch is that of its first bad image:

```rust
use codecsls::jpegls::{decode_many, encode_many, encode_many_with_threads, FrameInfo};

let frame = FrameInfo { width: 64, height: 64, bits_per_sample: 12, components: 1 };
let slices = vec![vec![0u8; frame.byte_len()]; 40];
let streams = encode_many(frame, &slices)?;
assert_eq!(streams, encode_many_with_threads(frame, &slices, 8)?);
let back = decode_many(&streams)?;
assert!(back.iter().zip(&slices).all(|((f, pixels), slice)| *f == frame && pixels == slice));
# Ok::<(), codecsls::jpegls::Error>(())
```

### What it does not read

Each is refused with `Error::Unsupported`, never decoded wrongly:

- more than four components, subsampled components, a scan of part of the components;
- the colour transforms of the HP encoders (an APP8 `mrfx` segment), which are not in the
  standard;
- restart intervals;
- mapping tables and point transforms;
- dimensions above 65535 (LSE type 4), a height given after the scan (DNL);
- a RESET above 255.

### Against CharLS

CharLS is the reference implementation. The tests build it and check, on made-up images of
every bit depth from 2 to 16, gray and of 2 to 4 components in the three interleave modes:

- the encoder writes the bytes CharLS writes;
- CharLS decodes what this crate encodes;
- this crate decodes what CharLS encodes, lossless and near-lossless, to the samples CharLS
  decodes.

The CharLS the tests build is release 2.4.5, the newest, pinned as a submodule under
`charls-ref/`. The suite also passes against CharLS's main branch (3.0.0, not released,
2026-11-14):

```sh
git -C charls-ref/charls fetch origin main && git -C charls-ref/charls checkout FETCH_HEAD
cargo test
```

Where CharLS differs from this crate:

- **The parameters above 12 bits.** This crate writes the coding parameters out in an LSE
  segment above 12 bits, as CharLS 2 does. CharLS 3.0.0 leaves those 15 bytes out; against
  it the streams are compared without them, and are the same.
- **Four components by sample.** The lossless stream CharLS 2.4.5 writes for an 8-bit image
  of 4 components interleaved by sample can hold another image: samples of the fourth
  component come back changed (4 of the 335 such test images, 13 to 168 samples each).
  CharLS 3.0.0 is exact on them, and writes the bytes this crate does. The streams CharLS
  gets wrong are left out of the byte comparison.
- **The size of the pixel buffer.** CharLS 2.4.5 takes a pixel buffer one byte short of
  the frame without an error; here it is one.

On 172 slices of a public CT (3D-IRCADb patient 1.2, 512 x 512, signed 16-bit, 90.2 MB of
pixels), one core of a Ryzen 7 PRO 7840U, best of 5 passes, measured 2026-11-17 with
`bench/`:

| | Size | Encode per slice | Decode per slice |
|---|---|---|---|
| this crate 0.1.0 | 26.64 MB (1/3.385) | 2.48 ms | 2.74 ms |
| CharLS 2.4.5 | 26.64 MB (1/3.385) | 3.68 ms | 3.13 ms |

The 172 streams are byte for byte those of CharLS.

`encode` and `decode` run on the calling thread. For many images there are `encode_many`
and `decode_many`, which code an image per thread: on two threads, or on as many as the
caller says. The same study as one batch (8 cores, 16 threads):

| Threads | 1 | 2 (the default) | 4 | 8 | 16 |
|---|---|---|---|---|---|
| encoding | 440 ms | 225 ms | 119 ms | 71 ms | 51 ms |
| decoding | 489 ms | 248 ms | 132 ms | 81 ms | 56 ms |

The same check on real 8-bit images, the masks of the same case (172 slices each, 512 x
512), where nearly everything is runs:

| | Size | Encode per slice, this crate / CharLS | Decode per slice, this crate / CharLS |
|---|---|---|---|
| liver mask, 8-bit | 1/646 | 0.08 / 0.08 ms | 0.13 / 0.12 ms |
| labelled slices, 16-bit | 1/192 | 0.13 / 0.15 ms | 0.29 / 0.26 ms |

Byte for byte CharLS's again, and exact both ways.

```sh
git clone --recurse-submodules https://github.com/Aegyptiacus11/codecsls-rs
cargo test
cargo run --release --manifest-path bench/Cargo.toml -- <folder of DICOM slices>
```

The tests need `cmake` and a C++ compiler, for CharLS. The crate itself needs neither.

### The conformance set of the standard

T.87 annex E comes with reference streams and the images they hold. All of them are
checked, with `tests/conformance.rs`:

- the lossless streams decode to the test images: colour in the three interleave modes,
  12-bit gray, and the stream with other coding parameters than the defaults (T1 = T2 = T3
  = 9, RESET = 31);
- the near-lossless streams decode within 3 of them, and the 12-bit one to the decoded
  image the set gives;
- the encoder writes the coded data of the reference's lossless streams, scan for scan;
- the two streams with subsampled components are refused.

The files carry no licence, so they are not in this repository itself. CharLS's
repository has them, and it is here as a submodule:

```sh
CODECSLS_CONFORMANCE=charls-ref/charls/test/conformance cargo test --test conformance -- --ignored
```

### Not checked yet

- Colour images from a camera or a scanner (those of the tests are made up, and the
  conformance set has one).
- Speed on colour.

## Licence

MIT or Apache-2.0, at your option.
