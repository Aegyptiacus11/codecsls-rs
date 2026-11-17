# jpegls

JPEG-LS (ITU-T T.87 | ISO/IEC 14495-1) for images of one component, in safe Rust with no
dependencies.

- **Encoder**: lossless, 2 to 16 bits per sample.
- **Decoder**: lossless and near-lossless, 2 to 16 bits per sample.
- **Samples**: one byte each up to 8 bits per sample, two in little-endian order above, row
  after row: what DICOM calls native pixel data.

```rust
use jpegls::{decode, encode, FrameInfo};

let frame = FrameInfo { width: 512, height: 512, bits_per_sample: 16 };
let pixels = vec![0u8; frame.byte_len()];
let stream = encode(frame, &pixels)?;
let (same_frame, same_pixels) = decode(&stream)?;
assert_eq!((same_frame, same_pixels), (frame, pixels));
# Ok::<(), jpegls::Error>(())
```

The size of an image comes from its stream. A caller that knows what to expect reads the
headers first and compares:

```rust
# let frame = jpegls::FrameInfo { width: 4, height: 4, bits_per_sample: 8 };
# let stream = jpegls::encode(frame, &[0; 16])?;
let decoder = jpegls::Decoder::new(&stream)?;
assert_eq!(decoder.frame(), frame);
let pixels = decoder.decode()?;
# Ok::<(), jpegls::Error>(())
```

## What it does not read

Each is refused with `Error::Unsupported`, never decoded wrongly:

- more than one component (colour);
- restart intervals;
- mapping tables and point transforms;
- dimensions above 65535 (LSE type 4), a height given after the scan (DNL);
- a RESET above 255.

## Against CharLS

CharLS is the reference implementation. The tests build it and check, on made-up images of
every bit depth from 2 to 16:

- the encoder writes the bytes CharLS writes;
- CharLS decodes what this crate encodes;
- this crate decodes what CharLS encodes, lossless and near-lossless, to the samples CharLS
  decodes.

On 172 slices of a public CT (3D-IRCADb patient 1.2, 512 x 512, signed 16-bit, 90.2 MB of
pixels), one core of a Ryzen 7 PRO 7840U, best of 5 passes, measured 2026-11-17 with
`bench/`:

| | Size | Encode per slice | Decode per slice |
|---|---|---|---|
| this crate 0.1.0 | 26.64 MB (1/3.385) | 2.74 ms | 3.13 ms |
| CharLS 2.4.5 (`charls` 0.4.2) | 26.64 MB (1/3.385) | 3.60 ms | 3.13 ms |

The 172 streams are byte for byte those of CharLS.

```sh
cargo test
cargo run --release --manifest-path bench/Cargo.toml -- <folder of DICOM slices>
```

The tests need `cmake` and a C++ compiler, for CharLS. The crate itself needs neither.

## Not checked yet

- The conformance images of T.87 annex H.
- Streams with coding parameters other than the defaults, from another encoder.
- 8-bit images from a scanner (the tests' 8-bit images are made up).

## Licence

MIT or Apache-2.0, at your option.
