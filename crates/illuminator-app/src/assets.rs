//! Icons from IBM's Carbon Design System, licensed under Apache 2.0: https://carbondesignsystem.com

use gpui_kit::assets::{Assets, IconNamed};
use gpui_kit::{AssetSource, Result, SharedString};
use std::borrow::Cow;

macro_rules! carbon_icons {
    ($($name:ident => $file:literal,)*) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum Carbon {
            $($name,)*
        }

        impl Carbon {
            #[cfg(test)]
            const ALL: &[Self] = &[$(Self::$name,)*];
        }

        impl IconNamed for Carbon {
            fn path(self) -> SharedString {
                match self {
                    $(Self::$name => concat!("carbon/", $file),)*
                }
                .into()
            }
        }

        const FILES: &[(&str, &[u8])] = &[
            $((
                concat!("carbon/", $file),
                include_bytes!(concat!("../assets/icons/", $file)),
            ),)*
        ];
    };
}

carbon_icons! {
    AutoWindow => "window--auto.svg",
    Close => "close.svg",
    Compare => "hanging-protocol.svg",
    CornerText => "text-annotation-toggle.svg",
    Crosshair => "3D-cursor--alt.svg",
    FolderOpen => "folder--open.svg",
    Grid => "grid.svg",
    Help => "help.svg",
    Link => "sync-settings.svg",
    Ortho => "3D-MPR-Toggle.svg",
    Pause => "pause.svg",
    Play => "play.svg",
    Radiological => "reflect--horizontal.svg",
    Single => "array-users.svg",
    View => "view.svg",
    ViewOff => "view--off.svg",
    WindowPreset => "window--preset.svg",
    Reload => "renew--alt.svg",
}

const OVERRIDES: &[(&str, &[u8])] = &[
    (
        "icons/check.svg",
        include_bytes!("../assets/icons/checkmark.svg"),
    ),
    (
        "icons/close.svg",
        include_bytes!("../assets/icons/close.svg"),
    ),
];

pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        let ours = FILES
            .iter()
            .chain(OVERRIDES)
            .find(|(name, _)| *name == path);
        match ours {
            Some((_, bytes)) => Ok(Some(Cow::Borrowed(*bytes))),
            None => Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = Assets.list(path)?;
        paths.extend(
            FILES
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
        let paths = Carbon::ALL.iter().map(|icon| icon.path().to_string());
        for path in paths.chain(OVERRIDES.iter().map(|(path, _)| path.to_string())) {
            let bytes = AppAssets.load(&path).unwrap().unwrap();
            assert!(bytes.starts_with(b"<svg"), "{path}");
        }
    }
}
