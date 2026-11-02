//! Fenwick prefix of row heights. Index 0 of the tree is unused.
//! `sum(k)` is the first `k` heights.

/// Largest power of two that is `<= n`. Zero when `n` is zero.
pub fn highest_power_of_two(n: usize) -> usize {
    if n == 0 {
        return 0;
    }
    let mut bit = 1usize;
    while bit.saturating_mul(2) <= n {
        bit = bit.saturating_mul(2);
    }
    bit
}

#[derive(Clone, Debug, Default)]
pub struct PrefixHeights {
    values: Vec<f32>,
    tree: Vec<f32>,
}

impl PrefixHeights {
    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn height_at(&self, index: usize) -> f32 {
        self.values.get(index).copied().unwrap_or(0.0)
    }

    pub fn rebuild(&mut self, heights: &[f32]) {
        self.values.clear();
        self.values.extend_from_slice(heights);
        self.reindex();
    }

    fn reindex(&mut self) {
        self.tree.clear();
        self.tree.resize(self.values.len() + 1, 0.0);
        for index in 0..self.values.len() {
            let height = self.values[index];
            self.add_delta(index, height);
        }
    }

    /// Point-update the height at `index` by `delta`.
    pub fn add(&mut self, index: usize, delta: f32) {
        if index >= self.values.len() || delta == 0.0 {
            return;
        }
        self.values[index] += delta;
        self.add_delta(index, delta);
    }

    fn add_delta(&mut self, index: usize, delta: f32) {
        let mut cursor = index + 1;
        while cursor < self.tree.len() {
            self.tree[cursor] += delta;
            cursor += cursor.isolate_lowest_one();
        }
    }

    /// Sum of the first `k` items. `k` counts from 1.
    pub fn sum(&self, mut k: usize) -> f32 {
        let mut total = 0.0;
        while k > 0 {
            total += self.tree.get(k).copied().unwrap_or(0.0);
            k -= k.isolate_lowest_one();
        }
        total
    }

    pub fn item_start(&self, index: usize) -> f32 {
        self.sum(index)
    }

    pub fn total(&self) -> f32 {
        self.sum(self.values.len())
    }

    /// Largest `i` in `0..=len` whose prefix is `<= target`.
    /// An offset past the total height returns `len`.
    pub fn find_index(&self, target: f32) -> usize {
        let mut index = 0usize;
        let mut bit = highest_power_of_two(self.values.len());
        let mut acc = 0.0;
        while bit != 0 {
            let next = index + bit;
            let node = self.tree.get(next).copied().unwrap_or(0.0);
            if next <= self.values.len() && acc + node <= target {
                index = next;
                acc += node;
            }
            bit /= 2;
        }
        index
    }
}
