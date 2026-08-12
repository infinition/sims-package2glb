//! The DBPF container, in both the versions the Sims games ship.
//!
//! A package is a header, a pile of resource bytes, and an index at the end
//! saying where each resource lives. The Sims 3 and The Sims 4 use version 2 of
//! the format and differ only in which compressor they reach for: zlib for
//! Sims 4, EA's own RefPack for Sims 3. A header flag also lets the index hoist
//! fields shared by every entry out of the entries themselves.
//!
//! The Sims 2 uses version 1, which keeps its index at a fixed place, has no
//! such flag, and says nothing about compression in the entries: a separate
//! `DIR` resource lists the keys that are compressed.

use std::io::Read;

pub const TYPE_IMG: u32 = 0x00B2D882;
pub const TYPE_MODL: u32 = 0x01661233;
pub const TYPE_MLOD: u32 = 0x01D10F34;

/// The Sims 2 directory of compressed resources.
pub const TYPE_DIR: u32 = 0xE86B1EEF;
/// The Sims 2 geometry container.
pub const TYPE_GMDC: u32 = 0xAC4F8687;

const COMP_NONE: u16 = 0x0000;
const COMP_ZLIB: u16 = 0x5A42;
const COMP_REFPACK: u16 = 0xFFFF;
const COMP_STREAM: u16 = 0xFFFE;

pub struct Resource {
    pub kind: u32,
    pub group: u32,
    pub instance: u64,
    pub data: Vec<u8>,
}

pub struct Package {
    pub resources: Vec<Resource>,
    /// Sims 3 packages compress with RefPack, Sims 4 with zlib. Nothing in the
    /// header names the game, so the compressor is the usable tell.
    pub refpack_seen: bool,
    /// Container version 1, which only The Sims 2 uses.
    pub sims2: bool,
}

impl Package {
    pub fn game(&self) -> &'static str {
        match (self.sims2, self.refpack_seen) {
            (true, _) => "Sims 2",
            (false, true) => "Sims 3",
            (false, false) => "Sims 4",
        }
    }

    pub fn images(&self) -> impl Iterator<Item = &Resource> {
        self.resources.iter().filter(|r| r.kind == TYPE_IMG)
    }
}

fn u16_at(data: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([data[at], data[at + 1]])
}

fn u32_at(data: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]])
}

fn u64_at(data: &[u8], at: usize) -> u64 {
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&data[at..at + 8]);
    u64::from_le_bytes(buf)
}

pub fn read(blob: &[u8]) -> Result<Package, String> {
    if blob.len() < 96 || &blob[..4] != b"DBPF" {
        return Err("not_dbpf".into());
    }
    // Version 1 is The Sims 2, a different container with its own index and its
    // own way of saying what is compressed.
    if u32_at(blob, 4) == 1 {
        return read_v1(blob);
    }

    let entry_count = u32_at(blob, 36) as usize;
    let index_size = u32_at(blob, 44) as usize;
    let mut index_offset = u64_at(blob, 64) as usize;
    if index_offset == 0 {
        index_offset = u32_at(blob, 40) as usize;
    }
    if index_offset + index_size > blob.len() || index_size < 4 {
        return Err("bad_index".into());
    }

    let index = &blob[index_offset..index_offset + index_size];
    let flags = u32_at(index, 0);
    let mut at = 4usize;

    // A field flagged constant is stored once, right after the flags, and then
    // omitted from every entry -- so entries are shorter than the usual 32 bytes.
    let mut constant = [None::<u32>; 3];
    for (bit, slot) in [(0x1u32, 0usize), (0x2, 1), (0x4, 2)] {
        if flags & bit != 0 {
            constant[slot] = Some(u32_at(index, at));
            at += 4;
        }
    }

    let mut resources = Vec::with_capacity(entry_count);
    let mut refpack_seen = false;

    for _ in 0..entry_count {
        let field = |slot: Option<usize>, at: &mut usize| -> u32 {
            match slot.and_then(|s| constant[s]) {
                Some(value) => value,
                None => {
                    let value = u32_at(index, *at);
                    *at += 4;
                    value
                }
            }
        };
        let kind = field(Some(0), &mut at);
        let group = field(Some(1), &mut at);
        let inst_hi = field(Some(2), &mut at);
        let inst_lo = field(None, &mut at);

        let offset = u32_at(index, at) as usize;
        let size_field = u32_at(index, at + 4);
        let plain_size = u32_at(index, at + 8) as usize;
        at += 12;
        let compression = u16_at(index, at);
        at += 4;

        let size = (size_field & 0x7FFF_FFFF) as usize;
        if offset + size > blob.len() {
            continue;
        }
        let raw = &blob[offset..offset + size];
        if compression == COMP_REFPACK || compression == COMP_STREAM {
            refpack_seen = true;
        }

        let data = decompress(raw, compression, plain_size).unwrap_or_else(|_| raw.to_vec());
        resources.push(Resource {
            kind,
            group,
            instance: ((inst_hi as u64) << 32) | inst_lo as u64,
            data,
        });
    }

    Ok(Package {
        resources,
        refpack_seen,
        sims2: false,
    })
}

