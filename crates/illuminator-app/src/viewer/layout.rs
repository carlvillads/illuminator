//! layout.rs
//!
//! Viewer layout.
//!

use super::*;

impl Viewer {
    /// Every view on screen in reading order
    pub(super) fn views(&self, window: &Window) -> Vec<View> {
        let count = self.layers.len();
        if count == 0 {
            return Vec::new();
        }
        let area = view_area(window);
        let mut ortho: Vec<Cell> = ViewPlane::ALL
            .map(|axis| Cell {
                base: 0,
                axis,
                mip: false,
                slice: None,
            })
            .to_vec();
        if self.show_mip {
            ortho.push(Cell {
                base: 0,
                axis: ViewPlane::Coronal,
                mip: true,
                slice: None,
            });
        }
        let mip = self.show_mip;
        let cells: Vec<Cell> = match self.layout {
            Layout::Ortho => ortho,
            Layout::Compare => ortho
                .iter()
                .flat_map(|&cell| {
                    (0..count).map(move |index| Cell {
                        base: index,
                        ..cell
                    })
                })
                .collect(),
            Layout::Grid(axis) => (0..count)
                .map(|index| Cell {
                    base: index,
                    axis,
                    mip,
                    slice: None,
                })
                .collect(),
            Layout::Single { axis, layer } => {
                let base = layer.filter(|&index| index < count).unwrap_or(0);
                vec![Cell {
                    base,
                    axis,
                    mip,
                    slice: None,
                }]
            }
            Layout::Lightbox(axis) => {
                let (current, slices) = self.layers[0].display.slice_position(axis);
                let (columns, rows) = self.lightbox_grid;
                lightbox_slices(current, slices, self.lightbox_step, columns * rows)
                    .into_iter()
                    .map(|slice| Cell {
                        base: 0,
                        axis,
                        mip: false,
                        slice: Some(slice),
                    })
                    .collect()
            }
        };
        let (columns, rows) = match self.layout {
            // a short last page keeps the size of the views
            Layout::Lightbox(_) => self.lightbox_grid,
            Layout::Compare => (count, cells.len().div_ceil(count)),
            _ => {
                let columns = grid_columns(cells.len(), area);
                (columns, cells.len().div_ceil(columns))
            }
        };
        let cell_size = size(area.width / columns as f32, area.height / rows as f32);
        let scale = window.scale_factor();
        let overlaid = self.layout.overlaid();
        cells
            .into_iter()
            .enumerate()
            .map(|(n, cell)| {
                let Cell {
                    base,
                    axis,
                    mip,
                    slice,
                } = cell;
                let display = &self.layers[base].display;
                let plane = match slice {
                    Some(slice) => display.slice_plane(axis, slice, self.radiological),
                    None => display.plane(axis, self.radiological),
                };
                let (width, height) = (
                    (cell_size.width * scale) as usize,
                    (cell_size.height * scale) as usize,
                );
                let whole = fit_size(&plane, width, height);
                let fit = (
                    whole.0 as f64 / width.max(1) as f64,
                    whole.1 as f64 / height.max(1) as f64,
                );
                let zoom = self.zoom[axis.index()];
                let visible = zoom.visible(fit);
                // the visible part may have another shape than the whole plane so fit it again
                let image_pixels = fit_size(&visible.apply(plane), width, height);
                View {
                    origin: point(
                        (n % columns) as f32 * cell_size.width,
                        (n / columns) as f32 * cell_size.height,
                    ),
                    size: cell_size,
                    axis,
                    base,
                    layers: if overlaid { 0..count } else { base..base + 1 },
                    image_pixels,
                    image_size: size(image_pixels.0 as f32 / scale, image_pixels.1 as f32 / scale),
                    zoom,
                    fit,
                    visible,
                    mip,
                    slot: match slice {
                        Some(_) => LIGHTBOX_SLOTS + n,
                        None => axis.index() + if mip { 3 } else { 0 },
                    },
                    plane,
                    slice,
                    current: slice.is_none_or(|slice| slice == display.slice_position(axis).0),
                }
            })
            .collect()
    }

