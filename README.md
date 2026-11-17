# codecsls

Lossless codecs for medical images, in safe Rust with no dependencies. One so far:
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

Two things CharLS 2.4.5 does that this crate does not:

- its lossless stream of a 4-component image interleaved by sample can hold another image:
  a pixel that differs from a run only by one in its fourth component is taken into the run
  (4 of the 335 such test images). Those streams are left out of the byte comparison;
- through the `charls` crate, an image that does not compress fails with "destination
  buffer too small".

On 172 slices of a public CT (3D-IRCADb patient 1.2, 512 x 512, signed 16-bit, 90.2 MB of
pixels), one core of a Ryzen 7 PRO 7840U, best of 5 passes, measured 2026-11-17 with
`bench/`:

| | Size | Encode per slice | Decode per slice |
|---|---|---|---|
| this crate 0.1.0 | 26.64 MB (1/3.385) | 2.69 ms | 3.17 ms |
| CharLS 2.4.5 (`charls` 0.4.2) | 26.64 MB (1/3.385) | 3.66 ms | 3.16 ms |

The 172 streams are byte for byte those of CharLS. Colour has not been timed.

```sh
cargo test
cargo run --release --manifest-path bench/Cargo.toml -- <folder of DICOM slices>
```

The tests need `cmake` and a C++ compiler, for CharLS. The crate itself needs neither.

### Not checked yet

- The conformance images of T.87 annex H.
- Streams with coding parameters other than the defaults, from another encoder.
- Images from a scanner at 8 bits, or in colour (those of the tests are made up).

## Licence

MIT or Apache-2.0, at your option.
