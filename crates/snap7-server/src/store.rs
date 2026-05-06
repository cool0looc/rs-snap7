use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
pub struct DataStore {
    inner: Arc<Mutex<HashMap<(u16, u32), u8>>>,
}

impl DataStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn read_bytes(&self, db: u16, start: u32, count: u32) -> Vec<u8> {
        let inner = self.inner.lock().unwrap();
        let end = start.saturating_add(count);
        (start..end)
            .map(|offset| *inner.get(&(db, offset)).unwrap_or(&0))
            .collect()
    }

    pub fn write_bytes(&self, db: u16, start: u32, data: &[u8]) {
        let mut inner = self.inner.lock().unwrap();
        for (i, &byte) in data.iter().enumerate() {
            if let Some(offset) = start.checked_add(i as u32) {
                inner.insert((db, offset), byte);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_unset_returns_zeros() {
        let store = DataStore::new();
        let data = store.read_bytes(1, 0, 4);
        assert_eq!(data, vec![0, 0, 0, 0]);
    }

    #[test]
    fn write_then_read_roundtrip() {
        let store = DataStore::new();
        store.write_bytes(1, 0, &[0xDE, 0xAD, 0xBE, 0xEF]);
        let data = store.read_bytes(1, 0, 4);
        assert_eq!(data, vec![0xDE, 0xAD, 0xBE, 0xEF]);
    }

    #[test]
    fn partial_read_within_written_range() {
        let store = DataStore::new();
        store.write_bytes(1, 0, &[0x01, 0x02, 0x03, 0x04]);
        let data = store.read_bytes(1, 1, 2);
        assert_eq!(data, vec![0x02, 0x03]);
    }

    #[test]
    fn write_to_different_dbs_isolated() {
        let store = DataStore::new();
        store.write_bytes(1, 0, &[0xAA]);
        store.write_bytes(2, 0, &[0xBB]);
        assert_eq!(store.read_bytes(1, 0, 1), vec![0xAA]);
        assert_eq!(store.read_bytes(2, 0, 1), vec![0xBB]);
    }
}
