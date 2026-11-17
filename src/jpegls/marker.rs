//! The marker segments around the coded data (T.87 C.1, C.2).

use super::params::{Params, Presets};
use super::{Error, FrameInfo, Interleave};

const SOI: u8 = 0xD8;
const EOI: u8 = 0xD9;
const SOS: u8 = 0xDA;
const DRI: u8 = 0xDD;
const APP8: u8 = 0xE8;
const SOF55: u8 = 0xF7;
const LSE: u8 = 0xF8;

/// SOI and the frame header, for scans with the default parameters.
///
/// Above 12 bits the parameters are written out in an LSE segment as well: decoders have
/// disagreed on the defaults there, and the reference encoder writes them for that reason.
pub(crate) fn write_frame(out: &mut Vec<u8>, frame: FrameInfo, p: &Params) {
    out.extend([0xFF, SOI]);
    out.extend([
        0xFF,
        SOF55,
        0,
        8 + 3 * frame.components,
        frame.bits_per_sample,
    ]);
    out.extend((frame.height as u16).to_be_bytes());
    out.extend((frame.width as u16).to_be_bytes());
    out.push(frame.components);
    for id in 1..=frame.components {
        // its identifier, no subsampling, no quantization table
        out.extend([id, 0x11, 0]);
    }
    if frame.bits_per_sample > 12 {
        out.extend([0xFF, LSE, 0, 13, 1]);
        for value in [p.maxval, p.t1, p.t2, p.t3, p.reset] {
            out.extend((value as u16).to_be_bytes());
        }
    }
}

/// The header of a lossless scan of `count` components from the one numbered `first`.
pub(crate) fn write_scan(out: &mut Vec<u8>, first: u8, count: u8, interleave: Interleave) {
    out.extend([0xFF, SOS, 0, 6 + 2 * count, count]);
    for id in first..first + count {
        // no mapping table
        out.extend([id, 0]);
    }
    let interleave = match interleave {
        Interleave::None => 0,
        Interleave::Line => 1,
        Interleave::Sample => 2,
    };
    // NEAR, the interleaving, no point transform
    out.extend([0, interleave, 0]);
}

pub(crate) fn write_end(out: &mut Vec<u8>) {
    out.extend([0xFF, EOI]);
}

fn u16_at(bytes: &[u8], at: usize) -> i32 {
    i32::from(u16::from_be_bytes([bytes[at], bytes[at + 1]]))
}

/// A scan header and where its coded data starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Scan {
    /// How many components it codes: one, or all of the frame's.
    pub components: usize,
    pub interleave: Interleave,
    pub params: Params,
    pub start: usize,
}

/// What the segments read so far said.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Segments {
    frame: Option<FrameInfo>,
    presets: Presets,
}

/// Reads the start of image and up to the coded data of the first scan.
pub(crate) fn read_header(data: &[u8]) -> Result<(Segments, FrameInfo, Scan), Error> {
    match data.get(..2) {
        None => return Err(Error::Truncated),
        Some([0xFF, SOI]) => {}
        Some(_) => return Err(Error::InvalidData("no start of image")),
    }
    let mut segments = Segments::default();
    let scan = segments.next_scan(data, 2, 0)?;
    match segments.frame {
        Some(frame) => Ok((segments, frame, scan)),
        // next_scan refuses a scan that comes before the frame header
        None => Err(Error::InvalidData("a scan before the frame header")),
    }
}

impl Segments {
    /// Reads the segments at `pos` up to the coded data of the next scan, `decoded`
    /// components having been decoded already.
    pub fn next_scan(
        &mut self,
        data: &[u8],
        mut pos: usize,
        decoded: usize,
    ) -> Result<Scan, Error> {
        loop {
            match data.get(pos) {
                None => return Err(Error::Truncated),
                Some(0xFF) => {}
                Some(_) => return Err(Error::InvalidData("a marker was expected")),
            }
            // any number of 0xFF may come before the marker's code
            while data.get(pos) == Some(&0xFF) {
                pos += 1;
            }
            let code = *data.get(pos).ok_or(Error::Truncated)?;
            pos += 1;
            match code {
                SOI => return Err(Error::InvalidData("a second start of image")),
                EOI => return Err(Error::InvalidData("the image ends before all its scans")),
                0x00 | 0x01 | 0xD0..=0xD7 => {
                    return Err(Error::InvalidData("a marker that belongs in coded data"));
                }
                _ => {}
            }
            let length = match data.get(pos..pos + 2) {
                Some(bytes) => usize::from(u16::from_be_bytes([bytes[0], bytes[1]])),
                None => return Err(Error::Truncated),
            };
            if length < 2 {
                return Err(Error::InvalidData(
                    "a segment shorter than its length field",
                ));
            }
            let body = data.get(pos + 2..pos + length).ok_or(Error::Truncated)?;
            pos += length;
            match code {
                SOF55 => {
                    if self.frame.is_some() {
                        return Err(Error::InvalidData("a second frame header"));
                    }
                    self.frame = Some(read_frame(body)?);
                }
                LSE => read_lse(body, &mut self.presets)?,
                DRI => {
                    if !matches!(body.len(), 2..=4) {
                        return Err(Error::InvalidData("restart interval segment length"));
                    }
                    if body.iter().any(|&b| b != 0) {
                        return Err(Error::Unsupported("restart intervals"));
                    }
                }
                // what the HP encoders and CharLS write when the components were
                // transformed before coding: not in the standard, and not undone here
                APP8 if body.len() == 5 && body.starts_with(b"mrfx") && body[4] != 0 => {
                    return Err(Error::Unsupported("a colour transform"));
                }
                SOS => {
                    let frame = self
                        .frame
                        .ok_or(Error::InvalidData("a scan before the frame header"))?;
                    let (components, near, interleave) = read_scan(body)?;
                    let all = usize::from(frame.components);
                    let interleave = match (components, interleave) {
                        (1, _) => Interleave::None,
                        (n, 1) if n == all && decoded == 0 => Interleave::Line,
                        (n, 2) if n == all && decoded == 0 => Interleave::Sample,
                        (n, 0) if n == all && decoded == 0 => {
                            return Err(Error::InvalidData(
                                "several components in a scan that does not interleave",
                            ));
                        }
                        _ => return Err(Error::Unsupported("a scan of part of the components")),
                    };
                    let params = Params::new(frame.bits_per_sample, near, self.presets)?;
                    return Ok(Scan {
                        components,
                        interleave,
                        params,
                        start: pos,
                    });
                }
                // the frame headers of the other JPEG processes
                0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF => {
                    return Err(Error::Unsupported("a JPEG frame that is not JPEG-LS"));
                }
                // application data, comments and anything else with a length: skipped
                _ => {}
            }
        }
    }
}

