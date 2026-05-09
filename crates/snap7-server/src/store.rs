use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

/// Area codes recognised by the simulated PLC.
pub mod area {
    pub const PROCESS_INPUTS: u8 = 0x81;
    pub const PROCESS_OUTPUTS: u8 = 0x82;
    pub const MARKERS: u8 = 0x83;
    pub const DATA_BLOCK: u8 = 0x84;
    pub const INSTANCE_DB: u8 = 0x85;
    pub const LOCAL_DATA: u8 = 0x86;
    pub const TIMER: u8 = 0x1D;
    pub const COUNTER: u8 = 0x1C;
}

/// CPU run-state for the simulated PLC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpuState {
    Run,
    Stop,
}

impl Default for CpuState {
    fn default() -> Self {
        CpuState::Stop
    }
}

/// Information about a data-access event passed to callbacks.
#[derive(Debug, Clone)]
pub struct EventInfo {
    pub event: &'static str, // "read" | "write" | "cpu_stop" | "cpu_start"
    pub area: u8,
    pub db_number: u16,
    pub start: u32,
    pub length: u32,
}

// ---------------------------------------------------------------------------
// DataStore – multi-area, CPU state, callbacks
// ---------------------------------------------------------------------------

/// A unified data store that maps `(area, db_number, offset) -> u8`.
///
/// Supports:
/// - Arbitrary area codes (PI / PA / MK / DB / TI / CT / …)
/// - Per-area registration (`register_area` / `unregister_area`)
/// - CPU run-state (`cpu_state` / `set_cpu_state`)
/// - Read / write event callbacks
#[derive(Clone)]
pub struct DataStore {
    inner: Arc<Mutex<StoreInner>>,
}

impl Default for DataStore {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(StoreInner {
                data: HashMap::new(),
                cpu_state: CpuState::Stop,
                registered_areas: HashMap::new(),
                locked_areas: HashMap::new(),
                events: VecDeque::new(),
                event_mask: 0xFFFF_FFFF,
                client_count: 0,
                read_callbacks: Vec::new(),
                write_callbacks: Vec::new(),
                event_callbacks: Vec::new(),
            })),
        }
    }
}

/// Server and CPU status returned by [`DataStore::get_status`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerStatus {
    /// 0 = stopped, 1 = running
    pub server_running: bool,
    pub cpu_state: CpuState,
    pub clients_count: usize,
}

struct StoreInner {
    /// `(area_code, db_number, offset) -> byte`
    data: HashMap<(u8, u16, u32), u8>,
    cpu_state: CpuState,
    /// Set of registered area codes (area_code -> size hint).
    registered_areas: HashMap<u8, usize>,
    /// Locked areas — writes to locked areas are rejected.
    locked_areas: HashMap<u8, bool>,
    /// Event log queue (capped at 1024).
    events: VecDeque<EventInfo>,
    /// Bitmask filter: only events whose kind matches are enqueued.
    event_mask: u32,
    /// Connected client count (incremented/decremented by dispatch).
    pub(crate) client_count: usize,
    read_callbacks: Vec<Box<dyn Fn(&EventInfo) + Send>>,
    write_callbacks: Vec<Box<dyn Fn(&EventInfo) + Send>>,
    event_callbacks: Vec<Box<dyn Fn(&str) + Send>>,
}

impl DataStore {
    /// Create a new empty store.
    pub fn new() -> Self {
        Self::default()
    }

    // -- Area registration ---------------------------------------------------

    /// Register a memory area.  `size` is a hint; reads beyond written bytes
    /// return zeros.
    pub fn register_area(&self, area_code: u8, size: usize) {
        let mut inner = self.inner.lock().unwrap();
        inner.registered_areas.insert(area_code, size);
    }

    /// Unregister a previously registered area.
    pub fn unregister_area(&self, area_code: u8) {
        let mut inner = self.inner.lock().unwrap();
        inner.registered_areas.remove(&area_code);
        // Also purge stored bytes for this area.
        inner.data.retain(|k, _| k.0 != area_code);
    }

    /// Check whether an area is registered.
    pub fn is_area_registered(&self, area_code: u8) -> bool {
        self.inner.lock().unwrap().registered_areas.contains_key(&area_code)
    }

    /// Return the set of registered area codes.
    pub fn registered_areas(&self) -> Vec<u8> {
        self.inner.lock().unwrap().registered_areas.keys().copied().collect()
    }

    // -- Area lock / unlock --------------------------------------------------

    /// Lock an area: subsequent write_area calls to this area return without
    /// modifying data (silently skipped, matching C snap7 Srv_LockArea behaviour).
    pub fn lock_area(&self, area_code: u8) {
        self.inner.lock().unwrap().locked_areas.insert(area_code, true);
    }

