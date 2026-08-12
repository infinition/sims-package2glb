//! Turning a package into something you can look at.
//!
//! Two jobs live here: describing a package well enough for the interface to
//! offer its recolours, and assembling a GLB once one has been chosen.

use crate::dbpf::{self, Package, TYPE_MLOD, TYPE_MODL};
use crate::glb;
use crate::rcol::{self, Mesh};
use crate::texture::{self, Image};
use base64::Engine;
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;

#[derive(Serialize)]
pub struct Swatch {
    /// The texture instance, hex, also what `build` takes back.
    pub id: String,
    pub width: u32,
    pub height: u32,
    /// A small PNG as a data URL, ready for an `<img>`.
    pub thumbnail: String,
}

#[derive(Serialize)]
pub struct PackageInfo {
    pub path: String,
    pub name: String,
    pub game: &'static str,
    pub meshes: usize,
    pub triangles: usize,
    pub textures: usize,
    pub has_normals: bool,
    pub swatches: Vec<Swatch>,
    /// Set when the material's own texture is missing from the package, which
    /// is the normal state of affairs for Sims 3: the pick is then a guess.
    pub guessed: bool,
    /// A stable code rather than a sentence: the interface speaks two
    /// languages and picks the wording itself.
    pub warning: Option<&'static str>,
}

pub struct Model {
    pub meshes: Vec<Mesh>,
}

/// A package may hold one object described several times over: a MODL, and an
/// MLOD per level of detail, all sharing an instance. Only the densest is worth
/// keeping.
fn best_models(package: &Package) -> Vec<Model> {
    let mut best: HashMap<u64, (usize, Vec<Mesh>)> = HashMap::new();
    for resource in &package.resources {
        if resource.kind != TYPE_MODL && resource.kind != TYPE_MLOD {
            continue;
        }
        let meshes = rcol::extract(&resource.data);
        if meshes.is_empty() {
            continue;
        }
        let score: usize = meshes.iter().map(|m| m.vertices.positions.len()).sum();
        match best.get(&resource.instance) {
            Some((previous, _)) if *previous >= score => {}
            _ => {
                best.insert(resource.instance, (score, meshes));
            }
        }
    }
    let mut models: Vec<(usize, Vec<Mesh>)> = best.into_values().collect();
    models.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    models
        .into_iter()
        .map(|(_, meshes)| Model { meshes })
        .collect()
}

fn decoded_images(package: &Package) -> HashMap<u64, Image> {
    let mut out = HashMap::new();
    for resource in package.images() {
        if let Ok(image) = texture::decode(&resource.data) {
            out.insert(resource.instance, image);
        }
    }
    out
}

/// Every diffuse the object can wear, most likely first.
fn choices(models: &[Model], images: &HashMap<u64, Image>) -> (Vec<u64>, bool) {
    let mut ordered: Vec<u64> = Vec::new();
    for model in models {
        for mesh in &model.meshes {
            for candidate in &mesh.palette {
                if images.contains_key(candidate) && !ordered.contains(candidate) {
                    ordered.push(*candidate);
                }
            }
        }
    }
    if !ordered.is_empty() {
        return (ordered, false);
    }

    // Nothing the materials named is here. Offer the package's own textures,
    // biggest first, skipping the ones that are plainly normal maps.
    let mut fallback: Vec<(usize, u64)> = images
        .iter()
        .filter(|(_, image)| !texture::is_normal_map(image))
        .map(|(id, image)| ((image.width * image.height) as usize, *id))
        .collect();
    fallback.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    (fallback.into_iter().map(|(_, id)| id).collect(), true)
}

