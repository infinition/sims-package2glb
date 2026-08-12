//! A small glTF 2.0 binary writer -- just the subset these models need.

use serde_json::{json, Value};

const ARRAY_BUFFER: u32 = 34962;
const ELEMENT_ARRAY_BUFFER: u32 = 34963;
const FLOAT: u32 = 5126;
const UNSIGNED_SHORT: u32 = 5123;
const UNSIGNED_INT: u32 = 5125;

#[derive(Default)]
pub struct Builder {
    buffer: Vec<u8>,
    views: Vec<Value>,
    accessors: Vec<Value>,
    meshes: Vec<Value>,
    nodes: Vec<Value>,
    materials: Vec<Value>,
    textures: Vec<Value>,
    images: Vec<Value>,
    image_keys: Vec<String>,
    pub triangles: usize,
    pub normal_maps: usize,
}

impl Builder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn mesh_count(&self) -> usize {
        self.meshes.len()
    }

    pub fn texture_count(&self) -> usize {
        self.images.len()
    }

    fn add_view(&mut self, blob: &[u8], target: Option<u32>) -> usize {
        while self.buffer.len() % 4 != 0 {
            self.buffer.push(0);
        }
        let mut view = json!({
            "buffer": 0,
            "byteOffset": self.buffer.len(),
            "byteLength": blob.len(),
        });
        if let Some(target) = target {
            view["target"] = json!(target);
        }
        self.buffer.extend_from_slice(blob);
        self.views.push(view);
        self.views.len() - 1
    }

    fn add_accessor(
        &mut self,
        blob: &[u8],
        component: u32,
        count: usize,
        kind: &str,
        target: Option<u32>,
        bounds: Option<([f32; 3], [f32; 3])>,
    ) -> usize {
        let view = self.add_view(blob, target);
        let mut accessor = json!({
            "bufferView": view,
            "componentType": component,
            "count": count,
            "type": kind,
        });
        if let Some((lo, hi)) = bounds {
            accessor["min"] = json!(lo);
            accessor["max"] = json!(hi);
        }
        self.accessors.push(accessor);
        self.accessors.len() - 1
    }

    /// Textures are shared between materials, so the same bytes are only stored
    /// once however many recolours reference them.
    pub fn add_texture(&mut self, key: &str, png: &[u8]) -> usize {
        if let Some(index) = self.image_keys.iter().position(|k| k == key) {
            return index;
        }
        let view = self.add_view(png, None);
        self.images
            .push(json!({ "bufferView": view, "mimeType": "image/png" }));
        self.textures
            .push(json!({ "sampler": 0, "source": self.images.len() - 1 }));
        self.image_keys.push(key.to_string());
        self.textures.len() - 1
    }

    pub fn add_material(
        &mut self,
        name: &str,
        base_colour: Option<usize>,
        normal: Option<usize>,
        cutout: bool,
    ) -> usize {
        let mut pbr = json!({ "metallicFactor": 0.0, "roughnessFactor": 0.7 });
        match base_colour {
            Some(index) => pbr["baseColorTexture"] = json!({ "index": index }),
            None => pbr["baseColorFactor"] = json!([0.8, 0.8, 0.8, 1.0]),
        }
        let mut material = json!({
            "name": name,
            "pbrMetallicRoughness": pbr,
            "doubleSided": true,
        });
        if let Some(index) = normal {
            material["normalTexture"] = json!({ "index": index, "scale": 1.0 });
            self.normal_maps += 1;
        }
        if cutout {
            material["alphaMode"] = json!("MASK");
            material["alphaCutoff"] = json!(0.5);
        }
        self.materials.push(material);
        self.materials.len() - 1
    }

    #[allow(clippy::too_many_arguments)]
    pub fn add_mesh(
        &mut self,
        name: &str,
        positions: &[[f32; 3]],
        normals: &[[f32; 3]],
        uvs: &[[f32; 2]],
        tangents: &[[f32; 3]],
        tangent_w: &[f32],
        indices: &[u32],
        material: Option<usize>,
    ) {
        let mut blob = Vec::with_capacity(positions.len() * 12);
        let mut lo = [f32::INFINITY; 3];
        let mut hi = [f32::NEG_INFINITY; 3];
        for p in positions {
            for k in 0..3 {
                blob.extend_from_slice(&p[k].to_le_bytes());
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        let mut attributes = json!({
            "POSITION": self.add_accessor(
                &blob, FLOAT, positions.len(), "VEC3", Some(ARRAY_BUFFER), Some((lo, hi)))
        });

        if normals.len() == positions.len() {
            let mut blob = Vec::with_capacity(normals.len() * 12);
            for n in normals {
                let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-9);
                for axis in n {
                    blob.extend_from_slice(&(axis / length).to_le_bytes());
                }
            }
            attributes["NORMAL"] = json!(self.add_accessor(
                &blob,
                FLOAT,
                normals.len(),
                "VEC3",
                Some(ARRAY_BUFFER),
                None
            ));
        }

        if uvs.len() == positions.len() {
            let mut blob = Vec::with_capacity(uvs.len() * 8);
            for uv in uvs {
                blob.extend_from_slice(&uv[0].to_le_bytes());
                blob.extend_from_slice(&uv[1].to_le_bytes());
            }
            attributes["TEXCOORD_0"] =
                json!(self.add_accessor(&blob, FLOAT, uvs.len(), "VEC2", Some(ARRAY_BUFFER), None));
        }

        if tangents.len() == positions.len() && tangent_w.len() == positions.len() {
            let mut blob = Vec::with_capacity(tangents.len() * 16);
            for (t, w) in tangents.iter().zip(tangent_w) {
                let length = (t[0] * t[0] + t[1] * t[1] + t[2] * t[2]).sqrt().max(1e-9);
                for axis in t {
                    blob.extend_from_slice(&(axis / length).to_le_bytes());
                }
                blob.extend_from_slice(&w.to_le_bytes());
            }
            attributes["TANGENT"] = json!(self.add_accessor(
                &blob,
                FLOAT,
                tangents.len(),
                "VEC4",
                Some(ARRAY_BUFFER),
                None
            ));
        }

        let (blob, component) = if positions.len() > 65535 {
            (
                indices
                    .iter()
                    .flat_map(|i| i.to_le_bytes())
                    .collect::<Vec<u8>>(),
                UNSIGNED_INT,
            )
        } else {
            (
                indices
                    .iter()
                    .flat_map(|i| (*i as u16).to_le_bytes())
                    .collect::<Vec<u8>>(),
                UNSIGNED_SHORT,
            )
        };
        let index_accessor = self.add_accessor(
            &blob,
            component,
            indices.len(),
            "SCALAR",
            Some(ELEMENT_ARRAY_BUFFER),
            None,
        );

        let mut primitive = json!({
            "attributes": attributes,
            "indices": index_accessor,
            "mode": 4,
        });
        if let Some(material) = material {
            primitive["material"] = json!(material);
        }
        self.triangles += indices.len() / 3;
        self.meshes
            .push(json!({ "name": name, "primitives": [primitive] }));
        self.nodes
            .push(json!({ "name": name, "mesh": self.meshes.len() - 1 }));
    }

    pub fn finish(self, generator: &str) -> Vec<u8> {
        let mut gltf = json!({
            "asset": { "version": "2.0", "generator": generator },
            "scene": 0,
            "scenes": [{ "nodes": (0..self.nodes.len()).collect::<Vec<_>>() }],
            "nodes": self.nodes,
            "meshes": self.meshes,
            "accessors": self.accessors,
            "bufferViews": self.views,
            "buffers": [{ "byteLength": self.buffer.len() }],
        });
        if !self.materials.is_empty() {
            gltf["materials"] = json!(self.materials);
        }
        if !self.textures.is_empty() {
            gltf["textures"] = json!(self.textures);
            gltf["images"] = json!(self.images);
            gltf["samplers"] = json!([{
                "magFilter": 9729, "minFilter": 9987, "wrapS": 10497, "wrapT": 10497
            }]);
        }

        let mut json_chunk = serde_json::to_vec(&gltf).unwrap_or_default();
        while json_chunk.len() % 4 != 0 {
            json_chunk.push(b' ');
        }
        let mut bin_chunk = self.buffer;
        while bin_chunk.len() % 4 != 0 {
            bin_chunk.push(0);
        }

        let total = 12 + 8 + json_chunk.len() + 8 + bin_chunk.len();
        let mut out = Vec::with_capacity(total);
        out.extend_from_slice(&0x4654_6C67u32.to_le_bytes()); // "glTF"
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&(total as u32).to_le_bytes());
        out.extend_from_slice(&(json_chunk.len() as u32).to_le_bytes());
        out.extend_from_slice(&0x4E4F_534Au32.to_le_bytes()); // "JSON"
        out.extend_from_slice(&json_chunk);
        out.extend_from_slice(&(bin_chunk.len() as u32).to_le_bytes());
        out.extend_from_slice(&0x004E_4942u32.to_le_bytes()); // "BIN"
        out.extend_from_slice(&bin_chunk);
        out
    }
}
