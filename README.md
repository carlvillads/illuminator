# Illuminator

A simple and fast nifti viewer + more!

> [!WARNING]
> **Disclaimer:** Illuminator is intended for research, educational and/or personal use only and is not certified for any clinical use or diagnosis.

Illuminator opens NIfTI-1 (`.nii`, `.nii.gz` and `.hdr`/`.img` pairs) and NRRD (`.nrrd`, `.nhdr`) volumes (for now).

Some features:
- Stack volumes as layers, overlaid or side by side with a colormap and window per layer
- Label maps can optionally be drawn as outlines with per-label statistics
- Maximum intensity projections (MIPs)
- "Playback" of 4D volumes
- Histogram window
- SUV-BW, SUV-LBM<sub>James</sub>, and SUV-LBM<sub>Janma</sub>  for PET volumes

## Installation

Download the latest package from the [releases page](https://github.com/carlvillads/illuminator/releases).

- **Windows:** Run the `-setup.exe` installer.
- **macOS:** Download the image and drag the app to your applications folder.
- **Debian and Ubuntu:** Install with apt:

  ```bash
  sudo apt install ./illuminate_0.1.0_amd64.deb
  ```

## Use

Open volumes from the command line, or with <kbd>Ctrl</kbd>+<kbd>O</kbd> in an open window, or by dragging and dropping them on a open window.

```bash
illuminate ct.nii.gz pet.nii.gz -c inferno -l segmentation.nii.gz --layout compare
```

| Option | |
| --- | --- |
| `FILE...` | Volumes to open (in order) |
| `-l, --labels FILE` | Open a volume as a label map |
| `-c, --colormap NAME` | Colormap of the volume just before it |
| `--layout NAME` | `ortho` (default), `compare`, `grid`, `single` or `lightbox` |
| `--plane NAME` | Plane of the grid layout: `axial` (default), `coronal` or `sagittal` |
| `--mip` | Show MIPs on open |
| `--size WIDTHxHEIGHT` | Window size in points, e.g. `1440x900` |

Colormaps: `grayscale`, `gray-r`, `red`, `blue`, `viridis`, `plasma`, `magma`, `inferno`, `hot`, `bone`, `cool`, `turbo`, `grayc`, `lipari`, `navia`, `berlin` and `vik`.

### Mouse and keyboard

Press <kbd>?</kbd> in Illuminator to see this list.

| Input | Action |
| --- | --- |
| Click+drag | Move the cursor |
| Right-click+drag | Window/level of the selected layer |
| Scroll | Change slice |
| <kbd>Ctrl</kbd>+scroll | Zoom at the mouse |
| Middle-click+drag or <kbd>Alt</kbd>+drag | Pan a zoomed view |
| Double-click | Focus or unfocus a view |
| <kbd>=</kbd>/<kbd>-</kbd>/<kbd>0</kbd> | Zoom in/out/reset |
| <kbd>1</kbd> – <kbd>5</kbd> | View layouts |
| <kbd>↑</kbd>/<kbd>↓</kbd> | Next/previous slice |
| <kbd>←</kbd>/<kbd>→</kbd> | Previous/next frame |
| <kbd>Space</kbd> | Play/pause 4D volumes |
| <kbd>O</kbd> | Overlaid or side-by-side layers |
| <kbd>J</kbd>/<kbd>K</kbd> | Next/previous layer |
| <kbd>Alt</kbd>+<kbd>J</kbd>/<kbd>K</kbd> | Move layer down/up |
| <kbd>V</kbd> | Show or hide layer |
| <kbd>H</kbd> | Center cursor |
| <kbd>A</kbd> | Auto contrast |
| <kbd>M</kbd> | Next colormap |
| <kbd>L</kbd>/<kbd>Shift</kbd>+<kbd>L</kbd> | Label map on/off/toggle outlines |
| <kbd>S</kbd>/<kbd>R</kbd>/<kbd>C</kbd>/<kbd>I</kbd> | Crosshair sync/radiological orientation/crosshairs/view overlays |
| <kbd>P</kbd> | MIPs |
| <kbd>G</kbd> | Histogram window |
| <kbd>T</kbd> | Switch theme |
| <kbd>Ctrl</kbd>+<kbd>R</kbd> | Reload layer from disk |
| <kbd>Ctrl</kbd>+<kbd>Backspace</kbd> | Close layer |
| <kbd>Ctrl</kbd>+<kbd>C</kbd>/<kbd>E</kbd> | Copy/save the views as an image |
| <kbd>Ctrl</kbd>+<kbd>W</kbd>/<kbd>Q</kbd> | Close window/quit |

## Developing

### Building

```bash
cargo run --release -p illuminator-app -- [FILES]
```

Building on GNU/Linux needs `libfontconfig-dev libfreetype-dev libwayland-dev libxkbcommon-dev
libxkbcommon-x11-dev libx11-xcb-dev`.

Run tests with:

```bash
cargo test --workspace
```

Packages are built with [cargo-packager](https://github.com/crabnebula-dev/cargo-packager) and [cargo-about](https://github.com/EmbarkStudios/cargo-about):

```bash
cargo install cargo-packager --version 0.11.8 --locked
```

```bash
cargo install cargo-about --version 0.9.2 --locked --features cli
```

```bash
cargo packager --release -p illuminator-app
```

The packages are written to `target/packages`. Changes should be recorded in [CHANGELOG.md](CHANGELOG.md).

## Citing

If you feel Illuminator has helped you enough to warrant it, you can cite it using the "Cite this repository" button in the sidebar.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)

at your option.
