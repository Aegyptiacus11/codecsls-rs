//! The marker segments around the coded data (T.87 C.1, C.2).

use crate::params::{Params, Presets};
use crate::{Error, FrameInfo};

const SOI: u8 = 0xD8;
const EOI: u8 = 0xD9;
const SOS: u8 = 0xDA;
const DRI: u8 = 0xDD;
const SOF55: u8 = 0xF7;
const LSE: u8 = 0xF8;

/// What comes before the coded data of the one scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Header {
    pub frame: FrameInfo,
    pub params: Params,
    /// Where the coded data starts.
    pub scan: usize,
}

/// SOI, the frame, the scan header, for a lossless scan with the default parameters.
///
/// Above 12 bits the parameters are written out in an LSE segment as well: decoders have
/// disagreed on the defaults there, and the reference encoder writes them for that reason.
pub(crate) fn write_header(out: &mut Vec<u8>, frame: FrameInfo, p: &Params) {
    out.extend([0xFF, SOI]);
    out.extend([0xFF, SOF55, 0, 11, frame.bits_per_sample]);
    out.extend((frame.height as u16).to_be_bytes());
    out.extend((frame.width as u16).to_be_bytes());
    // one component: its identifier, no subsampling, no quantization table
    out.extend([1, 1, 0x11, 0]);
    if frame.bits_per_sample > 12 {
        out.extend([0xFF, LSE, 0, 13, 1]);
        for value in [p.maxval, p.t1, p.t2, p.t3, p.reset] {
            out.extend((value as u16).to_be_bytes());
        }
    }
    // one component, no mapping table, NEAR, no interleaving, no point transform
    out.extend([0xFF, SOS, 0, 8, 1, 1, 0, p.near as u8, 0, 0]);
}

pub(crate) fn write_end(out: &mut Vec<u8>) {
    out.extend([0xFF, EOI]);
}

fn u16_at(bytes: &[u8], at: usize) -> i32 {
    i32::from(u16::from_be_bytes([bytes[at], bytes[at + 1]]))
}

/// Reads up to the coded data of the first scan.
pub(crate) fn read_header(data: &[u8]) -> Result<Header, Error> {
    match data.get(..2) {
        None => return Err(Error::Truncated),
        Some([0xFF, SOI]) => {}
        Some(_) => return Err(Error::InvalidData("no start of image")),
    }
    let mut pos = 2;
    let mut frame = None;
    let mut presets = Presets::default();
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
            EOI => return Err(Error::InvalidData("no scan")),
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
                if frame.is_some() {
                    return Err(Error::InvalidData("a second frame header"));
                }
                frame = Some(read_frame(body)?);
            }
            LSE => read_lse(body, &mut presets)?,
            DRI => {
                if !matches!(body.len(), 2..=4) {
                    return Err(Error::InvalidData("restart interval segment length"));
                }
                if body.iter().any(|&b| b != 0) {
                    return Err(Error::Unsupported("restart intervals"));
                }
            }
            SOS => {
                let frame = frame.ok_or(Error::InvalidData("a scan before the frame header"))?;
                let near = read_scan(body)?;
                let params = Params::new(frame.bits_per_sample, near, presets)?;
                return Ok(Header {
                    frame,
                    params,
                    scan: pos,
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

fn read_frame(body: &[u8]) -> Result<FrameInfo, Error> {
    if body.len() < 6 {
        return Err(Error::InvalidData("frame header length"));
    }
    let components = usize::from(body[5]);
    if body.len() != 6 + 3 * components {
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
    if components != 1 {
        return Err(Error::Unsupported("more than one component"));
    }
    Ok(FrameInfo {
        width: width as u32,
        height: height as u32,
        bits_per_sample: bits,
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
        // mapping tables: only an error when the scan asks for one
        Some(2 | 3) => Ok(()),
        Some(4) => Err(Error::Unsupported("dimensions above 65535")),
        Some(_) => Err(Error::InvalidData("an LSE segment of unknown type")),
    }
}

/// The scan header of a single component: gives NEAR.
fn read_scan(body: &[u8]) -> Result<i32, Error> {
    let components = usize::from(
        *body
            .first()
            .ok_or(Error::InvalidData("scan header length"))?,
    );
    if components != 1 {
        return Err(Error::InvalidData(
            "a scan of more components than the frame has",
        ));
    }
    if body.len() != 6 {
        return Err(Error::InvalidData("scan header length"));
    }
    let (mapping, near, interleave, transform) = (body[2], body[3], body[4], body[5]);
    if mapping != 0 {
        return Err(Error::Unsupported("a mapping table"));
    }
    if interleave > 2 {
        return Err(Error::InvalidData("interleave mode"));
    }
    if transform != 0 {
        return Err(Error::Unsupported("a point transform"));
    }
    Ok(i32::from(near))
}

/// Checks that the image ends at `pos`, where the coded data stopped.
pub(crate) fn read_end(data: &[u8], pos: usize) -> Result<(), Error> {
    match data.get(pos..pos + 2) {
        Some([0xFF, EOI]) => Ok(()),
        Some([0xFF, SOS]) => Err(Error::InvalidData("a second scan for a single component")),
        Some([0xFF, 0xD0..=0xD7]) => Err(Error::Unsupported("restart intervals")),
        Some(_) => Err(Error::InvalidData("no end of image after the scan")),
        None => Err(Error::Truncated),
    }
}
