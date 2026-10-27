//! Inferno colormap.
//! Polynomial approximation by Matt Zucker: <https://www.shadertoy.com/view/WlfXRN>
//! of <https://github.com/BIDS/colormap/blob/master/colormaps.py>

use super::{ColormapDefinition, Ramp};

pub(crate) const INFERNO: ColormapDefinition = ColormapDefinition {
    name: "Inferno",
    id: "inferno",
    description: "Perceptually uniform. Black through purple and orange to yellow",
    ramp: Ramp::Polynomial(&[
        [
            0.0002189403691192265,
            0.001651004631001012,
            -0.01948089843709184,
        ],
        [0.1065134194856116, 0.5639564367884091, 3.932712388889277],
        [11.60249308247187, -3.972853965665698, -15.9423941062914],
        [-41.70399613139459, 17.43639888205313, 44.35414519872813],
        [77.162935699427, -33.40235894210092, -81.80730925738993],
        [-71.31942824499214, 32.62606426397723, 73.20951985803202],
        [25.13112622477341, -12.24266895238567, -23.07032500287172],
    ]),
};
