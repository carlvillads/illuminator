//! The app icons.
//!
//! Icons from IBM's Carbon Design System, licensed under Apache 2.0:
//! <https://carbondesignsystem.com>

use gpui_kit::assets::{Assets, IconNamed};
use gpui_kit::{AssetSource, Result, SharedString};
use std::borrow::Cow;

macro_rules! application_ui_icons {
    ($($name:ident => $file:literal,)*) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum ApplicationUiIcon {
            $($name,)*
        }

        impl ApplicationUiIcon {
            #[cfg(test)]
            const ALL: &[Self] = &[$(Self::$name,)*];
        }

        impl IconNamed for ApplicationUiIcon {
            fn path(self) -> SharedString {
                match self {
                    $(Self::$name => concat!("carbon/", $file),)*
                }
                .into()
            }
        }

        const ICON_FILES: &[(&str, &[u8])] = &[
            $((
                concat!("carbon/", $file),
                include_bytes!(concat!("../assets/icons/", $file)),
            ),)*
        ];
    };
}

application_ui_icons! {
    FitWindow => "window--auto.svg",
    Close => "close.svg",
    Compare => "hanging-protocol.svg",
    CornerText => "dicom--overlay.svg",
    Crosshair => "3D-cursor--alt.svg",
    FolderOpen => "folder--open.svg",
    Grid => "grid.svg",
    Lightbox => "thumbnail--2.svg",
    Help => "help.svg",
    CursorSynchronization => "sync-settings.svg",
    Orthogonal => "3D-MPR-Toggle.svg",
    Pause => "pause.svg",
    Play => "play.svg",
    Radiological => "reflect--horizontal.svg",
    SingleView => "array-users.svg",
    View => "view.svg",
    ViewOff => "view--off.svg",
    WindowPreset => "window--preset.svg",
    Reload => "renew--alt.svg",
    Mip => "shape--exclude.svg",
    Camera => "camera.svg",
    Expand => "maximize.svg",
    Theme => "brightness-contrast.svg",
}

const LIBRARY_ICON_OVERRIDES: &[(&str, &[u8])] = &[
    (
        "icons/check.svg",
        include_bytes!("../assets/icons/checkmark.svg"),
    ),
    (
        "icons/close.svg",
        include_bytes!("../assets/icons/close.svg"),
    ),
];

pub struct ApplicationAssets;

impl AssetSource for ApplicationAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        let bundled = ICON_FILES
            .iter()
            .chain(LIBRARY_ICON_OVERRIDES)
            .find(|(name, _)| *name == path);
        match bundled {
            Some((_, bytes)) => Ok(Some(Cow::Borrowed(*bytes))),
            None => Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = Assets.list(path)?;
        paths.extend(
            ICON_FILES
                .iter()
                .map(|(name, _)| *name)
                .filter(|name| name.starts_with(path))
                .map(SharedString::from),
        );
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_is_an_svg() {
        let paths = ApplicationUiIcon::ALL
            .iter()
            .map(|icon| icon.path().to_string());
        for path in paths.chain(
            LIBRARY_ICON_OVERRIDES
                .iter()
                .map(|(path, _)| path.to_string()),
        ) {
            let bytes = ApplicationAssets.load(&path).unwrap().unwrap();
            assert!(bytes.starts_with(b"<svg"), "{path}");
        }
    }
}
