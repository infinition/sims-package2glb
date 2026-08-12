//! RCOL: the chunk container every Sims model resource is wrapped in.
//!
//! A `MODL` or `MLOD` resource is a list of chunks -- the model itself plus the
//! vertex format, vertex buffer, index buffer and materials it points at. Two
//! details are easy to get wrong and produce convincing garbage rather than an
//! error:
//!
//!  * a chunk reference `0x1000000N` is **relative** to the index of the chunk
//!    holding it, not absolute (measured across a corpus: 110/110 references
//!    resolve to the expected chunk tag when read as relative, 77/110 as
//!    absolute);
//!  * several meshes routinely share one vertex and one index buffer, and each
//!    mesh entry carries the offsets and counts of its own slice.

pub const TAG_MODL: &[u8; 4] = b"MODL";
pub const TAG_MLOD: &[u8; 4] = b"MLOD";

pub const PARAM_DIFFUSE: u32 = 0x6CC0FD85;
pub const PARAM_NORMAL: u32 = 0x6E56548A;

const USAGE_POSITION: u8 = 0;
const USAGE_NORMAL: u8 = 1;
const USAGE_UV: u8 = 2;
const USAGE_TANGENT: u8 = 5;

fn u32_at(data: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]])
}

fn i16_at(data: &[u8], at: usize) -> i16 {
    i16::from_le_bytes([data[at], data[at + 1]])
}

fn f32_at(data: &[u8], at: usize) -> f32 {
    f32::from_le_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]])
}

pub struct Rcol<'a> {
    data: &'a [u8],
    entries: Vec<(usize, usize)>,
}

impl<'a> Rcol<'a> {
    /// Layout: version, public count, unused, external count, internal count,
    /// then the external keys, the internal keys, and the chunk table.
    pub fn parse(data: &'a [u8]) -> Option<Self> {
        if data.len() < 20 {
            return None;
        }
        let external = u32_at(data, 12) as usize;
        let internal = u32_at(data, 16) as usize;
        if internal > 8192 || external > 8192 {
            return None;
        }
        let mut at = 20 + (external + internal) * 16;
        if at + internal * 8 > data.len() {
            return None;
        }
        let mut entries = Vec::with_capacity(internal);
        for _ in 0..internal {
            let position = u32_at(data, at) as usize;
            let size = u32_at(data, at + 4) as usize;
            at += 8;
            entries.push((position, size));
        }
        Some(Rcol { data, entries })
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn chunk(&self, index: usize) -> Option<&'a [u8]> {
        let (position, size) = *self.entries.get(index)?;
        self.data.get(position..position + size)
    }

    pub fn tag(&self, index: usize) -> Option<&'a [u8]> {
        self.chunk(index).and_then(|c| c.get(..4))
    }
}

/// `0x1000000N` names the chunk `N` places after `base`; anything else is null.
fn chunk_ref(value: u32, base: usize) -> Option<usize> {
    if value != 0 && value & 0xF000_0000 == 0x1000_0000 {
        Some(base + (value & 0x0FFF_FFFF) as usize)
    } else {
        None
    }
}

#[derive(Clone, Copy)]
struct Element {
    usage: u8,
    usage_index: u8,
    offset: usize,
    size: usize,
}

fn parse_vrtf(chunk: &[u8]) -> Option<(usize, Vec<Element>)> {
    if chunk.len() < 16 {
        return None;
    }
    let stride = u32_at(chunk, 8) as usize;
    let count = u32_at(chunk, 12) as usize;
    if stride == 0 || stride > 256 || count > 32 || 20 + count * 4 > chunk.len() {
        return None;
    }
    let mut elements: Vec<Element> = (0..count)
        .map(|i| {
            let at = 20 + i * 4;
            Element {
                usage: chunk[at],
                usage_index: chunk[at + 1],
                offset: chunk[at + 3] as usize,
                size: 0,
            }
        })
        .collect();

    // An element's width is the gap to the next one, so it never has to be
    // guessed from the format code -- which Sims 3 and Sims 4 number differently.
    let mut order: Vec<usize> = (0..count).collect();
    order.sort_by_key(|&i| elements[i].offset);
    for (n, &i) in order.iter().enumerate() {
        let next = order
            .get(n + 1)
            .map(|&j| elements[j].offset)
            .unwrap_or(stride);
        elements[i].size = next.saturating_sub(elements[i].offset);
    }
    Some((stride, elements))
}

#[derive(Default)]
pub struct Vertices {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub tangents: Vec<[f32; 3]>,
}

fn unit_byte(byte: u8) -> f32 {
    (byte as f32 / 255.0) * 2.0 - 1.0
}

