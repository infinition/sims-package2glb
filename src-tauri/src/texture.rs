//! Sims textures: DDS containers holding block-compressed pixels.
//!
//! The Sims 2 keeps its images in a different container, `cImageData`, inside a
//! TXTR resource. It is not a DDS at all: a small object header naming the
//! image, then the mipmaps smallest to largest, each preceded by its size. The
//! largest mip is wrapped back into a DDS here so the block decoder is shared.
//!
//! Sims 3 stores ordinary DXT1/DXT5. Sims 4 stores the same blocks with their
//! fields split into planes -- the `DST1`/`DST5` four-character codes -- which
//! compresses far better but is not a DDS any tool will open. Undoing that
//! split is the only thing standing between the file and a normal texture.

use crate::dbpf::TYPE_TXTR;

pub struct Image {
    pub width: u32,
    pub height: u32,
    /// Straight RGBA8, one byte per channel, row major from the top.
    pub pixels: Vec<u8>,
}

fn u32_at(data: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]])
}

fn block_count(width: u32, height: u32, mipmaps: u32) -> usize {
    let mut total = 0usize;
    for level in 0..mipmaps.max(1) {
        let w = (width >> level).max(1);
        let h = (height >> level).max(1);
        total += (w.div_ceil(4) * h.div_ceil(4)) as usize;
    }
    total
}

/// Turn an EA `DST1`/`DST5` texture back into a DDS any decoder understands.
///
/// The block fields are grouped into planes spanning the *whole* mipmap chain
/// at once, endpoints before indices. For DST5, with `n` the total block count:
///
/// ```text
/// [ alpha a0/a1 : 2 B ][ colour c0/c1 : 4 B ][ alpha indices : 6 B ][ colour indices : 4 B ]
/// ```
///
/// and DST1 keeps only the two colour planes. Anything already DXT is returned
/// untouched, so Sims 3 files pass straight through.
pub fn unshuffle(dds: &[u8]) -> Vec<u8> {
    if dds.len() < 128 || &dds[..4] != b"DDS " {
        return dds.to_vec();
    }
    let fourcc = &dds[84..88];
    let dst5 = fourcc == b"DST5";
    let dst1 = fourcc == b"DST1";
    if !dst5 && !dst1 {
        return dds.to_vec();
    }

    let height = u32_at(dds, 12);
    let width = u32_at(dds, 16);
    let mipmaps = u32_at(dds, 28).max(1);
    let body = &dds[128..];

    let stride = if dst5 { 16 } else { 8 };
    let mut blocks = block_count(width, height, mipmaps);
    if blocks * stride != body.len() {
        blocks = body.len() / stride;
    }

    let mut out = vec![0u8; blocks * stride];
    if dst5 {
        let (alpha, colour, alpha_idx, colour_idx) = (0, blocks * 2, blocks * 6, blocks * 12);
        for i in 0..blocks {
            let at = i * 16;
            out[at..at + 2].copy_from_slice(&body[alpha + i * 2..alpha + i * 2 + 2]);
            out[at + 2..at + 8].copy_from_slice(&body[alpha_idx + i * 6..alpha_idx + i * 6 + 6]);
            out[at + 8..at + 12].copy_from_slice(&body[colour + i * 4..colour + i * 4 + 4]);
            out[at + 12..at + 16]
                .copy_from_slice(&body[colour_idx + i * 4..colour_idx + i * 4 + 4]);
        }
    } else {
        let (colour, colour_idx) = (0, blocks * 4);
        for i in 0..blocks {
            let at = i * 8;
            out[at..at + 4].copy_from_slice(&body[colour + i * 4..colour + i * 4 + 4]);
            out[at + 4..at + 8].copy_from_slice(&body[colour_idx + i * 4..colour_idx + i * 4 + 4]);
        }
    }

    let mut fixed = dds[..128].to_vec();
    fixed[84..88].copy_from_slice(if dst5 { b"DXT5" } else { b"DXT1" });
    fixed.extend_from_slice(&out);
    fixed
}

