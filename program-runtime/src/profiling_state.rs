#[derive(Debug, Clone)]
pub struct ActiveEntry {
    pub id: String,
    pub start_cu: u64,
    pub start_sequence: usize,
    pub start_heap: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct HeapMetrics {
    pub start_heap: u64,
    pub end_heap: u64,
    pub total_heap: u64,
    pub net_heap: u64,
    pub remaining_heap: u64,
}

#[derive(Debug, Clone)]
pub struct CompletedEntry {
    pub id: String,
    pub start_cu: u64,
    pub end_cu: u64,
    pub start_sequence: usize,
    pub end_sequence: usize,
    pub total_cu: u64,
    pub net_cu: u64,
    pub remaining_cu: u64,
    pub heap: Option<HeapMetrics>,
}

#[derive(Debug, Default)]
pub struct ProfilingState {
    // Stack of currently active profiling sections (LIFO for same IDs)
    active_stack: Vec<ActiveEntry>,

    // All completed profiling sections (for net CU calculation)
    completed: Vec<CompletedEntry>,

    // Sequence counter to track temporal ordering
    next_sequence: usize,
}

impl ProfilingState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Start profiling for the given ID
    pub fn start(&mut self, id: String, current_cu: u64, heap_value: u64, with_heap: bool) {
        let entry = ActiveEntry {
            id,
            start_cu: current_cu,
            start_sequence: self.next_sequence,
            start_heap: if with_heap { Some(heap_value) } else { None },
        };

        self.active_stack.push(entry);
        self.next_sequence += 1;
    }

    /// End profiling for the given ID (LIFO - finds most recent matching ID)
    pub fn end(&mut self, id: &str, current_cu: u64, heap_value: u64, with_heap: bool) -> Result<(), String> {
        // Find the most recent (top-most) matching ID in the stack
        let pos = self
            .active_stack
            .iter()
            .rposition(|entry| entry.id == id)
            .ok_or_else(|| format!("No active profiling section found for ID: {}", id))?;

        // Remove the entry from the stack
        let active_entry = self.active_stack.remove(pos);

        // Calculate total CU consumed
        let total_cu = active_entry.start_cu.saturating_sub(current_cu);

        // Calculate heap metrics if enabled in both start and end calls
        let heap = if let Some(start_heap_value) = active_entry.start_heap {
            if with_heap {
                // Heap tracking enabled
                // heap_value = cumulative heap used so far
                // total_heap = heap consumed in this section = end_used - start_used
                let total_heap = heap_value.saturating_sub(start_heap_value);
                // remaining_heap = heap available at start = 32_000 - start_heap_value
                let remaining_heap = 32_000u64.saturating_sub(start_heap_value);
                Some(HeapMetrics {
                    start_heap: start_heap_value,
                    end_heap: heap_value,
                    total_heap,
                    net_heap: 0, // Will be calculated in post_process
                    remaining_heap,
                })
            } else {
                // Heap disabled at end (start enabled, end disabled)
                None
            }
        } else {
            // Heap was disabled at start
            None
        };

        // Create completed entry
        let completed_entry = CompletedEntry {
            id: active_entry.id,
            start_cu: active_entry.start_cu,
            end_cu: current_cu,
            start_sequence: active_entry.start_sequence,
            end_sequence: self.next_sequence,
            total_cu,
            net_cu: 0, // Will be calculated in post_process
            remaining_cu: active_entry.start_cu, // CU available at start
            heap,
        };

        self.completed.push(completed_entry);
        self.next_sequence += 1;

        Ok(())
    }

    /// Calculate net CU consumption and net heap consumption for all completed entries
    pub fn post_process(&mut self) {
        for i in 0..self.completed.len() {
            let mut children_cu = 0;
            let mut children_heap = 0;
            let entry = &self.completed[i];

            // Find all child entries (started after and ended before this entry)
            for other in &self.completed {
                if other.start_sequence > entry.start_sequence
                    && other.end_sequence < entry.end_sequence
                {
                    children_cu += other.total_cu;
                    if let Some(ref other_heap) = other.heap {
                        children_heap += other_heap.total_heap;
                    }
                }
            }

            // Update net CU
            self.completed[i].net_cu = entry.total_cu.saturating_sub(children_cu);

            // Update net heap if heap tracking is enabled for this entry
            if let Some(ref mut heap) = self.completed[i].heap {
                heap.net_heap = heap.total_heap.saturating_sub(children_heap);
            }
        }
    }

    /// Get all completed entries (for logging at end of instruction)
    pub fn get_completed(&self) -> &[CompletedEntry] {
        &self.completed
    }

    /// Get active entries (for debugging)
    pub fn get_active(&self) -> &[ActiveEntry] {
        &self.active_stack
    }

