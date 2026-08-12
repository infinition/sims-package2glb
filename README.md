<div align="center">

<img src="src-tauri/icons/128x128.png" width="96" alt="sims-package2glb" />

# sims-package2glb

**Open The Sims 2, 3 and 4 `.package` mods and get real glTF out of them.**

Mesh, UVs, normals, normal map and texture, embedded in a single `.glb`.
No Blender, no Python, no Sims 4 Studio. One window, drag and drop.

[![License: MIT](https://img.shields.io/badge/License-MIT-e08b3c.svg)](LICENSE)
[![Platform](https://img.shields.io/badge/platform-Windows-2e2e2e.svg)](#install)
[![Built with Tauri](https://img.shields.io/badge/built%20with-Tauri%202-2e2e2e.svg)](https://tauri.app)

**English** · [Français](README.fr.md)

</div>

---

## What it does

A `.package` file is a container. Inside it sit the model, its levels of detail,
its materials and every recolour the creator shipped, all in formats Electronic
Arts never documented. Existing tools either need a full modding suite installed
or hand you a pile of raw resources and wish you luck.

This one reads those formats directly and writes a standard glTF binary that
Blender, Godot, Unreal, Unity, Three.js or any glTF viewer opens without a
plugin.

| | |
|---|---|
| **Drop anything** | One file, a selection, or a whole folder. |
| **See it immediately** | Built in Three.js viewer, orbit, wireframe, grid. |
| **Pick the colour** | Mods ship several recolours. Every one is offered as a thumbnail, and the preview updates on click. |
| **Export in bulk** | One folder per object. Optionally the raw resources too: textures as readable `.dds` and `.png`, original 3D resources. |
| **Three games** | The Sims 4, The Sims 3 and The Sims 2, detected automatically. |
| **English or French** | Toggle in the corner, remembered between sessions. |

## Install

Grab the latest build from the [releases page](../../releases):

- `sims-package2glb_x.y.z_x64-setup.exe` for the installer,
- `sims-package2glb.exe` for a single portable file.

Windows only for now. The whole application is a 4.5 MB executable with no
runtime to install.

## Use

**Drop them on the executable** and they are converted where they already sit:
one folder per object beside the package, raw resources included, no window.
The same works from a terminal, which reports what it did:

```bash
sims-package2glb.exe "C:\Mods\my object.package"
sims-package2glb.exe "C:\Mods"
```

Or open the window for the viewer and the colour picker:

1. Drag `.package` files, or a folder of them, onto the window.
2. Click an entry in the list to look at it.
3. Pick a colour from the strip at the bottom.
4. Choose an output folder and press **Export**.

Each object lands in its own folder as `<name>.glb`. Tick *Also extract raw
resources* to get, alongside it:

```
<name>/
  <name>.glb
  1_Textures/    every texture as a readable .dds and a .png preview
  2_Assets_3D/   original MODL / MLOD resources
  3_Data/        object definitions, strings, tuning
```

## Build from source

Requires [Rust](https://rustup.rs) and [Node](https://nodejs.org).

```bash
npm install
npm run tauri dev      # development window
npm run tauri build    # executable plus NSIS installer
```

## How it is put together

The Rust side owns every format decision. The interface only asks for a scan, a
preview or an export, and arranges the answers.

| file | role |
|------|------|
| `src-tauri/src/dbpf.rs` | DBPF container, zlib and RefPack decompression |
| `src-tauri/src/texture.rs` | `DST1`/`DST5` to DXT, the Sims 2 `cImageData`, block decoding, normal maps |
| `src-tauri/src/rcol.rs` | RCOL container, `MODL`/`MLOD`, geometry and materials |
| `src-tauri/src/gmdc.rs` | The Sims 2 geometry container |
| `src-tauri/src/glb.rs` | glTF 2.0 binary writer |
| `src-tauri/src/extract.rs` | level of detail choice, recolours, Sims 2 material binding, assembly |
| `src/viewer.js` | Three.js scene |
| `src/main.js` | application shell |
| `src/i18n.js` | English and French wording |

## Notes on the formats

These are the findings that make the difference between a correct model and a
convincing pile of garbage. None of them are guessable, and all were measured
against real packages rather than assumed.

**Sims 4 textures are not DXT5, whatever the header suggests.** The four
character code reads `DST5`. Electronic Arts keeps the same DXT blocks but
splits their fields into planes covering the *whole mipmap chain at once*,
endpoints before indices:

```
[ alpha a0/a1 : 2 B ][ colour c0/c1 : 4 B ][ alpha indices : 6 B ][ colour indices : 4 B ]
```

Decoded as ordinary DXT5, this yields nothing but coloured noise. `DST1` keeps
only the two colour planes. Sims 3 stores plain DXT and passes straight through.

**RCOL chunk references are relative.** A reference `0x1000000N` names the chunk
`N` places *after* the one carrying it, not chunk `N`. Measured across a corpus
of packages: 110 out of 110 references resolve to the expected chunk tag when
read as relative, 77 out of 110 when read as absolute.

**Vertex and index buffers are shared.** Several meshes routinely live in one
buffer pair, and each mesh entry carries the offsets and counts of its own
slice, at byte 24 (vertices, in bytes), 32 (indices, in elements), 40 (vertex
count) and 44 (triangle count). Ignore them and you get the right model wrapped
in a starburst of stray triangles. Index buffers are delta encoded across the
entire buffer, so the chain has to be unrolled in full before the slice is cut.

**Positions are homogeneous.** `p = (x, y, z) / w`, with all four components
stored as 16 bit integers. The Sims 4 always writes `w = 32767`, which is why
dividing by a constant appears to work until a Sims 3 file arrives: that game
varies the divisor per vertex (32767, 16383, 10922) to spend precision where the
model needs it.

**A `MODL` chunk of version `0x03xx` (Sims 4) or `0x01xx` (Sims 3) lists no
meshes.** It is a level of detail descriptor pointing at an `MLOD` chunk inside
the same resource. Only version `0x02xx` carries a mesh list.

**Normal maps store two channels.** X sits in alpha, Y in the colour part, where
R, G and B carry the same signal and G is the most precise. Z is reconstructed.
The green channel follows the DirectX convention and has to be inverted for
glTF: on the high relief textures, the stored X and Y hold the *same* sign
relation to the diffuse gradient, which is the signature of green pointing down.
Tangents are exported as `TANGENT` with the handedness derived from the UVs, so
the map is read in the frame it was authored in.

**The default material often points outside the package.** A recolour mod keeps
the base game material as its default and ships its own textures as extra
materials. Following the default alone gives an object with no texture at all.
For Sims 3 the reference cannot be resolved from the package at all, which is
why the colour picker exists.

**The Sims 2 is a different shape entirely.** Its container is version 1 of
DBPF, with a fixed index and no compression marker in the entries: a separate
`DIR` resource lists the compressed keys, and each compressed resource opens
with its own length before the RefPack stream. Its geometry is not RCOL at all
but a `GMDC`, a flat array of typed elements tied together by data groups and
carved into named subsets by index groups. Textures live in `cImageData`
containers: an object header naming the image and its size, then the mipmaps
smallest to largest, each preceded by its size. The largest mip is wrapped back
into a DDS to decode. The materials (`cMaterialDefinition`) name the texture
each mesh wears, and the mesh subset names line up with those names, so the
right texture lands on the right part. When a subset has no material in the
package, the colour picker still offers the rest.

## Contributing

Issues and pull requests are welcome, especially packages that come out wrong.
Attach the `.package` if you can share it, or say where it came from.

## Licence

[MIT](LICENSE). Created by [infinition](https://github.com/infinition).

An independent, unofficial tool. The Sims is a trademark of Electronic Arts Inc.
This project is not affiliated with, endorsed by, or sponsored by Electronic
Arts Inc., and ships no game asset. Anything you extract stays subject to the
terms of whoever created it.