fn parse_vbuf(
    chunk: &[u8],
    stride: usize,
    elements: &[Element],
    byte_offset: usize,
    wanted: usize,
) -> Option<Vertices> {
    let payload = chunk.get(16 + byte_offset..)?;
    let count = wanted.min(payload.len() / stride);
    if count == 0 {
        return None;
    }
    let mut out = Vertices::default();

    for i in 0..count {
        let base = i * stride;
        for element in elements {
            let at = base + element.offset;
            if at + element.size > payload.len() {
                continue;
            }
            match element.usage {
                USAGE_POSITION => {
                    if element.size >= 12 {
                        out.positions.push([
                            f32_at(payload, at),
                            f32_at(payload, at + 4),
                            f32_at(payload, at + 8),
                        ]);
                    } else if element.size >= 8 {
                        // Homogeneous: the fourth short is the divisor. Sims 4
                        // always writes 32767; Sims 3 varies it per vertex
                        // (32767, 16383, 10922) to spend precision where needed.
                        let w = match i16_at(payload, at + 6) {
                            0 => 32767.0,
                            w => w as f32,
                        };
                        out.positions.push([
                            i16_at(payload, at) as f32 / w,
                            i16_at(payload, at + 2) as f32 / w,
                            i16_at(payload, at + 4) as f32 / w,
                        ]);
                    }
                }
                USAGE_NORMAL => {
                    if element.size >= 12 {
                        out.normals.push([
                            f32_at(payload, at),
                            f32_at(payload, at + 4),
                            f32_at(payload, at + 8),
                        ]);
                    } else {
                        out.normals.push([
                            unit_byte(payload[at]),
                            unit_byte(payload[at + 1]),
                            unit_byte(payload[at + 2]),
                        ]);
                    }
                }
                USAGE_UV if element.usage_index == 0 => {
                    if element.size >= 8 {
                        out.uvs.push([f32_at(payload, at), f32_at(payload, at + 4)]);
                    } else {
                        out.uvs.push([
                            i16_at(payload, at) as f32 / 32767.0,
                            i16_at(payload, at + 2) as f32 / 32767.0,
                        ]);
                    }
                }
                USAGE_TANGENT => {
                    if element.size >= 12 {
                        out.tangents.push([
                            f32_at(payload, at),
                            f32_at(payload, at + 4),
                            f32_at(payload, at + 8),
                        ]);
                    } else {
                        out.tangents.push([
                            unit_byte(payload[at]),
                            unit_byte(payload[at + 1]),
                            unit_byte(payload[at + 2]),
                        ]);
                    }
                }
                _ => {}
            }
        }
    }

    if out.positions.len() != count {
        return None;
    }
    for list in [&mut out.normals, &mut out.tangents] {
        if list.len() != count {
            list.clear();
        }
    }
    if out.uvs.len() != count {
        out.uvs.clear();
    }
    Some(out)
}

/// Indices are stored as successive differences over the *whole* buffer, so the
/// chain is unrolled in full before the mesh's slice is cut out of it.
fn parse_ibuf(chunk: &[u8], offset: usize, wanted: usize) -> Vec<u32> {
    if chunk.len() < 16 {
        return Vec::new();
    }
    let delta = u32_at(chunk, 8) & 1 != 0;
    let payload = &chunk[16..];
    let total = payload.len() / 2;
    let mut all = Vec::with_capacity(total);
    let mut current = 0i64;
    for i in 0..total {
        let raw = i16_at(payload, i * 2);
        if delta {
            current += raw as i64;
        } else {
            current = u16::from_le_bytes([payload[i * 2], payload[i * 2 + 1]]) as i64;
        }
        all.push(current.max(0) as u32);
    }
    let end = (offset + wanted).min(all.len());
    if offset >= end {
        return Vec::new();
    }
    all[offset..end].to_vec()
}

/// Texture references declared by a material chunk, keyed by parameter hash.
///
/// Sims 4 stores a full type/group/instance key inline. Sims 3 stores an index
/// into a table that lives outside the package, so it resolves to nothing here
/// and the caller falls back to picking a texture by hand.
pub fn material_textures(chunk: &[u8]) -> Vec<(u32, u64)> {
    let base = find(chunk, b"MTRL").or_else(|| find(chunk, b"MTNF"));
    let Some(base) = base else {
        return Vec::new();
    };
    if base + 16 > chunk.len() {
        return Vec::new();
    }
    let count = u32_at(chunk, base + 12) as usize;
    if count > 512 {
        return Vec::new();
    }

    let mut out = Vec::new();
    for i in 0..count {
        let at = base + 16 + i * 16;
        if at + 16 > chunk.len() {
            break;
        }
        let name = u32_at(chunk, at);
        let kind = u32_at(chunk, at + 4) & 0xFFFF;
        let entries = u32_at(chunk, at + 8);
        let value_at = base + u32_at(chunk, at + 12) as usize;
        if kind != 4 || entries != 4 || value_at + 16 > chunk.len() {
            continue;
        }
        let mut instance = [0u8; 8];
        instance.copy_from_slice(&chunk[value_at..value_at + 8]);
        out.push((name, u64::from_le_bytes(instance)));
    }
    out
}

