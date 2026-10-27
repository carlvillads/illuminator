//! Bone colormap.
//! Based on the colormap of the same name from matplotlib <https://github.com/matplotlib/matplotlib>.

use super::{ColormapDefinition, Ramp};

pub(crate) const BONE: ColormapDefinition = ColormapDefinition {
    name: "Bone",
    id: "bone",
    description: "Grayscale with blue tint",
    ramp: Ramp::Stops(&[
        (0.0, [0.0, 0.0, 0.0]),
        (0.365079, [0.319444, 0.319444, 0.444444]),
        (0.746032, [0.652778, 0.777778, 0.777778]),
        (1.0, [1.0, 1.0, 1.0]),
    ]),
};