pub fn scan(path: &Path) -> Result<PackageInfo, String> {
    let blob = std::fs::read(path).map_err(|e| e.to_string())?;
    let package = dbpf::read(&blob)?;
    let models = best_models(&package);
    let images = decoded_images(&package);
    let (palette, guessed) = choices(&models, &images);

    let mut swatches = Vec::new();
    for id in palette.iter().take(64) {
        let Some(image) = images.get(id) else {
            continue;
        };
        let thumb = texture::thumbnail(image, 72);
        let Ok(png) = texture::to_png(&thumb, true) else {
            continue;
        };
        swatches.push(Swatch {
            id: format!("{id:016X}"),
            width: image.width,
            height: image.height,
            thumbnail: format!(
                "data:image/png;base64,{}",
                base64::engine::general_purpose::STANDARD.encode(png)
            ),
        });
    }

    let triangles: usize = models
        .iter()
        .flat_map(|m| &m.meshes)
        .map(|m| m.indices.len() / 3)
        .sum();
    let meshes = models.iter().map(|m| m.meshes.len()).sum();
    let has_normals = models
        .iter()
        .flat_map(|m| &m.meshes)
        .any(|m| m.normal.map(|n| images.contains_key(&n)).unwrap_or(false));

    let warning = if meshes == 0 && package.sims2 {
        // The Sims 2 container is understood, its GMDC geometry is not yet.
        Some("sims2_no_geometry")
    } else if meshes == 0 {
        Some("no_mesh")
    } else if guessed && !swatches.is_empty() {
        Some("external_materials")
    } else {
        None
    };

    Ok(PackageInfo {
        path: path.to_string_lossy().into_owned(),
        name: path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default(),
        game: package.game(),
        meshes,
        triangles,
        textures: images.len(),
        has_normals,
        swatches,
        guessed,
        warning,
    })
}

pub struct Built {
    pub bytes: Vec<u8>,
    pub meshes: usize,
    pub triangles: usize,
    pub textures: usize,
    pub normal_maps: usize,
}

/// Assemble the GLB. `chosen` forces a diffuse on every mesh; without it each
/// mesh keeps the one its own material asks for.
pub fn build(path: &Path, chosen: Option<u64>) -> Result<Built, String> {
    let blob = std::fs::read(path).map_err(|e| e.to_string())?;
    let package = dbpf::read(&blob)?;
    let models = best_models(&package);
    if models.is_empty() {
        return Err("no_mesh".into());
    }
    let images = decoded_images(&package);
    let (palette, _) = choices(&models, &images);

    let mut builder = glb::Builder::new();
    let mut materials: HashMap<(Option<u64>, Option<u64>), usize> = HashMap::new();

    for model in &models {
        for mesh in &model.meshes {
            let diffuse = chosen
                .filter(|id| images.contains_key(id))
                .or_else(|| {
                    mesh.palette
                        .iter()
                        .find(|id| images.contains_key(id))
                        .copied()
                })
                .or_else(|| palette.first().copied());

            // Without a tangent frame a normal map would be applied in the
            // wrong basis, which looks worse than not applying it at all.
            let normal = mesh
                .normal
                .filter(|id| images.contains_key(id))
                .filter(|_| !mesh.tangent_w.is_empty());

            let key = (diffuse, normal);
            let material = match materials.get(&key) {
                Some(index) => *index,
                None => {
                    let base = diffuse.and_then(|id| {
                        let image = images.get(&id)?;
                        let cutout = texture::is_cutout(image);
                        let png = texture::to_png(image, cutout).ok()?;
                        Some((builder.add_texture(&format!("d{id:016X}"), &png), cutout))
                    });
                    let normal_index = normal.and_then(|id| {
                        let image = images.get(&id)?;
                        if !texture::is_normal_map(image) {
                            return None;
                        }
                        let png =
                            texture::to_png(&texture::normal_map_to_rgb(image), false).ok()?;
                        Some(builder.add_texture(&format!("n{id:016X}"), &png))
                    });
                    let name = diffuse
                        .map(|id| format!("mat_{id:016X}"))
                        .unwrap_or_else(|| "mat_defaut".into());
                    let index = builder.add_material(
                        &name,
                        base.map(|(i, _)| i),
                        normal_index,
                        base.map(|(_, c)| c).unwrap_or(false),
                    );
                    materials.insert(key, index);
                    index
                }
            };

            builder.add_mesh(
                &mesh.name,
                &mesh.vertices.positions,
                &mesh.vertices.normals,
                &mesh.vertices.uvs,
                &mesh.vertices.tangents,
                &mesh.tangent_w,
                &mesh.indices,
                Some(material),
            );
        }
    }

    let meshes = builder.mesh_count();
    let triangles = builder.triangles;
    let textures = builder.texture_count();
    let normal_maps = builder.normal_maps;
    Ok(Built {
        bytes: builder.finish("sims-package2glb"),
        meshes,
        triangles,
        textures,
        normal_maps,
    })
}