    pub(super) fn hit(&self, position: Point<Pixels>, window: &Window) -> Option<Hit> {
        let x = f32::from(position.x);
        let y = f32::from(position.y) - TOOLBAR_HEIGHT;
        self.views(window).into_iter().find_map(|view| {
            let (local_x, local_y) = (x - view.origin.x, y - view.origin.y);
            if local_x < 0.0
                || local_y < 0.0
                || local_x >= view.size.width
                || local_y >= view.size.height
            {
                return None;
            }
            let offset = view.image_offset();
            let in_view = (
                f64::from((local_x - offset.x) / view.image_size.width),
                f64::from(1.0 - (local_y - offset.y) / view.image_size.height),
            );
            let (s, t) = view.visible.to_plane(in_view);
            Some(Hit {
                base: view.base,
                axis: view.axis,
                mip: view.mip,
                s,
                t,
                in_view,
                image_size: view.image_size,
                fit: view.fit,
                plane: view.plane,
            })
        })
    }

    pub(super) fn set_lightbox_grid(&mut self, grid: (usize, usize), cx: &mut Context<Self>) {
        self.lightbox_grid = grid;
        // images of views that are gone
        let keep = LIGHTBOX_SLOTS + grid.0 * grid.1;
        let unused: Vec<Panel> = self
            .layers
            .iter_mut()
            .flat_map(|layer| layer.panels.split_off(keep.min(layer.panels.len())))
            .collect();
        self.retire(unused);
        cx.notify();
    }

    pub(super) fn slice_stride(&self) -> f64 {
        match self.layout {
            Layout::Lightbox(_) => self.lightbox_step as f64,
            _ => 1.0,
        }
    }

    /// Steps per Shift+arrow
    pub(super) fn page_steps(&self) -> f64 {
        match self.layout {
            Layout::Lightbox(_) => (self.lightbox_grid.0 * self.lightbox_grid.1) as f64,
            _ => 10.0,
        }
    }
}

/// What one view on screen shows before it gets a place and a size.
#[derive(Clone, Copy)]
struct Cell {
    /// The layer whose plane the view shows
    base: usize,
    axis: ViewPlane,
    mip: bool,
    /// The slice in lightbox views
    slice: Option<usize>,
}

fn lightbox_slices(current: usize, count: usize, step: usize, cells: usize) -> Vec<usize> {
    let (step, cells) = (step.max(1), cells.max(1));
    let phase = current % step;
    let page = (current - phase) / step / cells;
    let first = phase + page * cells * step;
    (0..cells)
        .map(|n| first + n * step)
        .take_while(|&slice| slice < count)
        .collect()
}

fn grid_columns(count: usize, area: Size<f32>) -> usize {
    let side = |columns: usize| {
        let rows = count.div_ceil(columns);
        (area.width / columns as f32).min(area.height / rows as f32)
    };
    (1..=count.max(1))
        .max_by(|&a, &b| side(a).total_cmp(&side(b)))
        .unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::{grid_columns, lightbox_slices, size};

    #[test]
    fn lightbox_pages_hold_the_cursor() {
        assert_eq!(lightbox_slices(0, 10, 1, 4), [0, 1, 2, 3]);
        assert_eq!(lightbox_slices(5, 10, 1, 4), [4, 5, 6, 7]);
        assert_eq!(lightbox_slices(9, 10, 1, 4), [8, 9]);
        assert_eq!(lightbox_slices(5, 10, 2, 4), [1, 3, 5, 7]);
        assert_eq!(lightbox_slices(9, 10, 2, 4), [9]);
        assert_eq!(lightbox_slices(4, 10, 2, 4), [0, 2, 4, 6]);
    }

    #[test]
    fn grids_make_cells_as_large_as_possible() {
        let laptop = size(1120.0, 840.0);
        assert_eq!(grid_columns(1, laptop), 1);
        assert_eq!(grid_columns(3, laptop), 2);
        assert_eq!(grid_columns(4, laptop), 2);
        assert_eq!(grid_columns(3, size(1600.0, 500.0)), 3);
    }
}
