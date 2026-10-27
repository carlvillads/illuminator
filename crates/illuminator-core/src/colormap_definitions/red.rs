//! Red colormap.

use super::{Cmap, Ramp};

pub(crate) const RED: Cmap = Cmap {
    name: "Red",
    id: "red",
    desc: "Black to red",
    ramp: Ramp::Stops(&[(0.0, [0.0, 0.0, 0.0]), (1.0, [1.0, 0.0, 0.0])]),
};
