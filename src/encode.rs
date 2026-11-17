//! The lossless encoder (T.87 annex A with NEAR = 0, annex B for several components).

use crate::bits::BitWriter;
use crate::context::{Coding, Contexts, Lines, Regular, Run};
use crate::params::{Params, Presets, J};
use crate::{marker, Error, FrameInfo, Interleave};

/// Codes an image without loss. An image of several components is interleaved by line.
///
/// `pixels` holds `frame.byte_len()` bytes: one per sample up to 8 bits per sample, two
/// in little-endian order above; the components of a pixel together, row after row. A
/// sample that does not fit `frame.bits_per_sample` is an error, not something the
/// stream would carry.
pub fn encode(frame: FrameInfo, pixels: &[u8]) -> Result<Vec<u8>, Error> {
    encode_interleaved(frame, Interleave::Line, pixels)
}

/// [`encode`], with the order of the components in the stream chosen. It changes the
/// stream and its size, not what is decoded; an image of one component has no such order.
pub fn encode_interleaved(
    frame: FrameInfo,
    interleave: Interleave,
    pixels: &[u8],
) -> Result<Vec<u8>, Error> {
    frame.validate()?;
    if pixels.len() != frame.byte_len() {
        return Err(Error::InvalidParameter(
            "the pixel buffer is not width x height pixels",
        ));
    }
    let coding = Coding::new(Params::new(frame.bits_per_sample, 0, Presets::default())?);
    // an error is taken modulo RANGE, a power of two here: its low bits, sign-extended
    let shift = 32 - u32::from(frame.bits_per_sample);
    let width = frame.width as usize;
    let rows = || pixels.chunks_exact(frame.row_len());
    let mut lines: Vec<Lines> = (0..frame.components).map(|_| Lines::new(width)).collect();

    // CT and MR come out at a third to a half; the vector grows for what does not
    let mut out = Vec::with_capacity(pixels.len() / 2 + 64);
    marker::write_frame(&mut out, frame, &coding.p);
    if frame.components == 1 || interleave == Interleave::None {
        for (component, lines) in lines.iter_mut().enumerate() {
            marker::write_scan(&mut out, component as u8 + 1, 1, Interleave::None);
            let mut writer = BitWriter::new(out);
            let mut contexts = Contexts::new(&coding.p);
            let mut run_index = 0;
            for row in rows() {
                load(row, component, frame, lines, coding.p.maxval)?;
                encode_line(
                    &coding,
                    &mut contexts,
                    &mut run_index,
                    lines,
                    shift,
                    &mut writer,
                );
                lines.advance();
            }
            out = writer.finish();
        }
    } else {
        marker::write_scan(&mut out, 1, frame.components, interleave);
        let mut writer = BitWriter::new(out);
        let mut contexts = Contexts::new(&coding.p);
        let mut run_indexes = [0; 4];
        for row in rows() {
            for (component, lines) in lines.iter_mut().enumerate() {
                load(row, component, frame, lines, coding.p.maxval)?;
            }
            if interleave == Interleave::Line {
                // each component keeps its own place in the run-length code (B.3)
                for (lines, run_index) in lines.iter_mut().zip(&mut run_indexes) {
                    encode_line(&coding, &mut contexts, run_index, lines, shift, &mut writer);
                }
            } else {
                let run_index = &mut run_indexes[0];
                encode_pixels(
                    &coding,
                    &mut contexts,
                    run_index,
                    &mut lines,
                    shift,
                    &mut writer,
                );
            }
            lines.iter_mut().for_each(Lines::advance);
        }
        out = writer.finish();
    }
    marker::write_end(&mut out);
    Ok(out)
}

/// Copies one component of a row of the pixel buffer into its line, refusing samples
/// above `maxval`.
fn load(
    row: &[u8],
    component: usize,
    frame: FrameInfo,
    lines: &mut Lines,
    maxval: i32,
) -> Result<(), Error> {
    let width = lines.width();
    let line = &mut lines.line[1..=width];
    let components = usize::from(frame.components);
    // MAXVAL is all ones, so a sample is above it exactly when the OR of all is
    let mut any = 0u16;
    if frame.bytes_per_sample() == 1 {
        for (sample, pixel) in line.iter_mut().zip(row.chunks_exact(components)) {
            *sample = u16::from(pixel[component]);
            any |= *sample;
        }
    } else if components == 1 {
        for (sample, bytes) in line.iter_mut().zip(row.as_chunks::<2>().0) {
            *sample = u16::from_le_bytes(*bytes);
            any |= *sample;
        }
    } else {
        for (sample, pixel) in line.iter_mut().zip(row.chunks_exact(2 * components)) {
            *sample = u16::from_le_bytes([pixel[2 * component], pixel[2 * component + 1]]);
            any |= *sample;
        }
    }
    if i32::from(any) > maxval {
        return Err(Error::SampleOutOfRange);
    }
    Ok(())
}

