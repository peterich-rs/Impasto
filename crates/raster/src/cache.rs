//! Recency cache with both an entry limit and a caller-estimated byte limit.

use std::hash::Hash;

use lru::LruCache;

pub(crate) struct BudgetCache<K, V> {
    entries: LruCache<K, (V, usize)>,
    bytes: usize,
    max_bytes: usize,
    max_entries: usize,
}

impl<K: Eq + Hash, V> BudgetCache<K, V> {
    pub(crate) fn new(max_entries: usize, max_bytes: usize) -> Self {
        Self {
            entries: LruCache::unbounded(),
            bytes: 0,
            max_bytes,
            max_entries,
        }
    }

    pub(crate) fn get(&mut self, key: &K) -> Option<&V> {
        self.entries.get(key).map(|(value, _)| value)
    }

    pub(crate) fn clear(&mut self) -> usize {
        let len = self.entries.len();
        self.entries.clear();
        self.bytes = 0;
        len
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn insert(&mut self, key: K, value: V, bytes: usize) -> u32 {
        if bytes > self.max_bytes || self.max_entries == 0 {
            return 0;
        }
        if let Some((_, old_bytes)) = self.entries.pop(&key) {
            self.bytes -= old_bytes;
        }
        let mut evicted: u32 = 0;
        while self.bytes > self.max_bytes - bytes || self.entries.len() >= self.max_entries {
            let Some((_, (_, old_bytes))) = self.entries.pop_lru() else {
                break;
            };
            self.bytes -= old_bytes;
            evicted = evicted.saturating_add(1);
        }
        self.bytes += bytes;
        self.entries.put(key, (value, bytes));
        evicted
    }
}