    /// Unlock an area previously locked with [`lock_area`](Self::lock_area).
    pub fn unlock_area(&self, area_code: u8) {
        self.inner.lock().unwrap().locked_areas.remove(&area_code);
    }

    /// Return whether an area is currently locked.
    pub fn is_area_locked(&self, area_code: u8) -> bool {
        self.inner.lock().unwrap().locked_areas.contains_key(&area_code)
    }

    // -- Server status -------------------------------------------------------

    /// Return current server/CPU status and connected client count.
    pub fn get_status(&self) -> ServerStatus {
        let inner = self.inner.lock().unwrap();
        ServerStatus {
            server_running: true,
            cpu_state: inner.cpu_state,
            clients_count: inner.client_count,
        }
    }

    /// Increment the internal client counter (called by dispatch on connect).
    pub(crate) fn client_connected(&self) {
        self.inner.lock().unwrap().client_count += 1;
    }

    /// Decrement the internal client counter (called by dispatch on disconnect).
    pub(crate) fn client_disconnected(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.client_count = inner.client_count.saturating_sub(1);
    }

    // -- Event queue ---------------------------------------------------------

    /// Return the current event filter mask.
    pub fn get_mask(&self) -> u32 {
        self.inner.lock().unwrap().event_mask
    }

    /// Set the event filter mask. Only events whose kind-bit is set are enqueued.
    pub fn set_mask(&self, mask: u32) {
        self.inner.lock().unwrap().event_mask = mask;
    }

    /// Drain the event queue.
    pub fn clear_events(&self) {
        self.inner.lock().unwrap().events.clear();
    }

    /// Pop the oldest event from the queue. Returns `None` when empty.
    pub fn pick_event(&self) -> Option<EventInfo> {
        self.inner.lock().unwrap().events.pop_front()
    }

    /// Push an event into the queue (respects mask; queue capped at 1024).
    #[allow(dead_code)]
    pub(crate) fn enqueue_event(&self, info: EventInfo, kind_bit: u32) {
        let mut inner = self.inner.lock().unwrap();
        if inner.event_mask & kind_bit == 0 {
            return;
        }
        if inner.events.len() >= 1024 {
            inner.events.pop_front(); // drop oldest when full
        }
        inner.events.push_back(info);
    }

    // -- CPU state -----------------------------------------------------------

    /// Current simulated CPU state.
    pub fn cpu_state(&self) -> CpuState {
        self.inner.lock().unwrap().cpu_state
    }

    /// Set the simulated CPU state and fire `event_callbacks`.
    pub fn set_cpu_state(&self, state: CpuState) {
        let mut inner = self.inner.lock().unwrap();
        inner.cpu_state = state;
        drop(inner); // release lock before calling callbacks

        let event = match state {
            CpuState::Run => "cpu_start",
            CpuState::Stop => "cpu_stop",
        };
        self.fire_event(event);
    }

    // -- Data access (compatible with dispatch) ------------------------------

    /// Read a contiguous range of bytes.
    pub fn read_bytes(&self, db: u16, start: u32, count: u32) -> Vec<u8> {
        let inner = self.inner.lock().unwrap();
        let end = start.saturating_add(count);
        (start..end)
            .map(|offset| *inner.data.get(&(0x84, db, offset)).unwrap_or(&0))
            .collect()
    }

    /// Read from an arbitrary area.
    pub fn read_area(&self, area: u8, db: u16, start: u32, count: u32) -> Vec<u8> {
        let inner = self.inner.lock().unwrap();
        let end = start.saturating_add(count);
        let data: Vec<u8> = (start..end)
            .map(|offset| *inner.data.get(&(area, db, offset)).unwrap_or(&0))
            .collect();

        // Fire read callbacks after releasing the lock
        drop(inner);
        self.fire_read(&EventInfo {
            event: "read",
            area,
            db_number: db,
            start,
            length: count,
        });
        data
    }

    /// Write to an arbitrary area.
    ///
    /// Silently no-ops if the area is currently locked via [`lock_area`](Self::lock_area).
    pub fn write_area(&self, area: u8, db: u16, start: u32, data: &[u8]) {
        let mut inner = self.inner.lock().unwrap();
        if inner.locked_areas.contains_key(&area) {
            return;
        }
        for (i, &byte) in data.iter().enumerate() {
            if let Some(offset) = start.checked_add(i as u32) {
                inner.data.insert((area, db, offset), byte);
            }
        }
        drop(inner);

        self.fire_write(&EventInfo {
            event: "write",
            area,
            db_number: db,
            start,
            length: data.len() as u32,
        });
    }

