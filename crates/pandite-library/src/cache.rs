//! A byte-budgeted cache keyed by path with rank-based eviction.
//!
//! Every entry remembers its folder index. The owner supplies a rank
//! function: a larger rank means "less useful now". Inserting an entry may
//! evict entries whose rank is strictly larger than the new entry's rank,
//! largest first. It never evicts an equal or better entry, so read-ahead
//! cannot thrash: when the budget is full of better entries, the new entry
//! is rejected instead.

use crate::fs::Stamp;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Index value for an entry whose path is not in the current folder list.
pub(crate) const NO_INDEX: usize = usize::MAX;

/// Eviction rank: distance from the point of interest, then `true` for the
/// side the user is less likely to visit next. Larger is evicted first.
pub(crate) type Rank = (usize, bool);

pub(crate) struct Entry<T> {
    pub value: T,
    pub bytes: usize,
    pub index: usize,
    pub stamp: Option<Stamp>,
}

pub(crate) struct Cache<T> {
    entries: HashMap<PathBuf, Entry<T>>,
    used: usize,
    budget: usize,
}

impl<T> Cache<T> {
    pub fn new(budget: usize) -> Self {
        Self {
            entries: HashMap::new(),
            used: 0,
            budget,
        }
    }

    pub fn get(&self, path: &Path) -> Option<&Entry<T>> {
        self.entries.get(path)
    }

    pub fn contains(&self, path: &Path) -> bool {
        self.entries.contains_key(path)
    }

    pub fn used(&self) -> usize {
        self.used
    }

    pub fn budget(&self) -> usize {
        self.budget
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn remove(&mut self, path: &Path) -> Option<Entry<T>> {
        let old = self.entries.remove(path)?;
        self.used -= old.bytes;
        Some(old)
    }

    /// Keep entries for which `keep` returns true.
    pub fn retain(&mut self, mut keep: impl FnMut(&Path, &Entry<T>) -> bool) {
        let used = &mut self.used;
        self.entries.retain(|path, entry| {
            let k = keep(path, entry);
            if !k {
                *used -= entry.bytes;
            }
            k
        });
    }

    /// Point every entry at its new folder index. Entries whose path is no
    /// longer listed are dropped when `drop_missing` is set, otherwise kept
    /// with [`NO_INDEX`] (first in line for eviction).
    pub fn remap(&mut self, index_of: impl Fn(&Path) -> Option<usize>, drop_missing: bool) {
        let used = &mut self.used;
        self.entries.retain(|path, entry| match index_of(path) {
            Some(i) => {
                entry.index = i;
                true
            }
            None if drop_missing => {
                *used -= entry.bytes;
                false
            }
            None => {
                entry.index = NO_INDEX;
                true
            }
        });
    }

    /// Bytes held by entries ranked strictly worse than `rank`.
    fn evictable(&self, rank: Rank, rank_of: &impl Fn(usize) -> Rank) -> usize {
        self.entries
            .values()
            .filter(|e| rank_of(e.index) > rank)
            .map(|e| e.bytes)
            .sum()
    }

    /// True when an entry of `bytes` at `rank` would be accepted.
    pub fn would_fit(
        &self,
        bytes: usize,
        rank: Rank,
        protected: bool,
        rank_of: impl Fn(usize) -> Rank,
    ) -> bool {
        protected
            || self.used + bytes <= self.budget
            || self.used - self.evictable(rank, &rank_of) + bytes <= self.budget
    }

    /// Insert `entry` at `rank`, evicting worse-ranked entries (worst
    /// first) until it fits. A `protected` entry is always inserted, even
    /// above budget. Returns false (and changes nothing) when the entry
    /// cannot fit without evicting an equal or better one.
    pub fn insert(
        &mut self,
        path: PathBuf,
        entry: Entry<T>,
        rank: Rank,
        protected: bool,
        rank_of: impl Fn(usize) -> Rank,
    ) -> bool {
        let previous = self.remove(&path);
        if !self.would_fit(entry.bytes, rank, protected, &rank_of) {
            if let Some(previous) = previous {
                self.used += previous.bytes;
                self.entries.insert(path, previous);
            }
            return false;
        }
        if self.used + entry.bytes > self.budget {
            let mut victims: Vec<(Rank, PathBuf)> = self
                .entries
                .iter()
                .filter_map(|(p, e)| {
                    let r = rank_of(e.index);
                    (r > rank).then(|| (r, p.clone()))
                })
                .collect();
            // Worst rank first; the path breaks ties so eviction is deterministic.
            victims.sort_by(|a, b| b.cmp(a));
            for (_, victim) in victims {
                if self.used + entry.bytes <= self.budget {
                    break;
                }
                self.remove(&victim);
            }
        }
        self.used += entry.bytes;
        self.entries.insert(path, entry);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(index: usize, bytes: usize) -> Entry<()> {
        Entry {
            value: (),
            bytes,
            index,
            stamp: None,
        }
    }

    fn dist(center: usize) -> impl Fn(usize) -> Rank {
        move |i| {
            if i >= center {
                (i - center, false)
            } else {
                (center - i, true)
            }
        }
    }

    #[test]
    fn insert_evicts_worst_first_and_never_better() {
        let mut c = Cache::new(30);
        for i in [5usize, 6, 4] {
            assert!(c.insert(
                PathBuf::from(i.to_string()),
                entry(i, 10),
                dist(5)(i),
                i == 5,
                dist(5)
            ));
        }
        assert_eq!(c.used(), 30);
        // 7 is worse than everything cached: rejected, nothing evicted.
        assert!(!c.insert("7".into(), entry(7, 10), dist(5)(7), false, dist(5)));
        assert_eq!(c.len(), 3);
        // Centered on 7 now: 4 is the worst (distance 3, before) and goes.
        assert!(c.insert("7".into(), entry(7, 10), dist(7)(7), true, dist(7)));
        assert!(!c.contains(Path::new("4")));
        assert_eq!(c.used(), 30);
        // A protected entry is kept even when it alone exceeds the budget.
        assert!(c.insert("9".into(), entry(9, 50), dist(9)(9), true, dist(9)));
        assert_eq!(c.len(), 1);
        assert_eq!(c.used(), 50);
        c.remap(|p| (p == Path::new("9")).then_some(0), true);
        assert_eq!(c.get(Path::new("9")).unwrap().index, 0);
        c.retain(|_, _| false);
        assert_eq!((c.len(), c.used()), (0, 0));
    }
}
