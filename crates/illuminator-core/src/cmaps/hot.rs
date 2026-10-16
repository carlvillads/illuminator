//! Hot colormap.
//! Based on the colormap of the same name from matplotlib <https://github.com/matplotlib/matplotlib>.

use super::{Cmap, Ramp};

pub(crate) const HOT: Cmap = Cmap {
    name: "Hot",
    id: "hot",
    desc: "Black through red and yellow to white",
    ramp: Ramp::Stops(&[
        (0.0, [0.0416, 0.0, 0.0]),
        (0.365079, [1.0, 0.0, 0.0]),
        (0.746032, [1.0, 1.0, 0.0]),
        (1.0, [1.0, 1.0, 1.0]),
    ]),
};
