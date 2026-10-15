//! The colors of each colormap.

mod bone;
mod cool;
mod grayc;
mod grayscale;
mod hot;
mod inferno;
mod lipari;
mod magma;
mod navia;
mod plasma;
mod turbo;
mod viridis;

pub(crate) use bone::BONE;
pub(crate) use cool::COOL;
pub(crate) use grayc::GRAYC;
pub(crate) use grayscale::{GRAYSCALE, INVERSE_GRAYSCALE};
pub(crate) use hot::HOT;
pub(crate) use inferno::INFERNO;
pub(crate) use lipari::LIPARI;
pub(crate) use magma::MAGMA;
pub(crate) use navia::NAVIA;
pub(crate) use plasma::PLASMA;
pub(crate) use turbo::TURBO;
pub(crate) use viridis::VIRIDIS;

pub(crate) struct Cmap {
    pub(crate) name: &'static str,
    pub(crate) id: &'static str,
    pub(crate) ramp: Ramp,
}

pub(crate) enum Ramp {
    /// Polynomial coefficients
    Polynomial(&'static [[f64; 3]]),
    Stops(&'static [(f64, [f64; 3])]),
    Table(&'static [[f64; 3]]),
}

impl Ramp {
    pub(crate) fn rgb(&self, t: f64) -> [f64; 3] {
        match *self {
            Self::Polynomial(coefficients) => coefficients.iter().rev().fold([0.0; 3], |sum, c| {
                std::array::from_fn(|channel| c[channel] + t * sum[channel])
            }),
            Self::Stops(stops) => {
                // A nan t gives nan colors
                let i = stops[1..]
                    .partition_point(|&(at, _)| at < t)
                    .min(stops.len() - 2);
                let ((from, low), (to, high)) = (stops[i], stops[i + 1]);
                mix(low, high, (t - from) / (to - from))
            }
            Self::Table(colors) => {
                let x = t * (colors.len() - 1) as f64;
                let i = (x as usize).min(colors.len() - 2);
                mix(colors[i], colors[i + 1], x - i as f64)
            }
        }
    }
}

// The color `f` of the way from `low` to `high`
fn mix(low: [f64; 3], high: [f64; 3], f: f64) -> [f64; 3] {
    std::array::from_fn(|channel| low[channel] + f * (high[channel] - low[channel]))
}
