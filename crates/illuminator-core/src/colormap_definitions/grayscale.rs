//! Grayscale colormap.

use super::{Cmap, Ramp};

pub(crate) const GRAYSCALE: Cmap = Cmap {
    name: "Grayscale",
    id: "grayscale",
    desc: "Linear black to white",
    ramp: Ramp::Stops(&[(0.0, [0.0; 3]), (1.0, [1.0; 3])]),
};

pub(crate) const INVERSE_GRAYSCALE: Cmap = Cmap {
    name: "Inverse grayscale",
    id: "gray-r",
    desc: "Linear white to black",
    ramp: Ramp::Stops(&[(0.0, [1.0; 3]), (1.0, [0.0; 3])]),
};