    /// Clear all state (called after logging at end of instruction)
    pub fn clear(&mut self) {
        self.active_stack.clear();
        self.completed.clear();
        self.next_sequence = 0;
    }

    /// Check if there are any active profiling sections
    pub fn has_active(&self) -> bool {
        !self.active_stack.is_empty()
    }

    /// Get the number of completed entries
    pub fn completed_count(&self) -> usize {
        self.completed.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_start_end() {
        let mut state = ProfilingState::new();

        // Start profiling (with_heap = false disables heap tracking)
        state.start("test".to_string(), 1000, 0, false);
        assert_eq!(state.active_stack.len(), 1);
        assert_eq!(state.completed.len(), 0);

        // End profiling
        state.end("test", 800, 0, false).unwrap();
        assert_eq!(state.active_stack.len(), 0);
        assert_eq!(state.completed.len(), 1);

        state.post_process();
        let entry = &state.completed[0];
        assert_eq!(entry.id, "test");
        assert_eq!(entry.total_cu, 200);
        assert_eq!(entry.net_cu, 200); // No children
        assert!(entry.heap.is_none()); // Heap tracking disabled
    }

    #[test]
    fn test_nested_profiling() {
        let mut state = ProfilingState::new();

        // Nested scenario: outer -> inner -> end inner -> end outer
        state.start("outer".to_string(), 1000, 0, false);
        state.start("inner".to_string(), 900, 0, false);
        state.end("inner", 800, 0, false).unwrap();
        state.end("outer", 700, 0, false).unwrap();

        state.post_process();

        assert_eq!(state.completed.len(), 2);

        // Find entries by ID
        let inner = state.completed.iter().find(|e| e.id == "inner").unwrap();
        let outer = state.completed.iter().find(|e| e.id == "outer").unwrap();

        // Check totals
        assert_eq!(inner.total_cu, 100); // 900 - 800
        assert_eq!(outer.total_cu, 300); // 1000 - 700

        // Check net CU (outer should subtract inner's consumption)
        assert_eq!(inner.net_cu, 100); // No children
        assert_eq!(outer.net_cu, 200); // 300 - 100
    }

    #[test]
    fn test_interleaved_profiling() {
        let mut state = ProfilingState::new();

        // Interleaved: A -> B -> end A -> end B
        state.start("A".to_string(), 1000, 0, false);
        state.start("B".to_string(), 900, 0, false);
        state.end("A", 800, 0, false).unwrap(); // A ends before B
        state.end("B", 700, 0, false).unwrap();

        state.post_process();

        assert_eq!(state.completed.len(), 2);

        let a = state.completed.iter().find(|e| e.id == "A").unwrap();
        let b = state.completed.iter().find(|e| e.id == "B").unwrap();

        // Both should have no children (they overlap but neither contains the other)
        assert_eq!(a.total_cu, 200); // 1000 - 800
        assert_eq!(a.net_cu, 200);
        assert_eq!(b.total_cu, 200); // 900 - 700
        assert_eq!(b.net_cu, 200);
    }

    #[test]
    fn test_same_id_multiple_times() {
        let mut state = ProfilingState::new();

        // Multiple same IDs (LIFO behavior)
        state.start("test".to_string(), 1000, 0, false);
        state.start("test".to_string(), 900, 0, false);
        state.end("test", 800, 0, false).unwrap(); // Should end the inner one
        state.end("test", 700, 0, false).unwrap(); // Should end the outer one

        state.post_process();

        assert_eq!(state.completed.len(), 2);

        // Sort by start sequence to identify outer vs inner
        let mut entries: Vec<_> = state.completed.iter().collect();
        entries.sort_by_key(|e| e.start_sequence);

        let outer = entries[0]; // Started first
        let inner = entries[1]; // Started second

        assert_eq!(outer.total_cu, 300); // 1000 - 700
        assert_eq!(inner.total_cu, 100); // 900 - 800

        // Inner has no children, outer contains inner
        assert_eq!(inner.net_cu, 100);
        assert_eq!(outer.net_cu, 200); // 300 - 100
    }

    #[test]
    fn test_complex_nested_scenario() {
        let mut state = ProfilingState::new();

        // Complex: outer -> middle -> inner -> end inner -> end middle -> end outer
        state.start("outer".to_string(), 1000, 0, false);
        state.start("middle".to_string(), 900, 0, false);
        state.start("inner".to_string(), 800, 0, false);
        state.end("inner", 700, 0, false).unwrap();
        state.end("middle", 600, 0, false).unwrap();
        state.end("outer", 500, 0, false).unwrap();

        state.post_process();

        assert_eq!(state.completed.len(), 3);

        let inner = state.completed.iter().find(|e| e.id == "inner").unwrap();
        let middle = state.completed.iter().find(|e| e.id == "middle").unwrap();
        let outer = state.completed.iter().find(|e| e.id == "outer").unwrap();

        // Check totals
        assert_eq!(inner.total_cu, 100); // 800 - 700
        assert_eq!(middle.total_cu, 300); // 900 - 600
        assert_eq!(outer.total_cu, 500); // 1000 - 500

        // Check net CU
        assert_eq!(inner.net_cu, 100); // No children
        assert_eq!(middle.net_cu, 200); // 300 - 100 (inner)
        assert_eq!(outer.net_cu, 100); // 500 - 300 (middle) - 100 (inner) = 100
    }

    #[test]
    fn test_end_nonexistent_id() {
        let mut state = ProfilingState::new();

        state.start("test".to_string(), 1000, 0, false);

        // Try to end a different ID
        let result = state.end("nonexistent", 800, 0, false);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("No active profiling section found"));

        // Original should still be active
        assert_eq!(state.active_stack.len(), 1);
        assert_eq!(state.completed.len(), 0);
    }

