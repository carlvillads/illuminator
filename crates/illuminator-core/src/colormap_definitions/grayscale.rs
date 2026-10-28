//! Grayscale colormap.

use super::{ColormapDefinition, Ramp};

pub(crate) const GRAYSCALE: ColormapDefinition = ColormapDefinition {
    name: "Grayscale",
    id: "grayscale",
    description: "Linear black to white",
    ramp: Ramp::Stops(&[(0.0, [0.0; 3]), (1.0, [1.0; 3])]),
};

pub(crate) const INVERSE_GRAYSCALE: ColormapDefinition = ColormapDefinition {
    name: "Inverse grayscale",
    id: "gray-r",
    description: "Linear white to black",
    ramp: Ramp::Stops(&[(0.0, [1.0; 3]), (1.0, [0.0; 3])]),
};