    /// Write to DB area (convenience, retained for backward compat).
    pub fn write_bytes(&self, db: u16, start: u32, data: &[u8]) {
        self.write_area(area::DATA_BLOCK, db, start, data);
    }

    // -- Callbacks -----------------------------------------------------------

    /// Register a callback fired on every data read.
    pub fn on_read<F>(&self, cb: F)
    where
        F: Fn(&EventInfo) + Send + 'static,
    {
        self.inner.lock().unwrap().read_callbacks.push(Box::new(cb));
    }

    /// Register a callback fired on every data write.
    pub fn on_write<F>(&self, cb: F)
    where
        F: Fn(&EventInfo) + Send + 'static,
    {
        self.inner.lock().unwrap().write_callbacks.push(Box::new(cb));
    }

    /// Register a callback fired on CPU state changes and other server events.
    pub fn on_event<F>(&self, cb: F)
    where
        F: Fn(&str) + Send + 'static,
    {
        self.inner.lock().unwrap().event_callbacks.push(Box::new(cb));
    }

    // -- Internal helpers ----------------------------------------------------

    fn fire_read(&self, info: &EventInfo) {
        // Take the callback list so we can invoke callbacks without
        // holding the lock.
        let callbacks = {
            let mut inner = self.inner.lock().unwrap();
            std::mem::take(&mut inner.read_callbacks)
        };
        for cb in &callbacks {
            cb(info);
        }
        // Restore callbacks
        self.inner.lock().unwrap().read_callbacks = callbacks;
    }

    fn fire_write(&self, info: &EventInfo) {
        let callbacks = {
            let mut inner = self.inner.lock().unwrap();
            std::mem::take(&mut inner.write_callbacks)
        };
        for cb in &callbacks {
            cb(info);
        }
        self.inner.lock().unwrap().write_callbacks = callbacks;
    }

    fn fire_event(&self, event: &str) {
        let callbacks = {
            let mut inner = self.inner.lock().unwrap();
            std::mem::take(&mut inner.event_callbacks)
        };
        for cb in &callbacks {
            cb(event);
        }
        self.inner.lock().unwrap().event_callbacks = callbacks;
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

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
    fn write_to_different_dbs_isolated() {
        let store = DataStore::new();
        store.write_bytes(1, 0, &[0xAA]);
        store.write_bytes(2, 0, &[0xBB]);
        assert_eq!(store.read_bytes(1, 0, 1), vec![0xAA]);
        assert_eq!(store.read_bytes(2, 0, 1), vec![0xBB]);
    }

    #[test]
    fn read_area_uses_area_code() {
        let store = DataStore::new();
        store.write_area(area::MARKERS, 0, 10, &[0x99]);
        let pa = store.read_area(area::PROCESS_OUTPUTS, 0, 10, 1);
        assert_eq!(pa, vec![0x00]); // different area → no data
        let mk = store.read_area(area::MARKERS, 0, 10, 1);
        assert_eq!(mk, vec![0x99]);
    }

    #[test]
    fn register_area_roundtrip() {
        let store = DataStore::new();
        assert!(!store.is_area_registered(0x81));
        store.register_area(0x81, 1024);
        assert!(store.is_area_registered(0x81));
        store.unregister_area(0x81);
        assert!(!store.is_area_registered(0x81));
    }

    #[test]
    fn cpu_state_defaults_to_stop() {
        let store = DataStore::new();
        assert_eq!(store.cpu_state(), CpuState::Stop);
    }

    #[test]
    fn cpu_state_transitions() {
        let store = DataStore::new();
        store.set_cpu_state(CpuState::Run);
        assert_eq!(store.cpu_state(), CpuState::Run);
        store.set_cpu_state(CpuState::Stop);
        assert_eq!(store.cpu_state(), CpuState::Stop);
    }

    #[test]
    fn write_callback_invoked() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let store = DataStore::new();
        let fired = Arc::new(AtomicBool::new(false));
        let f = fired.clone();
        store.on_write(move |_| {
            f.store(true, Ordering::SeqCst);
        });
        store.write_bytes(1, 0, &[0x01]);
        assert!(fired.load(Ordering::SeqCst));
    }

    #[test]
    fn event_callback_invoked() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let store = DataStore::new();
        let fired = Arc::new(AtomicBool::new(false));
        let f = fired.clone();
        store.on_event(move |e| {
            if e == "cpu_start" {
                f.store(true, Ordering::SeqCst);
            }
        });
        store.set_cpu_state(CpuState::Run);
        assert!(fired.load(Ordering::SeqCst));
    }
}
