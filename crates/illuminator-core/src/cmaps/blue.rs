//! Blue colormap.

use super::{Cmap, Ramp};

pub(crate) const BLUE: Cmap = Cmap {
    name: "Blue",
    id: "blue",
    ramp: Ramp::Stops(&[(0.0, [0.0, 0.0, 0.0]), (1.0, [0.0, 0.0, 1.0])]),
};
