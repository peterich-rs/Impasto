//! Conservative painter-order dependencies within a single render target.

use std::{collections::HashMap, hash::Hash};

const SIDE: usize = 32;

#[derive(Clone, Copy)]
pub(crate) struct Bounds(pub [f32; 4]);

impl Bounds {
    pub(crate) fn overlaps(self, other: Self) -> bool {
        let [x0, y0, x1, y1] = self.0;
        let [a0, b0, a1, b1] = other.0;
        x0 < a1 && a0 < x1 && y0 < b1 && b0 < y1
    }

    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "tile coordinates are clamped to the grid"
    )]
    fn tiles(self, size: [u32; 2]) -> impl Iterator<Item = usize> {
        let [x0, y0, x1, y1] = self.0;
        let tile = |coordinate: f32, dimension: u32| {
            (coordinate * SIDE as f32 / dimension.max(1) as f32).clamp(0.0, (SIDE - 1) as f32)
                as usize
        };
        let x = tile(x0, size[0])..=tile(x1, size[0]);
        let y = tile(y0, size[1])..=tile(y1, size[1]);
        y.flat_map(move |y| x.clone().map(move |x| y * SIDE + x))
    }
}

pub(crate) struct SpatialBatcher<K> {
    size: [u32; 2],
    epochs: [usize; SIDE * SIDE],
    active: HashMap<K, usize>,
    segment_start: usize,
}

impl<K: Eq + Hash> SpatialBatcher<K> {
    pub(crate) fn new(size: [u32; 2]) -> Self {
        Self {
            size,
            epochs: [0; SIDE * SIDE],
            active: HashMap::new(),
            segment_start: 0,
        }
    }

    pub(crate) fn barrier(&mut self, next: usize) {
        self.segment_start = next;
        self.active.clear();
    }

    pub(crate) fn append(
        &mut self,
        key: K,
        bounds: Bounds,
        next: usize,
        compatible: impl FnOnce(usize, bool) -> bool,
    ) -> usize {
        let dep = bounds
            .tiles(self.size)
            .map(|tile| self.epochs[tile])
            .max()
            .unwrap_or(0);
        let index = self
            .active
            .get(&key)
            .copied()
            .filter(|&candidate| {
                candidate >= self.segment_start && compatible(candidate, dep > candidate)
            })
            .unwrap_or(next);
        if index == next {
            self.active.insert(key, index);
        }
        for tile in bounds.tiles(self.size) {
            self.epochs[tile] = self.epochs[tile].max(index);
        }
        index
    }
}

/// A row index for bounded scans over Canvas shapes waiting on a pass.
pub(crate) struct OverlapRows {
    height: u32,
    rows: [Vec<Bounds>; SIDE],
}

impl OverlapRows {
    pub(crate) fn new(height: u32) -> Self {
        Self {
            height,
            rows: std::array::from_fn(|_| Vec::new()),
        }
    }

    pub(crate) fn clear(&mut self) {
        self.rows.iter_mut().for_each(Vec::clear);
    }

    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "tile coordinates are clamped to the grid"
    )]
    fn rows(&self, Bounds([_, min, _, max]): Bounds) -> std::ops::RangeInclusive<usize> {
        let row = |y: f32| {
            (y * SIDE as f32 / self.height.max(1) as f32).clamp(0.0, (SIDE - 1) as f32) as usize
        };
        row(min)..=row(max)
    }

    pub(crate) fn insert(&mut self, bounds: Bounds) {
        for row in self.rows(bounds) {
            self.rows[row].push(bounds);
        }
    }

    pub(crate) fn overlaps(&self, bounds: Bounds) -> bool {
        self.rows(bounds)
            .any(|row| self.rows[row].iter().any(|&other| other.overlaps(bounds)))
    }
}