fn rgb565(value: u16) -> [u8; 3] {
    let r = ((value >> 11) & 0x1F) as u32;
    let g = ((value >> 5) & 0x3F) as u32;
    let b = (value & 0x1F) as u32;
    [
        ((r * 255 + 15) / 31) as u8,
        ((g * 255 + 31) / 63) as u8,
        ((b * 255 + 15) / 31) as u8,
    ]
}

/// Rebuild a DDS around the largest mip of a Sims 2 `cImageData` texture.
///
/// The header walk follows the field order found in real files and confirmed
/// against the reference: class name, the embedded `cSGResource` with its two
/// fields and file name, then width, height, format and mip count, then a
/// repeated file name, then the mipmap list. Each mip is a type byte, a size
/// and the pixels; mipmaps run smallest to largest, so the last one is kept.
pub fn sims2_dds(data: &[u8]) -> Result<Vec<u8>, String> {
    let word = |data: &[u8], at: usize| -> Result<u32, String> {
        let slice = data.get(at..at + 4).ok_or("s2_bad_header")?;
        Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
    };
    let str_end = |data: &[u8], at: usize| -> Result<usize, String> {
        let length = *data.get(at).ok_or("s2_bad_header")? as usize;
        let end = at + 1 + length;
        if end > data.len() {
            return Err("s2_bad_header".into());
        }
        Ok(end)
    };

    if word(data, 0x0C)? != TYPE_TXTR {
        return Err("s2_not_txtr".into());
    }
    let mut at = 0x10;
    at = str_end(data, at)?; // cImageData
    at += 8; // block id and version
    at = str_end(data, at)?; // cSGResource
    at += 8; // its two leading fields
    at = str_end(data, at)?; // the resource file name
    let width = word(data, at)?;
    let height = word(data, at + 4)?;
    let format = word(data, at + 8)?;
    at += 16;
    at += 12; // purpose, outer loop, unknown
    at = str_end(data, at)?; // repeated file name, present from version 9
    let images = word(data, at)? as usize;
    at += 4;

    // Mipmaps are stored smallest to largest; the last one is the base level.
    let mut mip0: Option<(usize, usize)> = None;
    for _ in 0..images {
        let kind = *data.get(at).ok_or("s2_bad_header")?;
        at += 1;
        if kind == 0 {
            let size = word(data, at)? as usize;
            at += 4;
            if at + size > data.len() {
                return Err("s2_bad_header".into());
            }
            mip0 = Some((at, size));
            at += size;
        } else {
            at = str_end(data, at)?; // named reference, skip it
        }
    }
    let (offset, size) = mip0.ok_or("s2_no_image")?;

    let fourcc = match format {
        4 => b"DXT1",
        5 => b"DXT3",
        8 => b"DXT5",
        other => return Err(format!("unsupported_format:{other}")),
    };
    let mut dds = dds_header(width, height, fourcc, size as u32);
    dds.extend_from_slice(&data[offset..offset + size]);
    Ok(dds)
}

/// Decode a Sims 2 `cImageData` texture to RGBA8.
pub fn decode_sims2(data: &[u8]) -> Result<Image, String> {
    decode(&sims2_dds(data)?)
}

/// A minimal DDS header, fourcc and one mip level, for the block decoder and
/// for the files written out on export.
fn dds_header(width: u32, height: u32, fourcc: &[u8; 4], linear_size: u32) -> Vec<u8> {
    let mut dds = vec![0u8; 128];
    dds[..4].copy_from_slice(b"DDS ");
    dds[4..8].copy_from_slice(&124u32.to_le_bytes());
    // CAPS, HEIGHT, WIDTH, PIXELFORMAT, MIPMAPCOUNT, LINEARSIZE.
    dds[8..12].copy_from_slice(&0xA1007u32.to_le_bytes());
    dds[12..16].copy_from_slice(&height.to_le_bytes());
    dds[16..20].copy_from_slice(&width.to_le_bytes());
    dds[20..24].copy_from_slice(&linear_size.to_le_bytes());
    dds[28..32].copy_from_slice(&1u32.to_le_bytes());
    dds[76..80].copy_from_slice(&32u32.to_le_bytes());
    dds[80..84].copy_from_slice(&4u32.to_le_bytes());
    dds[84..88].copy_from_slice(fourcc);
    dds[108..112].copy_from_slice(&0x1000u32.to_le_bytes());
    dds
}

