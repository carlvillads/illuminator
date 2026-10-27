//! Red colormap.

use super::{ColormapDefinition, Ramp};

pub(crate) const RED: ColormapDefinition = ColormapDefinition {
    name: "Red",
    id: "red",
    description: "Black to red",
    ramp: Ramp::Stops(&[(0.0, [0.0, 0.0, 0.0]), (1.0, [1.0, 0.0, 0.0])]),
};
