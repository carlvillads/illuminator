//! Colormaps.
//! Each map has its own file under `colormap_definitions/`.

use crate::colormap_definitions::{self, ColormapDefinition};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Colormap {
    #[default]
    Grayscale,
    InverseGrayscale,
    Red,
    Blue,
    Viridis,
    Plasma,
    Magma,
    Inferno,
    Hot,
    Bone,
    Cool,
    Turbo,
    GrayC,
    Lipari,
    Navia,
    Berlin,
    Vik,
}

impl Colormap {
    pub const ALL: [Self; 17] = [
        Self::Grayscale,
        Self::InverseGrayscale,
        Self::Red,
        Self::Blue,
        Self::Viridis,
        Self::Plasma,
        Self::Magma,
        Self::Inferno,
        Self::Hot,
        Self::Bone,
        Self::Cool,
        Self::Turbo,
        Self::GrayC,
        Self::Lipari,
        Self::Navia,
        Self::Berlin,
        Self::Vik,
    ];

    fn definition(self) -> &'static ColormapDefinition {
        match self {
            Self::Grayscale => &colormap_definitions::GRAYSCALE,
            Self::InverseGrayscale => &colormap_definitions::INVERSE_GRAYSCALE,
            Self::Red => &colormap_definitions::RED,
            Self::Blue => &colormap_definitions::BLUE,
            Self::Viridis => &colormap_definitions::VIRIDIS,
            Self::Plasma => &colormap_definitions::PLASMA,
            Self::Magma => &colormap_definitions::MAGMA,
            Self::Inferno => &colormap_definitions::INFERNO,
            Self::Hot => &colormap_definitions::HOT,
            Self::Bone => &colormap_definitions::BONE,
            Self::Cool => &colormap_definitions::COOL,
            Self::Turbo => &colormap_definitions::TURBO,
            Self::GrayC => &colormap_definitions::GRAYC,
            Self::Lipari => &colormap_definitions::LIPARI,
            Self::Navia => &colormap_definitions::NAVIA,
            Self::Berlin => &colormap_definitions::BERLIN,
            Self::Vik => &colormap_definitions::VIK,
        }
    }

    pub fn name(self) -> &'static str {
        self.definition().name
    }

    pub fn id(self) -> &'static str {
        self.definition().id
    }

    pub fn description(self) -> &'static str {
        self.definition().description
    }

    /// The colormap after the current one in [`Self::ALL`].
    pub fn next(self) -> Self {
        let current = Self::ALL.iter().position(|&c| c == self);
        Self::ALL[current.map_or(0, |position| (position + 1) % Self::ALL.len())]
    }

    pub fn color(self, t: f64) -> [u8; 3] {
        let rgb = self.definition().ramp.rgb(t.clamp(0.0, 1.0));
        // `as u8` turns nans into 0 making invalid values black
        rgb.map(|channel| (channel.clamp(0.0, 1.0) * 255.0).round() as u8)
    }
}

/// A colormap sampled into a table
pub(crate) enum ColorTable {
    Grayscale,
    Sampled(Vec<[u8; 3]>),
}

impl ColorTable {
    const LENGTH: usize = 1024;

    pub(crate) fn new(colormap: Colormap) -> Self {
        match colormap {
            Colormap::Grayscale => Self::Grayscale,
            _ => Self::Sampled(
                (0..Self::LENGTH)
                    .map(|i| colormap.color(i as f64 / (Self::LENGTH - 1) as f64))
                    .collect(),
            ),
        }
    }

    pub(crate) fn color(&self, t: f64) -> [u8; 3] {
        match self {
            Self::Grayscale => Colormap::Grayscale.color(t),
            Self::Sampled(colors) => {
                colors[(t.clamp(0.0, 1.0) * (Self::LENGTH - 1) as f64).round() as usize]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::colormap_definitions::Ramp;

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
        assert_eq!(Colormap::Red.color(1.0), [255, 0, 0]);
        assert_eq!(Colormap::Blue.color(0.5), [0, 0, 128]);
        assert_eq!(Colormap::InverseGrayscale.color(0.0), [255, 255, 255]);
        assert_eq!(Colormap::InverseGrayscale.color(1.0), [0, 0, 0]);
    }

    #[test]
    fn stops_rise_from_0_to_1() {
        for colormap in Colormap::ALL {
            if let Ramp::Stops(stops) = colormap.definition().ramp {
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
        assert_eq!(Colormap::Grayscale.next(), Colormap::InverseGrayscale);
        assert_eq!(Colormap::Vik.next(), Colormap::Grayscale);
    }

    #[test]
    fn descriptions_are_one_short_sentence() {
        for colormap in Colormap::ALL {
            let description = colormap.description();
            assert!(description.len() <= 80, "{description}");
        }
    }
}
