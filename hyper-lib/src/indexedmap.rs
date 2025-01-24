use std::collections::btree_map::Keys;
use std::collections::BTreeMap;
use std::hash::Hash;

#[derive(Clone)]
pub struct IndexedMap<K, V> {
    map: BTreeMap<K, V>,
    // Consider storing references. I didn't do it because that means carrying the lifetime
    // over all the objects including this map
    // If we are on with adding lifetimes, making this a Cycle iterator may also be an option
    order: Vec<K>,
    next_index: usize,
}

impl<K, V> IndexedMap<K, V>
where
    K: Hash + Eq + Copy + Ord,
{
    pub fn new() -> Self {
        Self {
            map: BTreeMap::new(),
            order: Vec::new(),
            next_index: 0,
        }
    }

    pub fn inner(&self) -> &BTreeMap<K, V> {
        &self.map
    }

    pub fn inner_mut(&mut self) -> &mut BTreeMap<K, V> {
        &mut self.map
    }

    pub fn insert(&mut self, key: K, value: V) -> Option<V> {
        self.order.push(key);
        self.map.insert(key, value)
    }

    pub fn get(&self, key: &K) -> Option<&V> {
        self.map.get(key)
    }

    pub fn get_mut(&mut self, key: &K) -> Option<&mut V> {
        self.map.get_mut(key)
    }

    pub fn contains_key(&self, key: &K) -> bool {
        self.map.contains_key(key)
    }

    pub fn keys(&self) -> Keys<'_, K, V> {
        self.map.keys()
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn get_next(&mut self) -> (K, &mut V) {
        if self.map.is_empty() {
            panic!("Called get_next on a empty IndexedMap")
        }

        // If we've reached the end of the map, circle back
        let next_index = self.next_index % self.map.len();
        let key = self
            .order
            .get(next_index)
            .expect("Tried to fetch an non-existing item from an IndexedMap");
        self.next_index = next_index + 1;

        (*key, self.map.get_mut(key).unwrap())
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_get_next() {
        let mut map = IndexedMap::new();

        // Check that we get the index of the last added element
        for i in 0..10 {
            map.insert(i, i * 2);
            assert_eq!(map.get_next().0, i);
        }

        // Check that, if we iterate over the added elements
        // the returned key cycles over once the last element is reached
        for i in 0..3 * map.len() {
            assert_eq!(map.get_next().0, i % 10);
        }
    }
}
