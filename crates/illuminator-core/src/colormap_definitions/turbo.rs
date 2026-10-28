//! Turbo colormap.
//!
//! Polynomial approximation by Ruofei Du of Anton Mikhailov's Turbo.
//! Copyright 2019 Google LLC, Apache-2.0:
//! <https://gist.github.com/mikhailov-work/0d177465a8151eb6ede1768d51d476c7>
//! <https://research.google/blog/turbo-an-improved-rainbow-colormap-for-visualization/>

use super::{ColormapDefinition, Ramp};

pub(crate) const TURBO: ColormapDefinition = ColormapDefinition {
    name: "Turbo",
    id: "turbo",
    description: "Not perceptually uniform. Smooth rainbow with high contrast",
    ramp: Ramp::Polynomial(&[
        [0.13572138, 0.09140261, 0.10667330],
        [4.61539260, 2.19418839, 12.64194608],
        [-42.66032258, 4.84296658, -60.58204836],
        [132.13108234, -14.18503333, 110.36276771],
        [-152.94239396, 4.27729857, -89.90310912],
        [59.28637943, 2.82956604, 27.34824973],
    ]),
};
