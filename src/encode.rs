//! The lossless encoder (T.87 annex A with NEAR = 0).

use crate::bits::BitWriter;
use crate::context::{predict, Regular, Run};
use crate::params::{Params, Presets, Quantizer, J};
use crate::{marker, Error, FrameInfo};

/// Codes an image without loss.
///
/// `pixels` holds `frame.byte_len()` bytes: one per sample up to 8 bits per sample, two
/// in little-endian order above, row after row. A sample that does not fit
/// `frame.bits_per_sample` is an error, not something the stream would carry.
pub fn encode(frame: FrameInfo, pixels: &[u8]) -> Result<Vec<u8>, Error> {
    frame.validate()?;
    if pixels.len() != frame.byte_len() {
        return Err(Error::InvalidParameter(
            "the pixel buffer is not width x height samples",
        ));
    }
    let params = Params::new(frame.bits_per_sample, 0, Presets::default())?;
    // CT and MR come out at a third to a half; the vector grows for what does not
    let mut out = Vec::with_capacity(pixels.len() / 2 + 64);
    marker::write_header(&mut out, frame, &params);
    let mut writer = BitWriter::new(out);
    scan(frame, &params, pixels, &mut writer)?;
    let mut out = writer.finish();
    marker::write_end(&mut out);
    Ok(out)
}

/// Copies a row of the pixel buffer into the line, refusing samples above `maxval`.
fn load(row: &[u8], line: &mut [u16], maxval: i32) -> Result<(), Error> {
    // MAXVAL is all ones, so a sample is above it exactly when the OR of all is
    let mut any = 0u16;
    if row.len() == line.len() {
        for (sample, &byte) in line.iter_mut().zip(row) {
            *sample = u16::from(byte);
            any |= *sample;
        }
    } else {
        for (sample, bytes) in line.iter_mut().zip(row.chunks_exact(2)) {
            *sample = u16::from_le_bytes([bytes[0], bytes[1]]);
            any |= *sample;
        }
    }
    if i32::from(any) > maxval {
        return Err(Error::SampleOutOfRange);
    }
    Ok(())
}

fn scan(frame: FrameInfo, p: &Params, pixels: &[u8], w: &mut BitWriter) -> Result<(), Error> {
    let width = frame.width as usize;
    let quantizer = Quantizer::new(p);
    let mut regular = [Regular::new(p.initial_a()); 365];
    let mut run = [Run::new(p.initial_a(), 0), Run::new(p.initial_a(), 1)];
    let mut run_index = 0;
    // an error is taken modulo RANGE, a power of two here: its low bits, sign-extended
    let shift = 32 - u32::from(frame.bits_per_sample);
    let escape = p.limit - p.qbpp - 1;

    // A line holds its samples at 1..=width; 0 and width + 1 are the neighbours the
    // standard defines past the two edges (A.2.1). The line above the first is zeros.
    let mut above = vec![0u16; width + 2];
    let mut line = vec![0u16; width + 2];
    for row in pixels.chunks_exact(width * frame.bytes_per_sample()) {
        load(row, &mut line[1..=width], p.maxval)?;
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
                let e = ((i32::from(line[x]) - px) ^ sign) - sign;
                let e = (e << shift) >> shift;
                // errors 0, -1, 1, -2, ... take the codes 0, 1, 2, 3, ...; a context
                // that leans negative swaps each pair
                let mapped = e ^ ctx.flip(k, true);
                w.golomb(((mapped << 1) ^ (mapped >> 31)) as u32, k, escape, p.qbpp);
                ctx.update(e, 1, p.reset);
                x += 1;
                continue;
            }

            // Run mode (A.7): the samples equal to the one on the left, counted
            let start = x;
            while x <= width && line[x] == line[start - 1] {
                x += 1;
            }
            let mut count = (x - start) as u32;
            while count >= 1 << J[run_index] {
                w.put(1, 1);
                count -= 1 << J[run_index];
                if run_index < 31 {
                    run_index += 1;
                }
            }
            if x > width {
                // the line ended the run: one more 1 for what is left of it
                if count > 0 {
                    w.put(1, 1);
                }
                continue;
            }
            w.put(count, J[run_index] + 1);

            // the sample that ended the run (A.7.2)
            let rb = i32::from(above[x]);
            let ctx = &mut run[usize::from(ra == rb)];
            let e = if ra == rb {
                i32::from(line[x]) - ra
            } else if ra > rb {
                rb - i32::from(line[x])
            } else {
                i32::from(line[x]) - rb
            };
            let e = (e << shift) >> shift;
            let k = ctx.k();
            let odd = if e > 0 {
                ctx.positive_is_odd(k)
            } else {
                e < 0 && !ctx.positive_is_odd(k)
            };
            let mapped = 2 * e.abs() - ctx.ri_type - i32::from(odd);
            w.golomb(
                mapped as u32,
                k,
                p.limit - J[run_index] - p.qbpp - 2,
                p.qbpp,
            );
            ctx.update(e, mapped, p.reset);
            run_index = run_index.saturating_sub(1);
            x += 1;
        }
        std::mem::swap(&mut above, &mut line);
    }
    Ok(())
}
