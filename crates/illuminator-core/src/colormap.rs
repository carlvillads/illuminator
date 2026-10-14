//! Colormaps: grayscale, viridis, plasma, magma, and inferno.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Colormap {
    #[default]
    Grayscale,
    Viridis,
    Inferno,
    Plasma,
    Magma,
}

impl Colormap {
    pub const ALL: [Self; 5] = [
        Self::Grayscale,
        Self::Viridis,
        Self::Plasma,
        Self::Magma,
        Self::Inferno,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Grayscale => "Grayscale",
            Self::Viridis => "Viridis",
            Self::Inferno => "Inferno",
            Self::Plasma => "Plasma",
            Self::Magma => "Magma",
        }
    }

    pub fn color(self, t: f64) -> [u8; 3] {
        let t = t.clamp(0.0, 1.0);
        let rgb = match self {
            Self::Grayscale => [t; 3],
            Self::Viridis => polynomial(&VIRIDIS, t),
            Self::Inferno => polynomial(&INFERNO, t),
            Self::Plasma => polynomial(&PLASMA, t),
            Self::Magma => polynomial(&MAGMA, t),
        };
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

fn polynomial(coefficients: &[[f64; 3]; 7], t: f64) -> [f64; 3] {
    coefficients.iter().rev().fold([0.0; 3], |sum, c| {
        std::array::from_fn(|channel| c[channel] + t * sum[channel])
    })
}

/// Polynomial colormap approximations by Matt Zucker: https://www.shadertoy.com/view/WlfXRN
const VIRIDIS: [[f64; 3]; 7] = [
    [0.2777273272234177, 0.005407344544966578, 0.3340998053353061],
    [0.1050930431085774, 1.404613529898575, 1.384590162594685],
    [-0.3308618287255563, 0.214847559468213, 0.09509516302823659],
    [-4.634230498983486, -5.799100973351585, -19.33244095627987],
    [6.228269936347081, 14.17993336680509, 56.69055260068105],
    [4.776384997670288, -13.74514537774601, -65.35303263337234],
    [-5.435455855934631, 4.645852612178535, 26.3124352495832],
];
/// Polynomial colormap approximations by Matt Zucker: https://www.shadertoy.com/view/WlfXRN
const PLASMA: [[f64; 3]; 7] = [
    [0.05873234392399702, 0.02333670892565664, 0.5433401826748754],
    [2.176514634195958, 0.2383834171260182, 0.7539604599784036],
    [-2.689460476458034, -7.455851135738909, 3.110799939717086],
    [6.130348345893603, 42.3461881477227, -28.51885465332158],
    [-11.10743619062271, -82.66631109428045, 60.13984767418263],
    [10.02306557647065, 71.4136177009535, -54.07218655560067],
    [-3.658713842777788, -22.93153465461149, 18.19190778539828],
];
/// Polynomial colormap approximations by Matt Zucker: https://www.shadertoy.com/view/WlfXRN
const MAGMA: [[f64; 3]; 7] = [
    [
        -0.002136485053939582,
        -0.000749655052795221,
        -0.005386127855323933,
    ],
    [0.2516605407371642, 0.6775232436837668, 2.494026599312351],
    [8.353717279216625, -3.577719514958484, 0.3144679030132573],
    [-27.66873308576866, 14.26473078096533, -13.64921318813922],
    [52.17613981234068, -27.94360607168351, 12.94416944238394],
    [-50.76852536473588, 29.04658282127291, 4.23415299384598],
    [18.65570506591883, -11.48977351997711, -5.601961508734096],
];
/// Polynomial colormap approximations by Matt Zucker: https://www.shadertoy.com/view/WlfXRN
const INFERNO: [[f64; 3]; 7] = [
    [
        0.0002189403691192265,
        0.001651004631001012,
        -0.01948089843709184,
    ],
    [0.1065134194856116, 0.5639564367884091, 3.932712388889277],
    [11.60249308247187, -3.972853965665698, -15.9423941062914],
    [-41.70399613139459, 17.43639888205313, 44.35414519872813],
    [77.162935699427, -33.40235894210092, -81.80730925738993],
    [-71.31942824499214, 32.62606426397723, 73.20951985803202],
    [25.13112622477341, -12.24266895238567, -23.07032500287172],
];

#[cfg(test)]
mod tests {
    use super::*;

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
        // Close to the real viridis endpoints, dark purple and yellow.
        assert_eq!(Colormap::Viridis.color(0.0), [71, 1, 85]);
        assert_eq!(Colormap::Viridis.color(1.0), [252, 231, 33]);
    }
}
