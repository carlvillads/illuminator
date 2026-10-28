//! Hot colormap.
//! Based on the colormap of the same name from matplotlib <https://github.com/matplotlib/matplotlib>.

use super::{ColormapDefinition, Ramp};

pub(crate) const HOT: ColormapDefinition = ColormapDefinition {
    name: "Hot",
    id: "hot",
    description: "Black through red and yellow to white",
    ramp: Ramp::Stops(&[
        (0.0, [0.0416, 0.0, 0.0]),
        (0.365079, [1.0, 0.0, 0.0]),
        (0.746032, [1.0, 1.0, 0.0]),
        (1.0, [1.0, 1.0, 1.0]),
    ]),
};
