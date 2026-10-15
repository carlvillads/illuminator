//! Colormaps.
//! Each map has its own file under `cmaps/`.

use crate::cmaps::{self, Cmap};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Colormap {
    #[default]
    Grayscale,
    Viridis,
    Plasma,
    Magma,
    Inferno,
    Hot,
    Bone,
    Cool,
    Turbo,
}

impl Colormap {
    pub const ALL: [Self; 9] = [
        Self::Grayscale,
        Self::Viridis,
        Self::Plasma,
        Self::Magma,
        Self::Inferno,
        Self::Hot,
        Self::Bone,
        Self::Cool,
        Self::Turbo,
    ];

    fn cmap(self) -> &'static Cmap {
        match self {
            Self::Grayscale => &cmaps::GRAYSCALE,
            Self::Viridis => &cmaps::VIRIDIS,
            Self::Plasma => &cmaps::PLASMA,
            Self::Magma => &cmaps::MAGMA,
            Self::Inferno => &cmaps::INFERNO,
            Self::Hot => &cmaps::HOT,
            Self::Bone => &cmaps::BONE,
            Self::Cool => &cmaps::COOL,
            Self::Turbo => &cmaps::TURBO,
        }
    }

    pub fn name(self) -> &'static str {
        self.cmap().name
    }

    pub fn id(self) -> &'static str {
        self.cmap().id
    }

    /// The colormap after the current one in [`Self::ALL`].
    pub fn next(self) -> Self {
        let current = Self::ALL.iter().position(|&c| c == self);
        Self::ALL[current.map_or(0, |position| (position + 1) % Self::ALL.len())]
    }

    pub fn color(self, t: f64) -> [u8; 3] {
        let rgb = self.cmap().ramp.rgb(t.clamp(0.0, 1.0));
        // `as u8` turns nans into 0 making invalid values black
        rgb.map(|channel| (channel.clamp(0.0, 1.0) * 255.0).round() as u8)
    }
}

/// A colormap sampled into a table
pub(crate) enum ColorTable {
    Gray,
    Sampled(Vec<[u8; 3]>),
}

impl ColorTable {
    const SIZE: usize = 1024;

    pub(crate) fn new(colormap: Colormap) -> Self {
        match colormap {
            Colormap::Grayscale => Self::Gray,
            _ => Self::Sampled(
                (0..Self::SIZE)
                    .map(|i| colormap.color(i as f64 / (Self::SIZE - 1) as f64))
                    .collect(),
            ),
        }
    }

    pub(crate) fn color(&self, t: f64) -> [u8; 3] {
        match self {
            Self::Gray => Colormap::Grayscale.color(t),
            Self::Sampled(colors) => {
                colors[(t.clamp(0.0, 1.0) * (Self::SIZE - 1) as f64).round() as usize]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmaps::Ramp;

    #[test]
    fn color_tables_match_the_colormaps() {
        for colormap in Colormap::ALL {
            let table = ColorTable::new(colormap);
            for step in 0..=1000 {
                let t = step as f64 / 1000.0;
                let (sampled, exact) = (table.color(t), colormap.color(t));
                let off = (0..3).map(|c| sampled[c].abs_diff(exact[c])).max().unwrap();
                assert!(
                    off <= 1,
                    "{} at {t}: {sampled:?} vs {exact:?}",
                    colormap.name()
                );
            }
        }
    }

    #[test]
    fn colormaps() {
        assert_eq!(Colormap::Grayscale.color(0.0), [0, 0, 0]);
        assert_eq!(Colormap::Grayscale.color(1.0), [255, 255, 255]);
        assert_eq!(Colormap::Grayscale.color(7.0), [255, 255, 255]);
        assert_eq!(Colormap::Grayscale.color(f64::NAN), [0, 0, 0]);
        assert_eq!(Colormap::Viridis.color(0.0), [71, 1, 85]);
        assert_eq!(Colormap::Viridis.color(1.0), [252, 231, 33]);
        assert_eq!(Colormap::Hot.color(f64::NAN), [0, 0, 0]);
    }

    #[test]
    fn stops_rise_from_0_to_1() {
        for colormap in Colormap::ALL {
            if let Ramp::Stops(stops) = colormap.cmap().ramp {
                let name = colormap.name();
                assert_eq!(stops.first().map(|stop| stop.0), Some(0.0), "{name}");
                assert_eq!(stops.last().map(|stop| stop.0), Some(1.0), "{name}");
                assert!(stops.is_sorted_by(|a, b| a.0 < b.0), "{name}");
            }
        }
    }

    #[test]
    fn ids_are_unique() {
        for (n, colormap) in Colormap::ALL.into_iter().enumerate() {
            let id = colormap.id();
            assert!(Colormap::ALL[..n].iter().all(|c| c.id() != id), "{id}");
        }
    }

    #[test]
    fn next_wraps_around() {
        assert_eq!(Colormap::Grayscale.next(), Colormap::Viridis);
        assert_eq!(Colormap::Turbo.next(), Colormap::Grayscale);
    }
}