    #[test]
    fn test_clear_state() {
        let mut state = ProfilingState::new();

        state.start("test".to_string(), 1000, 0, false);
        state.end("test", 800, 0, false).unwrap();
        state.post_process();

        assert_eq!(state.completed.len(), 1);

        state.clear();

        assert_eq!(state.active_stack.len(), 0);
        assert_eq!(state.completed.len(), 0);
        assert_eq!(state.next_sequence, 0);
    }

    #[test]
    fn test_has_active_and_completed_count() {
        let mut state = ProfilingState::new();

        assert!(!state.has_active());
        assert_eq!(state.completed_count(), 0);

        state.start("test".to_string(), 1000, 0, false);
        assert!(state.has_active());
        assert_eq!(state.completed_count(), 0);

        state.end("test", 800, 0, false).unwrap();
        assert!(!state.has_active());
        assert_eq!(state.completed_count(), 1);
    }

    #[test]
    fn test_heap_tracking_enabled() {
        let mut state = ProfilingState::new();

        // Nested scenario with heap tracking enabled
        state.start("outer".to_string(), 5000, 1000, true); // CU=5000, Heap=1000
        state.start("inner".to_string(), 4500, 1200, true); // CU=4500, Heap=1200
        state.end("inner", 4000, 1400, true).unwrap(); // CU=4000, Heap=1400
        state.end("outer", 3500, 1600, true).unwrap(); // CU=3500, Heap=1600

        state.post_process();

        assert_eq!(state.completed.len(), 2);

        let inner = state.completed.iter().find(|e| e.id == "inner").unwrap();
        let outer = state.completed.iter().find(|e| e.id == "outer").unwrap();

        // Check CU calculations
        assert_eq!(inner.total_cu, 500); // 4500 - 4000
        assert_eq!(outer.total_cu, 1500); // 5000 - 3500
        assert_eq!(inner.net_cu, 500); // No children
        assert_eq!(outer.net_cu, 1000); // 1500 - 500

        // Check heap calculations
        let inner_heap = inner.heap.as_ref().unwrap();
        let outer_heap = outer.heap.as_ref().unwrap();

        assert_eq!(inner_heap.start_heap, 1200);
        assert_eq!(inner_heap.end_heap, 1400);
        assert_eq!(inner_heap.total_heap, 200); // 1400 - 1200
        assert_eq!(inner_heap.net_heap, 200); // No children
        assert_eq!(inner_heap.remaining_heap, 1400);

        assert_eq!(outer_heap.start_heap, 1000);
        assert_eq!(outer_heap.end_heap, 1600);
        assert_eq!(outer_heap.total_heap, 600); // 1600 - 1000
        assert_eq!(outer_heap.net_heap, 400); // 600 - 200
        assert_eq!(outer_heap.remaining_heap, 1600);
    }

    #[test]
    fn test_heap_tracking_disabled() {
        let mut state = ProfilingState::new();

        // Test various disable scenarios
        
        // Scenario 1: Both start and end with with_heap = false
        state.start("both_false".to_string(), 1000, 0, false);
        state.end("both_false", 800, 0, false).unwrap();

        // Scenario 2: Start with with_heap = true, end with with_heap = false
        state.start("start_enabled".to_string(), 1000, 500, true);
        state.end("start_enabled", 800, 600, false).unwrap();

        // Scenario 3: Start with with_heap = false, end with with_heap = true
        state.start("end_enabled".to_string(), 1000, 0, false);
        state.end("end_enabled", 800, 500, true).unwrap();

        state.post_process();

        assert_eq!(state.completed.len(), 3);

        // All should have heap tracking disabled (heap = None)
        for entry in &state.completed {
            assert!(entry.heap.is_none(), "Entry {} should have heap disabled", entry.id);
        }
    }
}
