//! Cool colormap.
//! Based on the colormap of the same name from matplotlib <https://github.com/matplotlib/matplotlib>.

use super::{ColormapDefinition, Ramp};

pub(crate) const COOL: ColormapDefinition = ColormapDefinition {
    name: "Cool",
    id: "cool",
    description: "Cyan to magenta",
    ramp: Ramp::Stops(&[(0.0, [0.0, 1.0, 1.0]), (1.0, [1.0, 0.0, 1.0])]),
};
