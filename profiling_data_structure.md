# Profiling State Management Data Structure Design

## Key Challenges

1. **Nested sections**: `start("outer") -> start("inner") -> end("inner") -> end("outer")`
2. **Interleaved sections**: `start("A") -> start("B") -> end("A") -> end("B")`
3. **Same ID multiple times**: `start("test") -> start("test") -> end("test") -> end("test")`
4. **Net CU calculation**: Need to subtract child consumption from parent consumption
5. **Deferred logging**: Calculate everything at instruction end

## Proposed Data Structure

```rust
pub struct ProfilingState {
    // Stack of currently active profiling sections (LIFO for same IDs)
    active_stack: Vec<ActiveEntry>,
    
    // All completed profiling sections (for net CU calculation)
    completed: Vec<CompletedEntry>,
    
    // Sequence counter to track temporal ordering
    next_sequence: usize,
}

struct ActiveEntry {
    id: String,
    start_cu: u64,
    start_sequence: usize, // When this section started (temporal order)
}

struct CompletedEntry {
    id: String,
    start_cu: u64,
    end_cu: u64,
    start_sequence: usize,
    end_sequence: usize,
    total_cu: u64,
    net_cu: u64, // Calculated during post-processing
}
```

## Algorithm

### Start Operation: `start(id)`
1. Push new `ActiveEntry` to stack with:
   - `id`: provided string
   - `start_cu`: current remaining CU
   - `start_sequence`: current sequence number
2. Increment `next_sequence`

### End Operation: `end(id)`
1. Search stack from top (LIFO) for matching `id`
2. Calculate `total_cu = start_cu - current_cu`
3. Move entry to `completed` with:
   - `end_cu`: current remaining CU
   - `end_sequence`: current sequence number
   - `total_cu`: calculated consumption
4. Increment `next_sequence`

### Post-Processing (End of Instruction)
Calculate net CU for each completed entry:
```rust
for entry in completed.iter_mut() {
    let mut children_cu = 0;
    for other in completed.iter() {
        // If 'other' started after 'entry' and ended before 'entry', it's a child
        if other.start_sequence > entry.start_sequence && 
           other.end_sequence < entry.end_sequence {
            children_cu += other.total_cu;
        }
    }
    entry.net_cu = entry.total_cu - children_cu;
}
```

## Example Walkthrough

```rust
// Initial state: CU = 1000
start("outer")     // active_stack: [outer@seq0@1000], seq = 1
start("inner")     // active_stack: [outer@seq0@1000, inner@seq1@900], seq = 2
end("inner")       // completed: [inner: total=100, seq1->seq2], active_stack: [outer@seq0@1000], seq = 3
end("outer")       // completed: [inner: total=100, outer: total=300], seq = 4

// Post-processing:
// inner: no children (no entries between seq1-seq2), net = 100
// outer: inner is child (seq1 > seq0, seq2 < seq3), net = 300 - 100 = 200
```

## Benefits

- **Handles nesting**: Parent-child relationships determined by sequence timing
- **Handles interleaving**: Same algorithm works for overlapping sections
- **LIFO matching**: Multiple same IDs handled with stack semantics
- **Accurate net CU**: Post-processing calculates true net consumption
- **Deferred output**: All calculations happen at instruction end