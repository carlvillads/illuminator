//! Grayscale colormap.

use super::{Cmap, Ramp};

pub(crate) const GRAYSCALE: Cmap = Cmap {
    name: "Grayscale",
    id: "grayscale",
    ramp: Ramp::Stops(&[(0.0, [0.0; 3]), (1.0, [1.0; 3])]),
};
