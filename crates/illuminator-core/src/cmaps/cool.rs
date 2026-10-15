//! Cool colormap.
//! Based on the colormap of the same name from matplotlib <https://github.com/matplotlib/matplotlib>.

use super::{Cmap, Ramp};

pub(crate) const COOL: Cmap = Cmap {
    name: "Cool",
    id: "cool",
    ramp: Ramp::Stops(&[(0.0, [0.0, 1.0, 1.0]), (1.0, [1.0, 0.0, 1.0])]),
};
