//! Magma colormap.
//! Polynomial approximation by Matt Zucker: <https://www.shadertoy.com/view/WlfXRN>
//! of <https://github.com/BIDS/colormap/blob/master/colormaps.py>

use super::{Cmap, Ramp};

pub(crate) const MAGMA: Cmap = Cmap {
    name: "Magma",
    id: "magma",
    desc: "Perceptually uniform. Black through purple and pink to pale yellow",
    ramp: Ramp::Polynomial(&[
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
    ]),
};
