use std::{
    collections::HashMap,
    hash::Hash,
    time::{Duration, Instant},
};

#[derive(Debug)]
struct Entry<V> {
    value: V,
    touched_at: Instant,
}

#[derive(Debug)]
pub struct BoundedTtlCache<K, V> {
    entries: HashMap<K, Entry<V>>,
    capacity: usize,
    ttl: Duration,
}

impl<K, V> BoundedTtlCache<K, V>
where
    K: Eq + Hash + Clone,
{
    #[must_use]
    pub fn new(capacity: usize, ttl: Duration) -> Self {
        Self {
            entries: HashMap::with_capacity(capacity),
            capacity,
            ttl,
        }
    }

    pub fn insert(&mut self, key: K, value: V) {
        self.prune_expired();
        if self.capacity == 0 {
            return;
        }
        if self.entries.len() >= self.capacity && !self.entries.contains_key(&key) {
            self.evict_oldest();
        }
        self.entries.insert(
            key,
            Entry {
                value,
                touched_at: Instant::now(),
            },
        );
    }

    pub fn get(&mut self, key: &K) -> Option<&V> {
        self.prune_expired();
        let entry = self.entries.get_mut(key)?;
        entry.touched_at = Instant::now();
        Some(&entry.value)
    }

    pub fn prune_expired(&mut self) {
        let now = Instant::now();
        let ttl = self.ttl;
        self.entries
            .retain(|_, entry| now.duration_since(entry.touched_at) <= ttl);
    }

    fn evict_oldest(&mut self) {
        let oldest = self
            .entries
            .iter()
            .min_by_key(|(_, entry)| entry.touched_at)
            .map(|(key, _)| key.clone());
        if let Some(key) = oldest {
            self.entries.remove(&key);
        }
    }
}
