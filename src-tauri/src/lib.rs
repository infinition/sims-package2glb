mod dbpf;
mod extract;
mod glb;
mod gmdc;
mod rcol;
mod texture;

use extract::PackageInfo;
use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::ipc::Response;

fn parse_instance(value: Option<String>) -> Option<u64> {
    let text = value?;
    let text = text.trim().trim_start_matches("0x");
    u64::from_str_radix(text, 16).ok()
}

/// Walk a dropped path: a package is taken as is, a folder is searched.
fn collect(input: &str, into: &mut Vec<PathBuf>) {
    let path = Path::new(input);
    if path.is_dir() {
        let Ok(entries) = std::fs::read_dir(path) else {
            return;
        };
        let mut found: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.extension()
                    .map(|e| e.eq_ignore_ascii_case("package"))
                    .unwrap_or(false)
            })
            .collect();
        found.sort();
        into.extend(found);
    } else if path
        .extension()
        .map(|e| e.eq_ignore_ascii_case("package"))
        .unwrap_or(false)
    {
        into.push(path.to_path_buf());
    }
}

/// Reading a package means inflating and decoding every texture in it, which is
/// far too slow to run on the thread answering the interface. Every command
/// hands its work to a blocking pool and awaits the result.
async fn offload<T, F>(work: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn scan_packages(paths: Vec<String>) -> Result<Vec<PackageInfo>, String> {
    offload(move || {
        let mut files = Vec::new();
        for input in &paths {
            collect(input, &mut files);
        }
        files.dedup();

        // Packages are independent, so they are read side by side.
        use rayon::prelude::*;
        Ok(files
            .par_iter()
            .filter_map(|path| extract::scan(path).ok())
            .collect())
    })
    .await
}

/// The GLB for the preview, handed over as raw bytes rather than JSON.
#[tauri::command]
async fn preview(path: String, swatch: Option<String>) -> Result<Response, String> {
    let bytes = offload(move || {
        extract::build(Path::new(&path), parse_instance(swatch)).map(|built| built.bytes)
    })
    .await?;
    Ok(Response::new(bytes))
}

#[derive(Serialize)]
struct ExportReport {
    name: String,
    glb: String,
    meshes: usize,
    triangles: usize,
    textures: usize,
    normal_maps: usize,
    resources: usize,
    previews: usize,
}

#[tauri::command]
async fn export(
    path: String,
    swatch: Option<String>,
    destination: String,
    with_resources: bool,
) -> Result<ExportReport, String> {
    offload(move || export_now(&path, swatch, &destination, with_resources)).await
}

fn export_now(
    path: &str,
    swatch: Option<String>,
    destination: &str,
    with_resources: bool,
) -> Result<ExportReport, String> {
    let source = Path::new(path);
    let name = source
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "objet".into());

    let folder = Path::new(destination).join(&name);
    std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;

    let built = extract::build(source, parse_instance(swatch))?;
    let glb = folder.join(format!("{name}.glb"));
    std::fs::write(&glb, &built.bytes).map_err(|e| e.to_string())?;

    let (resources, previews) = if with_resources {
        extract::dump_resources(source, &folder)?
    } else {
        (0, 0)
    };

    Ok(ExportReport {
        name,
        glb: glb.to_string_lossy().into_owned(),
        meshes: built.meshes,
        triangles: built.triangles,
        textures: built.textures,
        normal_maps: built.normal_maps,
        resources,
        previews,
    })
}

/// The application is built without a console so the window opens on its own.
/// When it is instead started from a terminal, borrowing that terminal is what
/// lets the run report what it did.
#[cfg(windows)]
fn attach_console() {
    use windows::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
    unsafe {
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

#[cfg(not(windows))]
fn attach_console() {}

/// Dropping packages straight onto the executable converts them where they
/// already are, one folder per object beside the package, and never opens a
/// window. Someone with a Downloads folder full of mods gets their models
/// without learning the interface first.
///
/// Returns false when there is nothing on the command line, which is the signal
/// to start normally.
fn run_headless() -> bool {
    let mut files = Vec::new();
    for argument in std::env::args().skip(1) {
        if argument.starts_with('-') {
            continue;
        }
        collect(&argument, &mut files);
    }
    if files.is_empty() {
        return false;
    }
    attach_console();

    let (mut done, mut failed) = (0usize, 0usize);
    for path in &files {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let Some(beside) = path.parent() else {
            failed += 1;
            continue;
        };
        match export_now(
            &path.to_string_lossy(),
            None,
            &beside.to_string_lossy(),
            true,
        ) {
            Ok(report) => {
                done += 1;
                println!(
                    "{name}: {} meshes, {} triangles, {} textures -> {}",
                    report.meshes, report.triangles, report.textures, report.glb
                );
            }
            Err(error) => {
                failed += 1;
                eprintln!("{name}: {error}");
            }
        }
    }
    println!("{done} converted, {failed} failed");
    true
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    if run_headless() {
        return;
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![scan_packages, preview, export])
        .run(tauri::generate_context!())
        .expect("sims-package2glb failed to start");
}
