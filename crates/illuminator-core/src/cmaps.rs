//! The colors of each colormap.

mod bone;
mod cool;
mod grayscale;
mod hot;
mod inferno;
mod magma;
mod plasma;
mod turbo;
mod viridis;

pub(crate) use bone::BONE;
pub(crate) use cool::COOL;
pub(crate) use grayscale::GRAYSCALE;
pub(crate) use hot::HOT;
pub(crate) use inferno::INFERNO;
pub(crate) use magma::MAGMA;
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
                let f = (t - from) / (to - from);
                std::array::from_fn(|channel| low[channel] + f * (high[channel] - low[channel]))
            }
        }
    }
}
