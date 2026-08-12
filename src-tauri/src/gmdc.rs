//! GMDC: the geometry container of The Sims 2.
//!
//! Nothing here resembles the RCOL chunks of the later games. A GMDC holds a
//! flat array of typed *elements* (positions, normals, texture coordinates,
//! bone data), a list of *data groups* naming which elements belong together,
//! and a list of named *index groups* holding the triangles. A group's element
//! indices are what ties the three together.
//!
//! Field order was taken from real files and checked the only way that means
//! anything: every index group's highest index lands exactly one below its
//! group's vertex count.

use crate::rcol::{Mesh, Vertices};

/// What an element carries, read from its identity number.
const POSITION: u32 = 0x5B83_0781;
const NORMAL: u32 = 0x3B83_078B;
const TEXCOORD: u32 = 0xBB83_07AB;

struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn u8(&mut self) -> Option<u8> {
        let value = *self.data.get(self.at)?;
        self.at += 1;
        Some(value)
    }

    fn u16(&mut self) -> Option<u16> {
        let slice = self.data.get(self.at..self.at + 2)?;
        self.at += 2;
        Some(u16::from_le_bytes([slice[0], slice[1]]))
    }

    fn u32(&mut self) -> Option<u32> {
        let slice = self.data.get(self.at..self.at + 4)?;
        self.at += 4;
        Some(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
    }

    fn bytes(&mut self, count: usize) -> Option<&'a [u8]> {
        let slice = self.data.get(self.at..self.at + count)?;
        self.at += count;
        Some(slice)
    }

    /// Length prefixed, one byte of length.
    fn string(&mut self) -> Option<String> {
        let length = self.u8()? as usize;
        Some(String::from_utf8_lossy(self.bytes(length)?).into_owned())
    }

    fn skip_string(&mut self) -> Option<()> {
        self.string().map(|_| ())
    }
}

struct Element {
    identity: u32,
    set: u32,
    count: usize,
    data: Vec<u8>,
}

impl Element {
    fn floats(&self, per_item: usize) -> Vec<f32> {
        self.data
            .chunks_exact(4)
            .take(self.count * per_item)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect()
    }
}

/// Read the wrapper: a version marker, the external key list, then one block
/// carrying its class name twice around a version number, then the resource
/// name. What follows is the geometry proper.
fn skip_header(reader: &mut Reader) -> Option<()> {
    reader.u32()?; // 0xFFFF0001
    let external = reader.u32()? as usize;
    for _ in 0..external {
        reader.u32()?;
        reader.u32()?;
        reader.u32()?;
    }
    let blocks = reader.u32()?;
    if blocks == 0 {
        return None;
    }
    reader.u32()?; // block type
    reader.skip_string()?; // cGeometryDataContainer
    reader.u32()?; // block type again
    reader.u32()?; // version
    reader.skip_string()?; // cSGResource
    reader.u32()?;
    reader.u32()?; // its version
    reader.skip_string()?; // the resource name
    Some(())
}

pub fn extract(data: &[u8]) -> Vec<Mesh> {
    let mut reader = Reader { data, at: 0 };
    if skip_header(&mut reader).is_none() {
        return Vec::new();
    }

    let Some(element_count) = reader.u32() else {
        return Vec::new();
    };
    if element_count > 4096 {
        return Vec::new();
    }
    let mut elements = Vec::with_capacity(element_count as usize);
    for _ in 0..element_count {
        let Some(count) = reader.u32() else {
            return Vec::new();
        };
        let (Some(identity), Some(_sub), Some(_block), Some(set), Some(size)) = (
            reader.u32(),
            reader.u32(),
            reader.u32(),
            reader.u32(),
            reader.u32(),
        ) else {
            return Vec::new();
        };
        let Some(payload) = reader.bytes(size as usize) else {
            return Vec::new();
        };
        reader.u32(); // trailing marker
        elements.push(Element {
            identity,
            set,
            count: count as usize,
            data: payload.to_vec(),
        });
    }

    // Data groups: which elements make up one pool of vertices.
    let Some(group_count) = reader.u32() else {
        return Vec::new();
    };
    if group_count > 1024 {
        return Vec::new();
    }
    let mut groups: Vec<(Vec<usize>, usize)> = Vec::with_capacity(group_count as usize);
    for _ in 0..group_count {
        let Some(references) = reader.u32() else {
            return Vec::new();
        };
        let mut refs = Vec::with_capacity(references as usize);
        for _ in 0..references {
            match reader.u16() {
                Some(index) => refs.push(index as usize),
                None => return Vec::new(),
            }
        }
        let Some(vertices) = reader.u32() else {
            return Vec::new();
        };
        reader.u32(); // the reference count again
                      // Three optional remapping tables, empty on every object seen so far.
        for _ in 0..3 {
            let Some(entries) = reader.u32() else {
                return Vec::new();
            };
            for _ in 0..entries {
                reader.u16();
            }
        }
        groups.push((refs, vertices as usize));
    }

    // Index groups: the triangles, one named subset each.
    let Some(subset_count) = reader.u32() else {
        return Vec::new();
    };
    let mut meshes = Vec::new();
    for _ in 0..subset_count.min(512) {
        let (Some(_primitive), Some(group), Some(name)) =
            (reader.u32(), reader.u32(), reader.string())
        else {
            break;
        };
        let Some(index_count) = reader.u32() else {
            break;
        };
        let mut indices = Vec::with_capacity(index_count as usize);
        for _ in 0..index_count {
            match reader.u16() {
                Some(index) => indices.push(index as u32),
                None => break,
            }
        }
        reader.u32(); // flags
        if let Some(bones) = reader.u32() {
            for _ in 0..bones {
                reader.u16();
            }
        }

        let Some((refs, vertex_count)) = groups.get(group as usize) else {
            continue;
        };
        let mut vertices = Vertices::default();
        for &index in refs {
            let Some(element) = elements.get(index) else {
                continue;
            };
            if element.count != *vertex_count {
                continue;
            }
            match element.identity {
                POSITION => {
                    vertices.positions = element
                        .floats(3)
                        .chunks_exact(3)
                        .map(|v| [v[0], v[1], v[2]])
                        .collect()
                }
                NORMAL => {
                    vertices.normals = element
                        .floats(3)
                        .chunks_exact(3)
                        .map(|v| [v[0], v[1], v[2]])
                        .collect()
                }
                // Only the first texture coordinate set is worth carrying; the
                // others are lightmap and morph channels.
                TEXCOORD if element.set == 2 && vertices.uvs.is_empty() => {
                    vertices.uvs = element
                        .floats(2)
                        .chunks_exact(2)
                        .map(|v| [v[0], v[1]])
                        .collect()
                }
                _ => {}
            }
        }

        let limit = vertices.positions.len() as u32;
        if limit == 0 {
            continue;
        }
        indices.retain(|&i| i < limit);
        indices.truncate(indices.len() - indices.len() % 3);
        if indices.is_empty() {
            continue;
        }

        meshes.push(Mesh {
            name: if name.is_empty() { "mesh".into() } else { name },
            vertices,
            tangent_w: Vec::new(),
            indices,
            normal: None,
            palette: Vec::new(),
        });
    }
    meshes
}
