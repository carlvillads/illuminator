//! Command line options

use clap::error::ErrorKind;
use clap::{CommandFactory, FromArgMatches, Parser, ValueEnum};
use illuminator_core::geometry::ViewPlane;
use illuminator_core::slicer::Colormap;
use std::ffi::OsString;
use std::path::PathBuf;

/// A viewer for NIfTI volumes and more.
#[derive(Debug, Parser)]
#[command(name = "illuminate", version, about)]
pub struct Cli {
    /// Volume(s) to open, opened in typed order.
    #[arg(value_name = "FILE")]
    pub files: Vec<PathBuf>,

    /// Volume to open as a label map. Opened on top of FILE(s).
    #[arg(short, long, value_name = "FILE")]
    pub labels: Vec<PathBuf>,

    /// Colormap of the FILE just before this argument.
    #[arg(short, long, value_enum)]
    pub colormap: Vec<ColormapName>,

    /// Which layout to open [default: ortho]
    #[arg(long, value_enum)]
    pub layout: Option<LayoutName>,

    /// Which plane of the grid [default: axial]
    #[arg(long, value_enum)]
    pub plane: Option<PlaneName>,

    /// Show MIPs (Maximum Intensity Projections)
    #[arg(long)]
    pub mip: bool,

    /// Window size in points
    #[arg(long, value_name = "WIDTHxHEIGHT", value_parser = window_size)]
    pub size: Option<(f32, f32)>,

    /// The colormap of each in `files` from the argument after it.
    #[arg(skip)]
    pub colormaps: Vec<Option<Colormap>>,
}

#[derive(Debug, Clone, Copy, PartialEq, ValueEnum)]
pub enum LayoutName {
    Ortho,
    Compare,
    Grid,
    Single,
    Lightbox,
}

#[derive(Debug, Clone, Copy, PartialEq, ValueEnum)]
pub enum PlaneName {
    Axial,
    Coronal,
    Sagittal,
}

impl From<PlaneName> for ViewPlane {
    fn from(plane: PlaneName) -> Self {
        match plane {
            PlaneName::Axial => Self::Axial,
            PlaneName::Coronal => Self::Coronal,
            PlaneName::Sagittal => Self::Sagittal,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, ValueEnum)]
pub enum ColormapName {
    Grayscale,
    Viridis,
    Plasma,
    Magma,
    Inferno,
}

impl From<ColormapName> for Colormap {
    fn from(colormap: ColormapName) -> Self {
        match colormap {
            ColormapName::Grayscale => Self::Grayscale,
            ColormapName::Inferno => Self::Inferno,
            ColormapName::Magma => Self::Magma,
            ColormapName::Plasma => Self::Plasma,
            ColormapName::Viridis => Self::Viridis,
        }
    }
}

impl Cli {
    /// Reads commandline, prints help and version or error
    pub fn read() -> Self {
        Self::try_read_from(std::env::args_os()).unwrap_or_else(|error| error.exit())
    }

    pub fn try_read_from<T: Into<OsString> + Clone>(
        args: impl IntoIterator<Item = T>,
    ) -> Result<Self, clap::Error> {
        let matches = Self::command().try_get_matches_from(args)?;
        let mut cli = Self::from_arg_matches(&matches)?;
        let positions = |id: &str| -> Vec<usize> {
            matches
                .indices_of(id)
                .map(Iterator::collect)
                .unwrap_or_default()
        };
        let (files, labels) = (positions("files"), positions("labels"));
        cli.colormaps = vec![None; cli.files.len()];
        for (position, &colormap) in positions("colormap").into_iter().zip(&cli.colormap) {
            let file = files.iter().rposition(|&file| file < position);
            let label = labels.iter().rfind(|&&label| label < position);
            let message = match (file, label) {
                (Some(file), label) if label.is_none_or(|&label| label < files[file]) => {
                    cli.colormaps[file] = Some(colormap.into());
                    continue;
                }
                (_, Some(_)) => "label maps do not accept the -c argument",
                _ => "the -c arg comes after the FILE it is for",
            };
            return Err(Self::command().error(ErrorKind::ArgumentConflict, message));
        }
        Ok(cli)
    }

    pub fn only_files(&self) -> bool {
        self.labels.is_empty()
            && self.colormap.is_empty()
            && self.layout.is_none()
            && self.plane.is_none()
            && !self.mip
            && self.size.is_none()
    }
}

fn window_size(text: &str) -> Result<(f32, f32), String> {
    let expected = || "expected a width and height, e.g. 1440x900".to_string();
    let (width, height) = text.split_once(['x', 'X']).ok_or_else(expected)?;
    let length = |part: &str| {
        part.trim()
            .parse::<f32>()
            .ok()
            .filter(|length| length.is_finite() && *length > 0.0)
    };
    match (length(width), length(height)) {
        (Some(width), Some(height)) => Ok((width, height)),
        _ => Err(expected()),
    }
}

#[cfg(test)]
mod tests {
    use super::{Cli, LayoutName, PlaneName};
    use clap::CommandFactory;
    use illuminator_core::slicer::Colormap;
    use std::path::PathBuf;

    fn read(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_read_from(std::iter::once("illuminate").chain(args.iter().copied()))
    }

    #[test]
    fn the_options_are_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn reads_files_and_options() {
        let cli = read(&[
            "ct.nii.gz",
            "-l",
            "seg.nii.gz",
            "pet.nii.gz",
            "--labels=body.nii.gz",
            "--layout",
            "lightbox",
            "--plane",
            "coronal",
            "--size",
            "1200x800",
        ])
        .unwrap();
        let paths = |names: &[&str]| names.iter().map(PathBuf::from).collect::<Vec<_>>();
        assert_eq!(cli.files, paths(&["ct.nii.gz", "pet.nii.gz"]));
        assert_eq!(cli.labels, paths(&["seg.nii.gz", "body.nii.gz"]));
        assert_eq!(cli.colormaps, [None, None]);
        assert_eq!(cli.layout, Some(LayoutName::Lightbox));
        assert_eq!(cli.plane, Some(PlaneName::Coronal));
        assert_eq!(cli.size, Some((1200.0, 800.0)));
        assert!(!cli.mip && !cli.only_files());
        assert!(read(&["ct.nii.gz"]).unwrap().only_files());
    }

    #[test]
    fn colormaps_go_with_the_file_before_them() {
        let cli = read(&[
            "ct.nii.gz",
            "pet.nii.gz",
            "-c",
            "inferno",
            "-l",
            "seg.nii.gz",
            "dose.nii.gz",
            "--colormap=viridis",
        ])
        .unwrap();
        let expected = [None, Some(Colormap::Inferno), Some(Colormap::Viridis)];
        assert_eq!(cli.colormaps, expected);
        assert!(!cli.only_files());
        assert!(read(&["-c", "magma", "pet.nii.gz"]).is_err());
        assert!(read(&["pet.nii.gz", "-l", "seg.nii.gz", "-c", "magma"]).is_err());
    }

    #[test]
    fn rejects_bad_values() {
        for bad in [
            ["--size", "1200"],
            ["--size", "0x800"],
            ["--layout", "mosaic"],
            ["--colormap", "jet"],
        ] {
            assert!(read(&bad).is_err(), "{bad:?}");
        }
    }
}
