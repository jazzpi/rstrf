use iced::{Task, task};

/// A cache for an expensive async computation, keyed by its input parameters.
///
/// - in `view()`, query with `get_stored()`
/// - in `update()`, call `request(key, f)` to check staleness and start the computation if needed
#[derive(Debug, Clone)]
pub struct AsyncCache<K, V> {
    stored: Option<(K, V)>,
    computing: Option<(K, task::Handle)>,
}

impl<K, V> Default for AsyncCache<K, V> {
    fn default() -> Self {
        Self {
            stored: None,
            computing: None,
        }
    }
}

impl<K: PartialEq, V> AsyncCache<K, V> {
    /// Returns the stored key+value, for use in view code.
    pub fn get_stored(&self) -> Option<(&K, &V)> {
        self.stored.as_ref().map(|(k, v)| (k, v))
    }

    /// Checks if `key` is already covered by the in-flight computation or stored result. If not,
    /// aborts any in-flight computation and starts a new one via `f`. The stale stored value is
    /// kept during recomputation to avoid flicker. Returns `Task::none()` if the result is already
    /// fresh or in flight.
    pub fn request<M: 'static, F>(&mut self, key: K, f: F) -> Task<M>
    where
        K: Clone,
        F: FnOnce(K) -> Task<M>,
    {
        let fresh = self.computing.as_ref().map_or(false, |(k, _)| k == &key)
            || self.stored.as_ref().map_or(false, |(k, _)| k == &key);
        if fresh {
            return Task::none();
        }

        // Key changed — abort any in-flight computation and start a new one.
        if let Some((_, handle)) = self.computing.take() {
            handle.abort();
        }

        let (task, handle) = f(key.clone()).abortable();
        self.computing = Some((key, handle));
        task
    }

    /// Stores the result of a completed computation. Ignores results from aborted computations
    /// (i.e. the key must match what is currently in-flight).
    pub fn store(&mut self, key: K, value: V) {
        let is_current = self.computing.as_ref().map_or(false, |(k, _)| k == &key);
        if is_current {
            self.computing = None;
            self.stored = Some((key, value));
        }
    }

    /// Clears both the stored result and the in-flight flag. Use when inputs become entirely
    /// invalid so nothing stale is displayed.
    pub fn reset(&mut self) {
        if let Some((_, handle)) = self.computing.take() {
            handle.abort();
        }
        self.stored = None;
    }
}
