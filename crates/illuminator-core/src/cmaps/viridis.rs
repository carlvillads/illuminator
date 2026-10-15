//! Viridis colormap.
//! Polynomial approximation by Matt Zucker: <https://www.shadertoy.com/view/WlfXRN>
//! of <https://github.com/BIDS/colormap/blob/master/colormaps.py>

use super::{Cmap, Ramp};

pub(crate) const VIRIDIS: Cmap = Cmap {
    name: "Viridis",
    id: "viridis",
    ramp: Ramp::Polynomial(&[
        [0.2777273272234177, 0.005407344544966578, 0.3340998053353061],
        [0.1050930431085774, 1.404613529898575, 1.384590162594685],
        [-0.3308618287255563, 0.214847559468213, 0.09509516302823659],
        [-4.634230498983486, -5.799100973351585, -19.33244095627987],
        [6.228269936347081, 14.17993336680509, 56.69055260068105],
        [4.776384997670288, -13.74514537774601, -65.35303263337234],
        [-5.435455855934631, 4.645852612178535, 26.3124352495832],
    ]),
};
