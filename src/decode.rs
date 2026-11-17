//! The decoder (T.87 annex A with any NEAR, annex B for several components).

use crate::bits::BitReader;
use crate::context::{Coding, Contexts, Lines, Regular, Run};
use crate::marker::{self, Scan, Segments};
use crate::params::J;
use crate::{Error, FrameInfo, Interleave};

/// No mapped error of a valid stream is above this; the check keeps the statistics of a
/// hostile one within 32 bits.
const MAX_MAPPED: u32 = 2 * 65_536;

/// A JPEG-LS stream whose headers were read: what the image is, before decoding it.
///
/// The size comes from the stream, so a caller that knows what to expect (the Rows,
/// Columns and Samples per Pixel of a DICOM instance) compares [`Decoder::frame`] with it
/// before decoding.
#[derive(Debug, Clone)]
pub struct Decoder<'a> {
    data: &'a [u8],
    frame: FrameInfo,
    segments: Segments,
    first: Scan,
}

impl<'a> Decoder<'a> {
    /// Reads the headers of `data`, up to the coded samples.
    pub fn new(data: &'a [u8]) -> Result<Decoder<'a>, Error> {
        let (segments, frame, first) = marker::read_header(data)?;
        Ok(Decoder {
            data,
            frame,
            segments,
            first,
        })
    }

    pub fn frame(&self) -> FrameInfo {
        self.frame
    }

    /// The largest difference between a decoded sample and the original: 0 for a
    /// lossless stream.
    pub fn near(&self) -> u8 {
        self.first.params.near as u8
    }

    /// How the stream orders the components; `None` for an image of one.
    pub fn interleave(&self) -> Interleave {
        self.first.interleave
    }

    /// Decodes into `out`, which holds `self.frame().byte_len()` bytes.
    pub fn decode_into(&self, out: &mut [u8]) -> Result<(), Error> {
        if out.len() != self.frame.byte_len() {
            return Err(Error::InvalidParameter(
                "the pixel buffer is not width x height pixels",
            ));
        }
        let row_len = self.frame.row_len();
        self.scans(|row, component, line| {
            store(
                line,
                &mut out[row * row_len..][..row_len],
                component,
                self.frame,
            );
            Ok(())
        })
    }

    /// Decodes into a new buffer, which grows as rows are decoded: a short stream that
    /// claims a huge image fails where its data ends, before the memory is taken.
    pub fn decode(&self) -> Result<Vec<u8>, Error> {
        let row_len = self.frame.row_len();
        let mut out = Vec::new();
        self.scans(|row, component, line| {
            let start = row * row_len;
            if out.len() == start {
                out.try_reserve(row_len).map_err(|_| Error::TooLarge)?;
                out.resize(start + row_len, 0);
            }
            store(line, &mut out[start..][..row_len], component, self.frame);
            Ok(())
        })?;
        Ok(out)
    }

    /// Decodes every scan, giving each line of each component to `put` with its row and
    /// component. Rows come in order, and the first component of a row before the others.
    fn scans(
        &self,
        mut put: impl FnMut(usize, usize, &[u16]) -> Result<(), Error>,
    ) -> Result<(), Error> {
        let width = self.frame.width as usize;
        let height = self.frame.height as usize;
        let all = usize::from(self.frame.components);
        let mut segments = self.segments;
        let mut scan = self.first;
        let mut decoded = 0;
        loop {
            let coding = Coding::new(scan.params);
            let mut contexts = Contexts::new(&coding.p);
            let mut reader = BitReader::new(self.data, scan.start);
            let mut lines: Vec<Lines> = (0..scan.components).map(|_| Lines::new(width)).collect();
            let mut run_indexes = [0; 4];
            for row in 0..height {
                match scan.interleave {
                    // each component keeps its own place in the run-length code (B.3)
                    Interleave::None | Interleave::Line => {
                        for (lines, run_index) in lines.iter_mut().zip(&mut run_indexes) {
                            decode_line(&coding, &mut contexts, run_index, lines, &mut reader)?;
                        }
                    }
                    Interleave::Sample => {
                        let run_index = &mut run_indexes[0];
                        decode_pixels(&coding, &mut contexts, run_index, &mut lines, &mut reader)?;
                    }
                }
                if reader.overrun() {
                    return Err(Error::Truncated);
                }
                for (component, lines) in lines.iter_mut().enumerate() {
                    put(row, decoded + component, &lines.line[1..=width])?;
                    lines.advance();
                }
            }
            let end = reader.end()?;
            decoded += scan.components;
            if decoded == all {
                return marker::read_end(self.data, end);
            }
            scan = segments.next_scan(self.data, end, decoded)?;
        }
    }
}

/// Decodes a stream: what the image is and its samples, one byte each up to 8 bits per
/// sample, two in little-endian order above; the components of a pixel together.
pub fn decode(data: &[u8]) -> Result<(FrameInfo, Vec<u8>), Error> {
    let decoder = Decoder::new(data)?;
    Ok((decoder.frame(), decoder.decode()?))
}

/// Writes the line of one component into a row of the pixel buffer.
fn store(line: &[u16], row: &mut [u8], component: usize, frame: FrameInfo) {
    let components = usize::from(frame.components);
    if frame.bytes_per_sample() == 1 {
        for (pixel, &sample) in row.chunks_exact_mut(components).zip(line) {
            pixel[component] = sample as u8;
        }
    } else if components == 1 {
        for (bytes, &sample) in row.as_chunks_mut::<2>().0.iter_mut().zip(line) {
            *bytes = sample.to_le_bytes();
        }
    } else {
        for (pixel, &sample) in row.chunks_exact_mut(2 * components).zip(line) {
            pixel[2 * component..][..2].copy_from_slice(&sample.to_le_bytes());
        }
    }
}

/// A sample in the regular mode (A.4 to A.6); `q` is the context with its sign.
#[inline(always)]
fn regular(
    coding: &Coding,
    contexts: &mut [Regular; 365],
    q: i32,
    predicted: i32,
    reader: &mut BitReader,
) -> Result<u16, Error> {
    let sign = q >> 31;
    let ctx = &mut contexts[((q ^ sign) - sign) as usize];
    let k = ctx.k();
    let px = (predicted + ((ctx.c ^ sign) - sign)).clamp(0, coding.p.maxval);
    let mapped = reader.golomb(k, coding.escape, coding.p.qbpp)?;
    if mapped > MAX_MAPPED {
        return Err(Error::InvalidData("an error value out of range"));
    }
    let e = ((mapped >> 1) as i32 ^ -((mapped & 1) as i32)) ^ ctx.flip(k, coding.p.near == 0);
    ctx.update(e, coding.step, coding.p.reset);
    Ok(coding.reconstruct(px + ((e ^ sign) - sign) * coding.step))
}

/// The error of the sample that ended a run (A.7.2), in steps of 2 NEAR + 1.
#[inline]
fn interruption(
    coding: &Coding,
    ctx: &mut Run,
    run_index: usize,
    reader: &mut BitReader,
) -> Result<i32, Error> {
    let k = ctx.k();
    let escape = coding.p.limit - J[run_index] - coding.p.qbpp - 2;
    let mapped = reader.golomb(k, escape, coding.p.qbpp)?;
    if mapped > MAX_MAPPED {
        return Err(Error::InvalidData("an error value out of range"));
    }
    // twice the magnitude, less one when the parity says so (A.7.2.1)
    let doubled = mapped as i32 + ctx.ri_type;
    let magnitude = (doubled + 1) >> 1;
    let negative = (doubled & 1 == 1) != ctx.positive_is_odd(k);
    let e = if negative { -magnitude } else { magnitude };
    ctx.update(e, mapped as i32, coding.p.reset);
    Ok(e * coding.step)
}

/// How many pixels the next piece of a run covers from `x`, and whether the sample that
/// ended the run follows (A.7.1): each 1 is a full stretch or what remains of the line;
/// a 0 comes with the length of a shorter one.
#[inline]
fn run_piece(
    x: usize,
    width: usize,
    run_index: &mut usize,
    reader: &mut BitReader,
) -> Result<(usize, bool), Error> {
    if reader.bit() == 1 {
        let full = 1usize << J[*run_index];
        let count = full.min(width + 1 - x);
        if count == full && *run_index < 31 {
            *run_index += 1;
        }
        return Ok((count, false));
    }
    let count = reader.bits(J[*run_index]) as usize;
    if x + count > width {
        return Err(Error::InvalidData("a run past the end of the line"));
    }
    Ok((count, true))
}

/// A line of one component.
fn decode_line(
    coding: &Coding,
    contexts: &mut Contexts,
    run_index: &mut usize,
    lines: &mut Lines,
    reader: &mut BitReader,
) -> Result<(), Error> {
    lines.edges();
    let width = lines.width();
    let mut x = 1;
    while x <= width {
        let q = lines.context(&coding.quantizer, x);
        if q != 0 {
            lines.line[x] = regular(coding, &mut contexts.regular, q, lines.predict(x), reader)?;
            x += 1;
            continue;
        }

        // Run mode (A.7): the sample on the left, repeated
        let left = lines.line[x - 1];
        loop {
            let (count, interrupted) = run_piece(x, width, run_index, reader)?;
            lines.line[x..x + count].fill(left);
            x += count;
            if interrupted {
                // predicted by the sample above when it differs from the run, with the
                // error turned so that it leans one way
                let ra = i32::from(left);
                let rb = i32::from(lines.above[x]);
                let agree = (ra - rb).abs() <= coding.p.near;
                let ctx = &mut contexts.run[usize::from(agree)];
                let e = interruption(coding, ctx, *run_index, reader)?;
                lines.line[x] = coding.reconstruct(if agree {
                    ra + e
                } else if ra > rb {
                    rb - e
                } else {
                    rb + e
                });
                *run_index = run_index.saturating_sub(1);
                x += 1;
                break;
            }
            if x > width {
                break;
            }
        }
    }
    Ok(())
}

/// A line of every component, pixel by pixel (B.4): a run is one of whole pixels.
fn decode_pixels(
    coding: &Coding,
    contexts: &mut Contexts,
    run_index: &mut usize,
    lines: &mut [Lines],
    reader: &mut BitReader,
) -> Result<(), Error> {
    lines.iter_mut().for_each(Lines::edges);
    let width = lines[0].width();
    let mut x = 1;
    while x <= width {
        let mut q = [0; 4];
        for (q, lines) in q.iter_mut().zip(lines.iter()) {
            *q = lines.context(&coding.quantizer, x);
        }
        if q != [0; 4] {
            for (&q, lines) in q.iter().zip(lines.iter_mut()) {
                lines.line[x] =
                    regular(coding, &mut contexts.regular, q, lines.predict(x), reader)?;
            }
            x += 1;
            continue;
        }

        loop {
            let (count, interrupted) = run_piece(x, width, run_index, reader)?;
            for lines in lines.iter_mut() {
                let left = lines.line[x - 1];
                lines.line[x..x + count].fill(left);
            }
            x += count;
            if interrupted {
                // each sample of the pixel predicted by the one above it
                for lines in lines.iter_mut() {
                    let ra = i32::from(lines.line[x - 1]);
                    let rb = i32::from(lines.above[x]);
                    let e = interruption(coding, &mut contexts.run[0], *run_index, reader)?;
                    lines.line[x] = coding.reconstruct(if ra > rb { rb - e } else { rb + e });
                }
                *run_index = run_index.saturating_sub(1);
                x += 1;
                break;
            }
            if x > width {
                break;
            }
        }
    }
    Ok(())
}
