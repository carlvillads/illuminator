//! Plasma colormap.
//! Polynomial approximation by Matt Zucker: <https://www.shadertoy.com/view/WlfXRN>
//! of <https://github.com/BIDS/colormap/blob/master/colormaps.py>

use super::{Cmap, Ramp};

pub(crate) const PLASMA: Cmap = Cmap {
    name: "Plasma",
    id: "plasma",
    desc: "Perceptually uniform. Dark blue through magenta to yellow",
    ramp: Ramp::Polynomial(&[
        [0.05873234392399702, 0.02333670892565664, 0.5433401826748754],
        [2.176514634195958, 0.2383834171260182, 0.7539604599784036],
        [-2.689460476458034, -7.455851135738909, 3.110799939717086],
        [6.130348345893603, 42.3461881477227, -28.51885465332158],
        [-11.10743619062271, -82.66631109428045, 60.13984767418263],
        [10.02306557647065, 71.4136177009535, -54.07218655560067],
        [-3.658713842777788, -22.93153465461149, 18.19190778539828],
    ]),
};