/// Decode mip level 0 of a DXT1/DXT5 DDS to RGBA8.
pub fn decode(dds: &[u8]) -> Result<Image, String> {
    let dds = unshuffle(dds);
    if dds.len() < 128 || &dds[..4] != b"DDS " {
        return Err("no_dds_header".into());
    }
    let height = u32_at(&dds, 12);
    let width = u32_at(&dds, 16);
    if width == 0 || height == 0 || width > 8192 || height > 8192 {
        return Err("bad_texture_size".into());
    }
    let (dxt1, dxt3, dxt5) = match &dds[84..88] {
        b"DXT1" => (true, false, false),
        b"DXT3" => (false, true, false),
        b"DXT5" | b"DXT4" => (false, false, true),
        other => {
            return Err(format!(
                "unsupported_format:{}",
                String::from_utf8_lossy(other)
            ))
        }
    };

    let stride = if dxt1 { 8 } else { 16 };
    let body = &dds[128..];
    let (bw, bh) = (width.div_ceil(4), height.div_ceil(4));
    let mut pixels = vec![0u8; (width * height * 4) as usize];

    for by in 0..bh {
        for bx in 0..bw {
            let at = ((by * bw + bx) as usize) * stride;
            if at + stride > body.len() {
                break;
            }
            let block = &body[at..at + stride];

            let mut alpha = [255u8; 16];
            let colour_at = if dxt1 {
                0
            } else if dxt5 {
                let (a0, a1) = (block[0], block[1]);
                let mut table = [0u8; 8];
                table[0] = a0;
                table[1] = a1;
                if a0 > a1 {
                    for i in 1..7 {
                        table[i + 1] =
                            (((7 - i) as u16 * a0 as u16 + i as u16 * a1 as u16) / 7) as u8;
                    }
                } else {
                    for i in 1..5 {
                        table[i + 1] =
                            (((5 - i) as u16 * a0 as u16 + i as u16 * a1 as u16) / 5) as u8;
                    }
                    table[6] = 0;
                    table[7] = 255;
                }
                let mut bits: u64 = 0;
                for (i, byte) in block[2..8].iter().enumerate() {
                    bits |= (*byte as u64) << (8 * i);
                }
                for (i, slot) in alpha.iter_mut().enumerate() {
                    *slot = table[((bits >> (3 * i)) & 0x7) as usize];
                }
                8
            } else if dxt3 {
                // DXT3 stores its alpha as four raw bits per pixel, no
                // endpoints to interpolate.
                for (i, slot) in alpha.iter_mut().enumerate() {
                    *slot = (block[i / 2] >> (4 * (i % 2))) & 0x0F;
                    *slot *= 17;
                }
                8
            } else {
                0
            };

            let c0 = u16::from_le_bytes([block[colour_at], block[colour_at + 1]]);
            let c1 = u16::from_le_bytes([block[colour_at + 2], block[colour_at + 3]]);
            let (p0, p1) = (rgb565(c0), rgb565(c1));
            let mut palette = [[0u8; 3]; 4];
            palette[0] = p0;
            palette[1] = p1;
            // DXT1 keeps one bit of transparency by ordering its endpoints.
            // DXT3 and DXT5 carry explicit alpha, so this never applies to them.
            let punch_through = dxt1 && c0 <= c1;
            for k in 0..3 {
                if punch_through {
                    palette[2][k] = ((p0[k] as u16 + p1[k] as u16) / 2) as u8;
                    palette[3][k] = 0;
                } else {
                    palette[2][k] = ((2 * p0[k] as u16 + p1[k] as u16) / 3) as u8;
                    palette[3][k] = ((p0[k] as u16 + 2 * p1[k] as u16) / 3) as u8;
                }
            }

            let indices = u32::from_le_bytes([
                block[colour_at + 4],
                block[colour_at + 5],
                block[colour_at + 6],
                block[colour_at + 7],
            ]);

            for py in 0..4u32 {
                for px in 0..4u32 {
                    let (x, y) = (bx * 4 + px, by * 4 + py);
                    if x >= width || y >= height {
                        continue;
                    }
                    let slot = (py * 4 + px) as usize;
                    let index = ((indices >> (2 * slot)) & 0x3) as usize;
                    let out = ((y * width + x) * 4) as usize;
                    pixels[out..out + 3].copy_from_slice(&palette[index]);
                    pixels[out + 3] = if punch_through && index == 3 {
                        0
                    } else {
                        alpha[slot]
                    };
                }
            }
        }
    }

    Ok(Image {
        width,
        height,
        pixels,
    })
}