/// Write every resource out, sorted into folders, next to the GLB.
pub fn dump_resources(path: &Path, into: &Path) -> Result<(usize, usize), String> {
    let blob = std::fs::read(path).map_err(|e| e.to_string())?;
    let package = dbpf::read(&blob)?;
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();

    let folders = ["1_Textures", "2_Assets_3D", "3_Donnees"];
    for folder in folders {
        std::fs::create_dir_all(into.join(folder)).map_err(|e| e.to_string())?;
    }

    let mut written = 0usize;
    let mut converted = 0usize;
    for resource in &package.resources {
        let magic = resource.data.get(..4).unwrap_or(&[]);
        let (folder, extension) = match (resource.kind, magic) {
            (_, b"DDS ") => ("1_Textures", "dds"),
            (_, [0xFF, 0xD8, 0xFF, _]) => ("1_Textures", "jpg"),
            (_, [0x89, b'P', b'N', b'G']) => ("1_Textures", "png"),
            (dbpf::TYPE_MODL, _) => ("2_Assets_3D", "modl"),
            (dbpf::TYPE_MLOD, _) => ("2_Assets_3D", "mlod"),
            _ => ("3_Donnees", "dat"),
        };
        let base = format!(
            "{stem}_0x{:08X}_0x{:08X}_0x{:016X}",
            resource.kind, resource.group, resource.instance
        );
        let target = into.join(folder).join(format!("{base}.{extension}"));

        let payload = if extension == "dds" {
            texture::unshuffle(&resource.data)
        } else {
            resource.data.clone()
        };
        if std::fs::write(&target, &payload).is_ok() {
            written += 1;
        }
        if extension == "dds" {
            if let Ok(image) = texture::decode(&resource.data) {
                if let Ok(png) = texture::to_png(&image, true) {
                    let preview = into.join(folder).join(format!("{base}.png"));
                    if std::fs::write(preview, png).is_ok() {
                        converted += 1;
                    }
                }
            }
        }
    }
    Ok((written, converted))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a GLB for every package given in `SIMS_TEST_PACKAGES` (a
    /// path-separated list) and drop it in `SIMS_TEST_OUT`, so the result can be
    /// compared against the reference implementation.
    #[test]
    fn build_reference_packages() {
        let Ok(list) = std::env::var("SIMS_TEST_PACKAGES") else {
            return;
        };
        let out = std::env::var("SIMS_TEST_OUT").unwrap_or_else(|_| ".".into());
        for entry in list.split('|').filter(|s| !s.is_empty()) {
            let path = Path::new(entry);
            let info = scan(path).expect("scan");
            let built = build(path, None).expect("build");
            println!(
                "{} [{}] {} maillages, {} triangles, {} textures, {} normales, {} coloris",
                info.name,
                info.game,
                built.meshes,
                built.triangles,
                built.textures,
                built.normal_maps,
                info.swatches.len()
            );
            let target = Path::new(&out).join(format!("{}.glb", info.name));
            std::fs::write(target, built.bytes).expect("write");
        }
    }
}
