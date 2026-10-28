//! Command line options.

use clap::builder::{PossibleValuesParser, TypedValueParser};
use clap::error::ErrorKind;
use clap::{CommandFactory, FromArgMatches, Parser, ValueEnum};
use illuminator_core::colormap::Colormap;
use illuminator_core::geometry::ViewPlane;
use std::ffi::OsString;
use std::path::PathBuf;

/// A viewer for NIfTI volumes and more.
#[derive(Debug, Parser)]
#[command(name = "illuminate", version, about)]
pub struct CommandLine {
    /// Volume(s) to open, opened in typed order.
    #[arg(value_name = "FILE")]
    pub files: Vec<PathBuf>,

    /// Volume to open as a label map. Opened on top of FILE(s).
    #[arg(short, long, value_name = "FILE")]
    pub labels: Vec<PathBuf>,

    /// Colormap of the FILE just before this argument.
    #[arg(short, long, value_parser = colormap_parser())]
    pub colormap: Vec<Colormap>,

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
    #[arg(long, value_name = "WIDTHxHEIGHT", value_parser = parse_window_size)]
    pub size: Option<(f32, f32)>,

    /// The colormap of each in `files` from the argument after it.
    #[arg(skip)]
    pub colormaps: Vec<Option<Colormap>>,
}

#[derive(Debug, Clone, Copy, PartialEq, ValueEnum)]
pub enum LayoutName {
    #[value(name = "ortho")]
    Orthogonal,
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

impl CommandLine {
    /// Reads commandline, prints help and version or error
    pub fn read() -> Self {
        Self::try_read_from(std::env::args_os()).unwrap_or_else(|error| error.exit())
    }

    pub fn try_read_from<T: Into<OsString> + Clone>(
        arguments: impl IntoIterator<Item = T>,
    ) -> Result<Self, clap::Error> {
        let matches = Self::command().try_get_matches_from(arguments)?;
        let mut command_line = Self::from_arg_matches(&matches)?;
        let argument_indices = |id: &str| -> Vec<usize> {
            matches
                .indices_of(id)
                .map(Iterator::collect)
                .unwrap_or_default()
        };
        let (files, labels) = (argument_indices("files"), argument_indices("labels"));
        let conflict = |message: &str| Self::command().error(ErrorKind::ArgumentConflict, message);
        command_line.colormaps = vec![None; command_line.files.len()];
        for (position, &colormap) in argument_indices("colormap")
            .into_iter()
            .zip(&command_line.colormap)
        {
            let file = files.iter().rposition(|&file| file < position);
            let label = labels.iter().rfind(|&&label| label < position);
            match (file, label) {
                (Some(file), label) if label.is_none_or(|&label| label < files[file]) => {
                    command_line.colormaps[file] = Some(colormap);
                }
                (_, Some(_)) => return Err(conflict("label maps do not accept the -c argument")),
                _ => return Err(conflict("the -c arg comes after the FILE it is for")),
            }
        }
        Ok(command_line)
    }

    pub fn has_only_files(&self) -> bool {
        self.labels.is_empty()
            && self.colormap.is_empty()
            && self.layout.is_none()
            && self.plane.is_none()
            && !self.mip
            && self.size.is_none()
    }
}

/// Colormap by its id.
fn colormap_parser() -> impl TypedValueParser<Value = Colormap> {
    PossibleValuesParser::new(Colormap::ALL.map(Colormap::id)).map(|id| {
        let colormap = Colormap::ALL.into_iter().find(|c| c.id() == id);
        colormap.expect("only ids pass the parser")
    })
}

fn parse_window_size(text: &str) -> Result<(f32, f32), String> {
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
    use super::{CommandLine, LayoutName, PlaneName};
    use clap::CommandFactory;
    use illuminator_core::colormap::Colormap;
    use std::path::PathBuf;

    fn parse_arguments(args: &[&str]) -> Result<CommandLine, clap::Error> {
        CommandLine::try_read_from(std::iter::once("illuminate").chain(args.iter().copied()))
    }

    #[test]
    fn the_options_are_consistent() {
        CommandLine::command().debug_assert();
    }

    #[test]
    fn reads_files_and_options() {
        let command_line = parse_arguments(&[
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
        assert_eq!(command_line.files, paths(&["ct.nii.gz", "pet.nii.gz"]));
        assert_eq!(command_line.labels, paths(&["seg.nii.gz", "body.nii.gz"]));
        assert_eq!(command_line.colormaps, [None, None]);
        assert_eq!(command_line.layout, Some(LayoutName::Lightbox));
        assert_eq!(command_line.plane, Some(PlaneName::Coronal));
        assert_eq!(command_line.size, Some((1200.0, 800.0)));
        assert!(!command_line.mip && !command_line.has_only_files());
        assert!(parse_arguments(&["ct.nii.gz"]).unwrap().has_only_files());
    }

    #[test]
    fn colormaps_go_with_the_file_before_them() {
        let command_line = parse_arguments(&[
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
        assert_eq!(command_line.colormaps, expected);
        assert!(!command_line.has_only_files());
        assert!(parse_arguments(&["-c", "magma", "pet.nii.gz"]).is_err());
        assert!(parse_arguments(&["pet.nii.gz", "-l", "seg.nii.gz", "-c", "magma"]).is_err());
    }

    #[test]
    fn rejects_bad_values() {
        for bad in [
            ["--size", "1200"],
            ["--size", "0x800"],
            ["--layout", "mosaic"],
            ["--colormap", "rainbow"],
        ] {
            assert!(parse_arguments(&bad).is_err(), "{bad:?}");
        }
    }
}