fn find(haystack: &[u8], needle: &[u8; 4]) -> Option<usize> {
    haystack.windows(4).position(|w| w == needle)
}

/// The default material of a state set. Its first internal reference is the one
/// the game falls back to when no state applies.
fn mtst_default(chunk: &[u8], base: usize) -> Option<usize> {
    let end = chunk.len().min(64);
    let mut at = 12;
    while at + 4 <= end {
        if let Some(index) = chunk_ref(u32_at(chunk, at), base) {
            return Some(index);
        }
        at += 4;
    }
    None
}

pub struct Mesh {
    pub name: String,
    pub vertices: Vertices,
    /// Handedness for each tangent, in glTF's `TANGENT.w` sense.
    pub tangent_w: Vec<f32>,
    pub indices: Vec<u32>,
    pub normal: Option<u64>,
    /// Every diffuse the resource offers, the mesh's own first: a mod ships its
    /// recolours as extra materials and its "default" often points at a base
    /// game texture that is not in the package at all.
    pub palette: Vec<u64>,
}

pub fn extract(data: &[u8]) -> Vec<Mesh> {
    let Some(rcol) = Rcol::parse(data) else {
        return Vec::new();
    };
    let mut meshes = Vec::new();
    let mut seen = Vec::new();

    for index in 0..rcol.len() {
        let Some(tag) = rcol.tag(index) else { continue };
        if tag != TAG_MODL && tag != TAG_MLOD {
            continue;
        }
        let Some(chunk) = rcol.chunk(index) else {
            continue;
        };
        if chunk.len() < 12 {
            continue;
        }
        // Version 0x03xx (Sims 4) and 0x01xx (Sims 3) are level-of-detail
        // descriptors pointing at an MLOD chunk; only 0x02xx lists meshes.
        if u32_at(chunk, 4) >> 8 != 0x02 {
            continue;
        }
        read_mesh_list(&rcol, chunk, index, &mut meshes, &mut seen);
    }
    meshes
}

fn read_mesh_list(
    rcol: &Rcol,
    chunk: &[u8],
    base: usize,
    meshes: &mut Vec<Mesh>,
    seen: &mut Vec<(u32, u32, u32, u32)>,
) {
    let count = u32_at(chunk, 8) as usize;
    let mut at = 12usize;
    let palette = diffuse_palette(rcol);

    for _ in 0..count.min(256) {
        if at + 4 > chunk.len() {
            break;
        }
        let size = u32_at(chunk, at) as usize;
        at += 4;
        let Some(entry) = chunk.get(at..at + size) else {
            break;
        };
        at += size;
        if entry.len() < 20 {
            continue;
        }

        let name = u32_at(entry, 0);
        let material = u32_at(entry, 4);
        let Some(i_vrtf) = chunk_ref(u32_at(entry, 8), base) else {
            continue; // shadow and reflection placeholders carry no vertex format
        };
        let Some(i_vbuf) = chunk_ref(u32_at(entry, 12), base) else {
            continue;
        };
        let Some(i_ibuf) = chunk_ref(u32_at(entry, 16), base) else {
            continue;
        };

        let (mut vertex_offset, mut index_offset) = (0usize, 0usize);
        let (mut vertex_count, mut triangle_count) = (usize::MAX, usize::MAX);
        if entry.len() >= 48 {
            vertex_offset = u32_at(entry, 24) as usize;
            index_offset = u32_at(entry, 32) as usize;
            vertex_count = u32_at(entry, 40) as usize;
            triangle_count = u32_at(entry, 44) as usize;
        }

        let signature = (
            name,
            u32_at(entry, 12),
            u32_at(entry, 16),
            vertex_offset as u32,
        );
        if seen.contains(&signature) {
            continue;
        }
        seen.push(signature);

        if rcol.tag(i_vrtf) != Some(b"VRTF")
            || rcol.tag(i_vbuf) != Some(b"VBUF")
            || rcol.tag(i_ibuf) != Some(b"IBUF")
        {
            continue;
        }
        let (Some(vrtf), Some(vbuf), Some(ibuf)) =
            (rcol.chunk(i_vrtf), rcol.chunk(i_vbuf), rcol.chunk(i_ibuf))
        else {
            continue;
        };
        let Some((stride, elements)) = parse_vrtf(vrtf) else {
            continue;
        };
        let Some(vertices) = parse_vbuf(vbuf, stride, &elements, vertex_offset, vertex_count)
        else {
            continue;
        };
        let wanted = triangle_count.saturating_mul(3);
        let mut indices = parse_ibuf(ibuf, index_offset, wanted);
        let limit = vertices.positions.len() as u32;
        indices.retain(|&i| i < limit);
        indices.truncate(indices.len() - indices.len() % 3);
        if indices.is_empty() {
            continue;
        }

        let mut material_index = chunk_ref(material, base);
        if let Some(index) = material_index {
            if rcol.tag(index) == Some(b"MTST") {
                material_index = rcol.chunk(index).and_then(|c| mtst_default(c, index));
            }
        }
        let textures = material_index
            .filter(|&i| rcol.tag(i) == Some(b"MATD"))
            .and_then(|i| rcol.chunk(i))
            .map(material_textures)
            .unwrap_or_default();
        let pick = |want: u32| textures.iter().find(|(n, _)| *n == want).map(|(_, v)| *v);

        let mut ordered = palette.clone();
        if let Some(own) = pick(PARAM_DIFFUSE) {
            if let Some(position) = ordered.iter().position(|&v| v == own) {
                ordered.rotate_left(position);
            } else {
                ordered.insert(0, own);
            }
        }

        let tangent_w = tangent_handedness(&vertices, &indices);
        meshes.push(Mesh {
            name: format!("mesh_0x{name:08X}"),
            vertices,
            tangent_w,
            indices,
            normal: pick(PARAM_NORMAL),
            palette: ordered,
        });
    }
}