/// A sample in the regular mode (A.4 to A.6): the error of its prediction, in the Golomb
/// code its context calls for. `q` is the context with its sign.
#[inline(always)]
fn regular(
    coding: &Coding,
    contexts: &mut [Regular; 365],
    q: i32,
    predicted: i32,
    sample: u16,
    shift: u32,
    w: &mut BitWriter,
) {
    let sign = q >> 31;
    let ctx = &mut contexts[((q ^ sign) - sign) as usize];
    let k = ctx.k();
    let px = (predicted + ((ctx.c ^ sign) - sign)).clamp(0, coding.p.maxval);
    let e = ((i32::from(sample) - px) ^ sign) - sign;
    let e = (e << shift) >> shift;
    // errors 0, -1, 1, -2, ... take the codes 0, 1, 2, 3, ...; a context that leans
    // negative swaps each pair
    let mapped = e ^ ctx.flip(k, true);
    w.golomb(
        ((mapped << 1) ^ (mapped >> 31)) as u32,
        k,
        coding.escape,
        coding.p.qbpp,
    );
    ctx.update(e, 1, coding.p.reset);
}

/// The length of a run (A.7.1): a 1 for each full stretch, then either a 1 for what the
/// end of the line cut short, or a 0 and the length of the last, shorter stretch.
#[inline]
fn run_length(mut count: u32, to_the_end: bool, run_index: &mut usize, w: &mut BitWriter) {
    while count >= 1 << J[*run_index] {
        w.put(1, 1);
        count -= 1 << J[*run_index];
        if *run_index < 31 {
            *run_index += 1;
        }
    }
    if !to_the_end {
        w.put(count, J[*run_index] + 1);
    } else if count > 0 {
        w.put(1, 1);
    }
}

/// The error `e` of the sample that ended a run (A.7.2).
#[inline]
fn interruption(coding: &Coding, ctx: &mut Run, e: i32, run_index: usize, w: &mut BitWriter) {
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
        coding.p.limit - J[run_index] - coding.p.qbpp - 2,
        coding.p.qbpp,
    );
    ctx.update(e, mapped, coding.p.reset);
}

/// A line of one component.
fn encode_line(
    coding: &Coding,
    contexts: &mut Contexts,
    run_index: &mut usize,
    lines: &mut Lines,
    shift: u32,
    w: &mut BitWriter,
) {
    lines.edges();
    let width = lines.width();
    let mut x = 1;
    while x <= width {
        let q = lines.context(&coding.quantizer, x);
        if q != 0 {
            let (predicted, sample) = (lines.predict(x), lines.line[x]);
            regular(
                coding,
                &mut contexts.regular,
                q,
                predicted,
                sample,
                shift,
                w,
            );
            x += 1;
            continue;
        }

        // Run mode (A.7): the samples equal to the one on the left, counted
        let start = x;
        while x <= width && lines.line[x] == lines.line[start - 1] {
            x += 1;
        }
        run_length((x - start) as u32, x > width, run_index, w);
        if x > width {
            break;
        }
        // the sample that ended the run: predicted by the one above when it differs
        // from the run, with the error turned so that it leans one way
        let ra = i32::from(lines.line[x - 1]);
        let rb = i32::from(lines.above[x]);
        let sample = i32::from(lines.line[x]);
        let e = if ra == rb {
            sample - ra
        } else if ra > rb {
            rb - sample
        } else {
            sample - rb
        };
        let ctx = &mut contexts.run[usize::from(ra == rb)];
        interruption(coding, ctx, (e << shift) >> shift, *run_index, w);
        *run_index = run_index.saturating_sub(1);
        x += 1;
    }
}

/// A line of every component, pixel by pixel (B.4): a run is one of whole pixels.
fn encode_pixels(
    coding: &Coding,
    contexts: &mut Contexts,
    run_index: &mut usize,
    lines: &mut [Lines],
    shift: u32,
    w: &mut BitWriter,
) {
    lines.iter_mut().for_each(Lines::edges);
    let width = lines[0].width();
    let mut x = 1;
    while x <= width {
        let mut q = [0; 4];
        for (q, lines) in q.iter_mut().zip(lines.iter()) {
            *q = lines.context(&coding.quantizer, x);
        }
        if q != [0; 4] {
            for (&q, lines) in q.iter().zip(lines.iter()) {
                let (predicted, sample) = (lines.predict(x), lines.line[x]);
                regular(
                    coding,
                    &mut contexts.regular,
                    q,
                    predicted,
                    sample,
                    shift,
                    w,
                );
            }
            x += 1;
            continue;
        }

        let start = x;
        while x <= width && lines.iter().all(|l| l.line[x] == l.line[start - 1]) {
            x += 1;
        }
        run_length((x - start) as u32, x > width, run_index, w);
        if x > width {
            break;
        }
        // the pixel that ended the run: each sample predicted by the one above it
        for lines in lines.iter() {
            let ra = i32::from(lines.line[x - 1]);
            let rb = i32::from(lines.above[x]);
            let sample = i32::from(lines.line[x]);
            let e = if ra > rb { rb - sample } else { sample - rb };
            interruption(
                coding,
                &mut contexts.run[0],
                (e << shift) >> shift,
                *run_index,
                w,
            );
        }
        *run_index = run_index.saturating_sub(1);
        x += 1;
    }
}