/// The Sims 2 container.
///
/// The index sits at a fixed place in the header and its entries are 20 or 24
/// bytes depending on the index version. Nothing in an entry says whether the
/// resource is compressed: that lives in a separate `DIR` resource listing the
/// keys that are, along with their plain size. A compressed resource then opens
/// with its own compressed length before the RefPack stream proper.
fn read_v1(blob: &[u8]) -> Result<Package, String> {
    let entry_count = u32_at(blob, 36) as usize;
    let index_offset = u32_at(blob, 40) as usize;
    let index_size = u32_at(blob, 44) as usize;
    let index_version = u32_at(blob, 60);
    let stride = if index_version == 2 { 24 } else { 20 };

    if index_offset + index_size > blob.len() || entry_count * stride > index_size + stride {
        return Err("bad_index".into());
    }

    let mut entries = Vec::with_capacity(entry_count);
    for i in 0..entry_count {
        let at = index_offset + i * stride;
        if at + stride > blob.len() {
            break;
        }
        let kind = u32_at(blob, at);
        let group = u32_at(blob, at + 4);
        let instance = u32_at(blob, at + 8);
        let instance_hi = if stride == 24 {
            u32_at(blob, at + 12)
        } else {
            0
        };
        let offset = u32_at(blob, at + stride - 8) as usize;
        let size = u32_at(blob, at + stride - 4) as usize;
        entries.push((kind, group, instance, instance_hi, offset, size));
    }

    // The directory of compressed resources, keyed the same way as the index.
    let mut compressed: Vec<(u32, u32, u32, u32, usize)> = Vec::new();
    for &(kind, _, _, _, offset, size) in &entries {
        if kind != TYPE_DIR || offset + size > blob.len() {
            continue;
        }
        let record = if stride == 24 { 20 } else { 16 };
        let table = &blob[offset..offset + size];
        for chunk in table.chunks_exact(record) {
            let hi = if record == 20 { u32_at(chunk, 12) } else { 0 };
            compressed.push((
                u32_at(chunk, 0),
                u32_at(chunk, 4),
                u32_at(chunk, 8),
                hi,
                u32_at(chunk, record - 4) as usize,
            ));
        }
    }

    let mut resources = Vec::with_capacity(entries.len());
    for (kind, group, instance, instance_hi, offset, size) in entries {
        if offset + size > blob.len() {
            continue;
        }
        let raw = &blob[offset..offset + size];
        let plain = compressed
            .iter()
            .find(|(t, g, i, h, _)| {
                *t == kind && *g == group && *i == instance && *h == instance_hi
            })
            .map(|(_, _, _, _, plain)| *plain);

        let data = match plain {
            // Four bytes of compressed length, then the RefPack stream.
            Some(plain) if raw.len() > 4 => refpack(&raw[4..], plain),
            _ => raw.to_vec(),
        };
        resources.push(Resource {
            kind,
            group,
            instance: ((instance_hi as u64) << 32) | instance as u64,
            data,
        });
    }

    Ok(Package {
        resources,
        refpack_seen: false,
        sims2: true,
    })
}

fn decompress(raw: &[u8], compression: u16, plain_size: usize) -> Result<Vec<u8>, String> {
    match compression {
        COMP_NONE => Ok(raw.to_vec()),
        COMP_ZLIB => inflate(raw),
        COMP_REFPACK | COMP_STREAM => Ok(refpack(raw, plain_size)),
        // Unknown marker: zlib is the likelier of the two, RefPack the fallback.
        _ => inflate(raw).or_else(|_| Ok(refpack(raw, plain_size))),
    }
}

fn inflate(raw: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(raw)
        .read_to_end(&mut out)
        .map_err(|e| e.to_string())?;
    Ok(out)
}

/// RefPack (a.k.a. QFS), EA's LZ77 variant, used by every compressed Sims 3
/// resource. Control bytes encode a run of literals followed by a back
/// reference; the top bits of the first byte select which of four shapes it is.
fn refpack(data: &[u8], expected: usize) -> Vec<u8> {
    if data.len() < 2 {
        return Vec::new();
    }
    let header = data[0];
    let width = if header & 0x80 != 0 { 4 } else { 3 };
    let mut at = 2usize;
    if header & 0x01 != 0 {
        at += width; // compressed size, which we do not need
    }
    at += width; // plain size, already known from the index

    let mut out: Vec<u8> = Vec::with_capacity(expected);
    while at < data.len() && out.len() < expected {
        let control = data[at] as usize;
        at += 1;
        let (literals, count, distance);

        if control < 0x80 {
            if at >= data.len() {
                break;
            }
            let b1 = data[at] as usize;
            at += 1;
            literals = control & 0x03;
            count = ((control >> 2) & 0x07) + 3;
            distance = ((control & 0x60) << 3) + b1 + 1;
        } else if control < 0xC0 {
            if at + 1 >= data.len() {
                break;
            }
            let (b1, b2) = (data[at] as usize, data[at + 1] as usize);
            at += 2;
            literals = (b1 >> 6) & 0x03;
            count = (control & 0x3F) + 4;
            distance = ((b1 & 0x3F) << 8) + b2 + 1;
        } else if control < 0xE0 {
            if at + 2 >= data.len() {
                break;
            }
            let (b1, b2, b3) = (
                data[at] as usize,
                data[at + 1] as usize,
                data[at + 2] as usize,
            );
            at += 3;
            literals = control & 0x03;
            count = ((control & 0x0C) << 6) + b3 + 5;
            distance = ((control & 0x10) << 12) + (b1 << 8) + b2 + 1;
        } else if control < 0xFC {
            let take = ((control & 0x1F) + 1) * 4;
            let end = (at + take).min(data.len());
            out.extend_from_slice(&data[at..end]);
            at = end;
            continue;
        } else {
            let take = control & 0x03;
            let end = (at + take).min(data.len());
            out.extend_from_slice(&data[at..end]);
            break;
        }

        let end = (at + literals).min(data.len());
        out.extend_from_slice(&data[at..end]);
        at = end;

        if distance == 0 || distance > out.len() {
            break;
        }
        for _ in 0..count {
            let byte = out[out.len() - distance];
            out.push(byte);
        }
    }
    out
}
