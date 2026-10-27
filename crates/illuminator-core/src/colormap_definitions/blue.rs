//! Blue colormap.

use super::{ColormapDefinition, Ramp};

pub(crate) const BLUE: ColormapDefinition = ColormapDefinition {
    name: "Blue",
    id: "blue",
    description: "Black to blue",
    ramp: Ramp::Stops(&[(0.0, [0.0, 0.0, 0.0]), (1.0, [0.0, 0.0, 1.0])]),
};
