//! The decoder (T.87 annex A, any NEAR).

use crate::bits::BitReader;
use crate::context::{predict, Regular, Run};
use crate::marker::{self, Header};
use crate::params::{Quantizer, J};
use crate::{Error, FrameInfo};

/// No mapped error of a valid stream is above this; the check keeps the statistics of a
/// hostile one within 32 bits.
const MAX_MAPPED: u32 = 2 * 65_536;

/// A JPEG-LS stream whose headers were read: what the image is, before decoding it.
///
/// The size comes from the stream, so a caller that knows what to expect (the Rows and
/// Columns of a DICOM instance) compares [`Decoder::frame`] with it before decoding.
#[derive(Debug, Clone)]
pub struct Decoder<'a> {
    data: &'a [u8],
    header: Header,
}

impl<'a> Decoder<'a> {
    /// Reads the headers of `data`, up to the coded samples.
    pub fn new(data: &'a [u8]) -> Result<Decoder<'a>, Error> {
        Ok(Decoder {
            data,
            header: marker::read_header(data)?,
        })
    }

    pub fn frame(&self) -> FrameInfo {
        self.header.frame
    }

    /// The largest difference between a decoded sample and the original: 0 for a
    /// lossless stream.
    pub fn near(&self) -> u8 {
        self.header.params.near as u8
    }

    /// Decodes into `out`, which holds `self.frame().byte_len()` bytes.
    pub fn decode_into(&self, out: &mut [u8]) -> Result<(), Error> {
        let frame = self.header.frame;
        if out.len() != frame.byte_len() {
            return Err(Error::InvalidParameter(
                "the pixel buffer is not width x height samples",
            ));
        }
        let mut rows = out.chunks_exact_mut(frame.width as usize * frame.bytes_per_sample());
        scan(&self.header, self.data, |line| {
            // as many rows as the scan has lines: the length was checked
            if let Some(row) = rows.next() {
                store(line, row);
            }
            Ok(())
        })
    }

    /// Decodes into a new buffer, which grows as rows are decoded: a short stream that
    /// claims a huge image fails where its data ends, before the memory is taken.
    pub fn decode(&self) -> Result<Vec<u8>, Error> {
        let frame = self.header.frame;
        let row_len = frame.width as usize * frame.bytes_per_sample();
        let mut out = Vec::new();
        scan(&self.header, self.data, |line| {
            let filled = out.len();
            out.try_reserve(row_len).map_err(|_| Error::TooLarge)?;
            out.resize(filled + row_len, 0);
            store(line, &mut out[filled..]);
            Ok(())
        })?;
        Ok(out)
    }
}

/// Decodes a stream: the image's size and its samples, one byte each up to 8 bits per
/// sample, two in little-endian order above.
pub fn decode(data: &[u8]) -> Result<(FrameInfo, Vec<u8>), Error> {
    let decoder = Decoder::new(data)?;
    Ok((decoder.frame(), decoder.decode()?))
}

fn store(line: &[u16], row: &mut [u8]) {
    if row.len() == line.len() {
        for (byte, &sample) in row.iter_mut().zip(line) {
            *byte = sample as u8;
        }
    } else {
        for (bytes, &sample) in row.as_chunks_mut::<2>().0.iter_mut().zip(line) {
            bytes.copy_from_slice(&sample.to_le_bytes());
        }
    }
}

/// Decodes the scan, giving each line to `row` as it is complete.
fn scan(
    header: &Header,
    data: &[u8],
    mut row: impl FnMut(&[u16]) -> Result<(), Error>,
) -> Result<(), Error> {
    let p = &header.params;
    let width = header.frame.width as usize;
    let quantizer = Quantizer::new(p);
    let mut regular = [Regular::new(p.initial_a()); 365];
    let mut run = [Run::new(p.initial_a(), 0), Run::new(p.initial_a(), 1)];
    let mut run_index = 0;
    let (near, lossless, step) = (p.near, p.near == 0, 2 * p.near + 1);
    let escape = p.limit - p.qbpp - 1;
    // a prediction plus an error, brought back into the sample range (A.4.5, A.8)
    let wrap = p.range * step;
    let reconstruct = |value: i32| -> u16 {
        let value = if value < -near {
            value + wrap
        } else if value > p.maxval + near {
            value - wrap
        } else {
            value
        };
        value.clamp(0, p.maxval) as u16
    };

    let mut reader = BitReader::new(data, header.scan);
    // lines as in the encoder: samples at 1..=width, the edge neighbours around them
    let mut above = vec![0u16; width + 2];
    let mut line = vec![0u16; width + 2];
    for _ in 0..header.frame.height {
        line[0] = above[1];
        above[width + 1] = above[width];
        let mut x = 1;
        while x <= width {
            let ra = i32::from(line[x - 1]);
            let rb = i32::from(above[x]);
            let rc = i32::from(above[x - 1]);
            let rd = i32::from(above[x + 1]);
            let q = quantizer.context(rd - rb, rb - rc, rc - ra);
            if q != 0 {
                let sign = q >> 31;
                let ctx = &mut regular[((q ^ sign) - sign) as usize];
                let k = ctx.k();
                let px = (predict(ra, rb, rc) + ((ctx.c ^ sign) - sign)).clamp(0, p.maxval);
                let mapped = reader.golomb(k, escape, p.qbpp)?;
                if mapped > MAX_MAPPED {
                    return Err(Error::InvalidData("an error value out of range"));
                }
                let e = ((mapped >> 1) as i32 ^ -((mapped & 1) as i32)) ^ ctx.flip(k, lossless);
                ctx.update(e, step, p.reset);
                line[x] = reconstruct(px + ((e ^ sign) - sign) * step);
                x += 1;
                continue;
            }

            // Run mode (A.7): each 1 is a full stretch of the sample on the left, or what
            // remains of the line; a 0 is followed by the length of a shorter one and by
            // the sample that ended it
            let left = line[x - 1];
            loop {
                if reader.bit() == 1 {
                    let full = 1usize << J[run_index];
                    let count = full.min(width + 1 - x);
                    line[x..x + count].fill(left);
                    x += count;
                    if count == full && run_index < 31 {
                        run_index += 1;
                    }
                    if x > width {
                        break;
                    }
                    continue;
                }
                let count = reader.bits(J[run_index]) as usize;
                if x + count > width {
                    return Err(Error::InvalidData("a run past the end of the line"));
                }
                line[x..x + count].fill(left);
                x += count;

                let rb = i32::from(above[x]);
                let agree = (ra - rb).abs() <= near;
                let ctx = &mut run[usize::from(agree)];
                let k = ctx.k();
                let mapped = reader.golomb(k, p.limit - J[run_index] - p.qbpp - 2, p.qbpp)?;
                if mapped > MAX_MAPPED {
                    return Err(Error::InvalidData("an error value out of range"));
                }
                // twice the magnitude, less one when the parity says so (A.7.2.1)
                let doubled = mapped as i32 + ctx.ri_type;
                let magnitude = (doubled + 1) >> 1;
                let negative = (doubled & 1 == 1) != ctx.positive_is_odd(k);
                let e = if negative { -magnitude } else { magnitude };
                ctx.update(e, mapped as i32, p.reset);
                line[x] = reconstruct(if agree {
                    ra + e * step
                } else if ra > rb {
                    rb - e * step
                } else {
                    rb + e * step
                });
                run_index = run_index.saturating_sub(1);
                x += 1;
                break;
            }
        }
        if reader.overrun() {
            return Err(Error::Truncated);
        }
        row(&line[1..=width])?;
        std::mem::swap(&mut above, &mut line);
    }
    marker::read_end(data, reader.end()?)
}