fn diffuse_palette(rcol: &Rcol) -> Vec<u64> {
    let mut out: Vec<u64> = Vec::new();
    for index in 0..rcol.len() {
        if rcol.tag(index) != Some(b"MATD") {
            continue;
        }
        let Some(chunk) = rcol.chunk(index) else {
            continue;
        };
        for (name, instance) in material_textures(chunk) {
            if name == PARAM_DIFFUSE && !out.contains(&instance) {
                out.push(instance);
            }
        }
    }
    out
}

/// glTF asks for `bitangent = cross(normal, tangent) * w`, with the bitangent
/// pointing *up* the image -- that is, along `-dP/dv`. Accumulating `dP/dv` per
/// triangle and comparing gives `w` without having to trust a convention.
fn tangent_handedness(vertices: &Vertices, indices: &[u32]) -> Vec<f32> {
    if vertices.tangents.is_empty() || vertices.uvs.is_empty() || vertices.normals.is_empty() {
        return Vec::new();
    }
    let mut accum = vec![[0f32; 3]; vertices.positions.len()];
    for triangle in indices.chunks_exact(3) {
        let (a, b, c) = (
            triangle[0] as usize,
            triangle[1] as usize,
            triangle[2] as usize,
        );
        let (pa, pb, pc) = (
            vertices.positions[a],
            vertices.positions[b],
            vertices.positions[c],
        );
        let (ua, ub, uc) = (vertices.uvs[a], vertices.uvs[b], vertices.uvs[c]);
        let e1 = [pb[0] - pa[0], pb[1] - pa[1], pb[2] - pa[2]];
        let e2 = [pc[0] - pa[0], pc[1] - pa[1], pc[2] - pa[2]];
        let (du1, dv1) = (ub[0] - ua[0], ub[1] - ua[1]);
        let (du2, dv2) = (uc[0] - ua[0], uc[1] - ua[1]);
        let det = du1 * dv2 - du2 * dv1;
        if det.abs() < 1e-12 {
            continue;
        }
        let r = 1.0 / det;
        let bitangent = [
            (e2[0] * du1 - e1[0] * du2) * r,
            (e2[1] * du1 - e1[1] * du2) * r,
            (e2[2] * du1 - e1[2] * du2) * r,
        ];
        for &vertex in &[a, b, c] {
            for (slot, value) in accum[vertex].iter_mut().zip(bitangent) {
                *slot += value;
            }
        }
    }

    vertices
        .tangents
        .iter()
        .enumerate()
        .map(|(i, tangent)| {
            let n = vertices.normals[i];
            let cross = [
                n[1] * tangent[2] - n[2] * tangent[1],
                n[2] * tangent[0] - n[0] * tangent[2],
                n[0] * tangent[1] - n[1] * tangent[0],
            ];
            let up = accum[i];
            let dot = -(cross[0] * up[0] + cross[1] * up[1] + cross[2] * up[2]);
            if dot >= 0.0 {
                1.0
            } else {
                -1.0
            }
        })
        .collect()
}