/// True when the alpha channel is a real cut-out mask rather than a spare
/// channel carrying something else (Sims often parks a gloss value there).
pub fn is_cutout(image: &Image) -> bool {
    let transparent = image.pixels.chunks_exact(4).filter(|p| p[3] < 16).count();
    transparent as f32 / (image.pixels.len() / 4) as f32 > 0.03
}

/// A Sims normal map keeps its three colour channels nearly equal and centred
/// on mid-grey; that is what tells it apart from a diffuse.
pub fn is_normal_map(image: &Image) -> bool {
    let mut spread = 0f32;
    let mut middle = 0f32;
    let mut seen = 0f32;
    for p in image.pixels.chunks_exact(4).step_by(17) {
        let (lo, hi) = (p[0].min(p[1]).min(p[2]), p[0].max(p[1]).max(p[2]));
        spread += (hi - lo) as f32;
        middle += p[1] as f32;
        seen += 1.0;
    }
    if seen == 0.0 {
        return false;
    }
    spread / seen < 12.0 && (96.0..160.0).contains(&(middle / seen))
}

/// Rebuild a glTF-ready normal map.
///
/// Only two channels are stored: X in alpha, Y in the colour part (R, G and B
/// carry the same signal, G most precisely). Z is reconstructed, and Y is
/// inverted because the games follow DirectX -- green pointing down -- where
/// glTF follows OpenGL. Measured on the high-relief textures: the stored X and
/// Y hold the *same* sign relation to the diffuse gradient, which is the
/// signature of green-down.
pub fn normal_map_to_rgb(image: &Image) -> Image {
    let mut pixels = vec![255u8; image.pixels.len()];
    for (src, dst) in image.pixels.chunks_exact(4).zip(pixels.chunks_exact_mut(4)) {
        let x = (src[3] as f32 / 255.0) * 2.0 - 1.0;
        let y = 1.0 - (src[1] as f32 / 255.0) * 2.0;
        let z = (1.0 - x * x - y * y).max(0.0).sqrt();
        dst[0] = (((x + 1.0) * 127.5).round()).clamp(0.0, 255.0) as u8;
        dst[1] = (((y + 1.0) * 127.5).round()).clamp(0.0, 255.0) as u8;
        dst[2] = (((z + 1.0) * 127.5).round()).clamp(0.0, 255.0) as u8;
        dst[3] = 255;
    }
    Image {
        width: image.width,
        height: image.height,
        pixels,
    }
}

/// Nearest-neighbour downscale, enough for the swatch thumbnails.
pub fn thumbnail(image: &Image, size: u32) -> Image {
    let mut pixels = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let sx = x * image.width / size;
            let sy = y * image.height / size;
            let from = ((sy * image.width + sx) * 4) as usize;
            let to = ((y * size + x) * 4) as usize;
            pixels[to..to + 4].copy_from_slice(&image.pixels[from..from + 4]);
        }
    }
    Image {
        width: size,
        height: size,
        pixels,
    }
}

pub fn to_png(image: &Image, keep_alpha: bool) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, image.width, image.height);
        encoder.set_color(if keep_alpha {
            png::ColorType::Rgba
        } else {
            png::ColorType::Rgb
        });
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        if keep_alpha {
            writer
                .write_image_data(&image.pixels)
                .map_err(|e| e.to_string())?;
        } else {
            let rgb: Vec<u8> = image
                .pixels
                .chunks_exact(4)
                .flat_map(|p| [p[0], p[1], p[2]])
                .collect();
            writer.write_image_data(&rgb).map_err(|e| e.to_string())?;
        }
    }
    Ok(out)
}