fn read_frame(body: &[u8]) -> Result<FrameInfo, Error> {
    if body.len() < 6 {
        return Err(Error::InvalidData("frame header length"));
    }
    let components = body[5];
    if body.len() != 6 + 3 * usize::from(components) {
        return Err(Error::InvalidData("frame header length"));
    }
    let (bits, height, width) = (body[0], u16_at(body, 1), u16_at(body, 3));
    if !(2..=16).contains(&bits) {
        return Err(Error::InvalidData("sample precision outside 2 to 16 bits"));
    }
    if width == 0 {
        return Err(Error::InvalidData("a width of zero"));
    }
    if height == 0 {
        return Err(Error::Unsupported("a height given after the scan"));
    }
    if components == 0 {
        return Err(Error::InvalidData("a frame of no component"));
    }
    if components > 4 {
        return Err(Error::Unsupported("more than four components"));
    }
    if body[6..]
        .as_chunks::<3>()
        .0
        .iter()
        .any(|component| component[1] != 0x11)
    {
        return Err(Error::Unsupported("subsampled components"));
    }
    Ok(FrameInfo {
        width: width as u32,
        height: height as u32,
        bits_per_sample: bits,
        components,
    })
}

fn read_lse(body: &[u8], presets: &mut Presets) -> Result<(), Error> {
    match body.first() {
        None => Err(Error::InvalidData("an empty LSE segment")),
        Some(1) => {
            if body.len() != 11 {
                return Err(Error::InvalidData("coding parameters segment length"));
            }
            *presets = Presets {
                maxval: u16_at(body, 1),
                t1: u16_at(body, 3),
                t2: u16_at(body, 5),
                t3: u16_at(body, 7),
                reset: u16_at(body, 9),
            };
            Ok(())
        }
        // mapping tables: only an error when a scan asks for one
        Some(2 | 3) => Ok(()),
        Some(4) => Err(Error::Unsupported("dimensions above 65535")),
        Some(_) => Err(Error::InvalidData("an LSE segment of unknown type")),
    }
}

/// A scan header: how many components, NEAR, and the interleave mode as written.
fn read_scan(body: &[u8]) -> Result<(usize, i32, u8), Error> {
    let components = usize::from(
        *body
            .first()
            .ok_or(Error::InvalidData("scan header length"))?,
    );
    if !(1..=4).contains(&components) {
        return Err(Error::InvalidData(
            "a scan of no component, or of more than four",
        ));
    }
    if body.len() != 4 + 2 * components {
        return Err(Error::InvalidData("scan header length"));
    }
    if body[1..1 + 2 * components]
        .as_chunks::<2>()
        .0
        .iter()
        .any(|component| component[1] != 0)
    {
        return Err(Error::Unsupported("a mapping table"));
    }
    let [near, interleave, transform] = [
        body[1 + 2 * components],
        body[2 + 2 * components],
        body[3 + 2 * components],
    ];
    if interleave > 2 {
        return Err(Error::InvalidData("interleave mode"));
    }
    if transform != 0 {
        return Err(Error::Unsupported("a point transform"));
    }
    Ok((components, i32::from(near), interleave))
}

/// Checks that the image ends at `pos`, where the coded data of its last scan stopped.
pub(crate) fn read_end(data: &[u8], pos: usize) -> Result<(), Error> {
    match data.get(pos..pos + 2) {
        Some([0xFF, EOI]) => Ok(()),
        Some([0xFF, SOS]) => Err(Error::InvalidData("more scans than components")),
        Some([0xFF, 0xD0..=0xD7]) => Err(Error::Unsupported("restart intervals")),
        Some(_) => Err(Error::InvalidData("no end of image after the scan")),
        None => Err(Error::Truncated),
    }
}
