use gpui_kit::assets::{Assets, icon_assets};
use gpui_kit::{AssetSource, Result, SharedString};
use std::borrow::Cow;

icon_assets!(
    ToolbarIcons,
    [
        Columns3,
        Crosshair,
        FlipHorizontal2,
        Grid3x3,
        LayoutGrid,
        Link
    ]
);

pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(bytes) = ToolbarIcons.load(path)? {
            return Ok(Some(bytes));
        }
        Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = Assets.list(path)?;
        paths.extend(ToolbarIcons.list(path)?);
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}
