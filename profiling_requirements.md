# Solana Runtime Profiling Syscalls Requirements

## Requirements Summary

### Core Functionality
1. **ID-based profiling**: Pass a string ID to both start and end syscalls to identify profiling sections
2. **Deferred logging**: Don't log immediately - store state and calculate on end
3. **Matching pairs**: Match start/end calls by ID, supporting interleaved/nested profiling
4. **Compute unit tracking**: Calculate both total and net CU consumption
5. **Heap tracking**: Optional heap usage tracking via third parameter
6. **End-of-instruction logging**: Only output profiling results at the end of instruction execution

### Compute Unit Calculations
- **Total CU**: Raw difference between start and end compute units for an ID
- **Net CU**: Total CU minus any CU consumed by other profiling sections that occurred within this start/end pair

### Example Scenario
```rust
start("outer")     // CU = 1000
start("inner")     // CU = 900  
end("inner")       // CU = 800, logs "inner consumed 100 CU (net 100 CU)"
end("outer")       // CU = 700, logs "outer consumed 300 CU (net 200 CU)"
                   // Net = 300 - 100 = 200 (subtracts inner consumption)
```

### Technical Challenges
1. **State management**: Need to store active profiling sessions in InvokeContext
2. **String handling**: Translate string IDs from program memory
3. **Nested tracking**: Handle overlapping profiling sections correctly
4. **CU accounting**: Track which CU consumption belongs to which profiling section

### Output Format
`"{id} consumed {total_cu} CU (net {net_cu} CU)"`

### Implementation Requirements
This requires:
- A profiling state data structure (stack or map) stored in InvokeContext
- Logic to handle nested/interleaved sections
- Accounting system to subtract inner consumption from outer sections
- String parameter handling in syscalls
- Hook into instruction completion to flush all profiling logs at once

### Heap Tracking (New Feature)

#### Heap Calculations
- **Total Heap**: Raw difference between start and end heap values for an ID
- **Net Heap**: Total heap minus any heap consumed by nested profiling sections
- **Remaining Heap**: Absolute heap value at the end of the profiling section

#### Heap Enable/Disable Logic
- **Enabled**: When `heap_value > 0` in both start and end calls
- **Disabled**: When `heap_value = 0` in either start or end calls
- **Heap values are passed as arguments** (not read from system state)

#### Heap Example Scenario
```rust
start("outer", heap=1000)    // CU=5000, Heap=1000
start("inner", heap=1200)    // CU=4500, Heap=1200  
end("inner", heap=1400)      // CU=4000, Heap=1400, logs CU + heap
end("outer", heap=1600)      // CU=3500, Heap=1600, logs CU + heap
                             // Inner: 200 heap consumed (net 200)
                             // Outer: 600 heap consumed (net 400) [600-200=400]
```

### Updated Output Format

#### CU Only (when heap_value = 0)
```
CU log:  1 operation_name consumed   1234 CU (net   1000 CU)
```

#### CU + Heap (when heap_value > 0)
```
CU log:  2 with_heap consumed   2345 CU (net   2000 CU)
HEAP :  5678 heap (net  5000 heap) remaining  3456
```

### Updated Syscall Interface
- `sol_log_compute_units_start(id_addr, id_len, heap_value)` - Start profiling with string ID and optional heap value
- `sol_log_compute_units_end(id_addr, id_len, heap_value)` - End profiling with string ID and optional heap value

**Parameters:**
- `id_addr`, `id_len`: String identifier for the profiling section
- `heap_value`: Heap usage value (0 = disabled, >0 = enabled with heap tracking)

Both syscalls should be free (no compute cost) for profiling purposes.

## Implementation Plan for Heap Feature

### Phase 1: Data Structure Updates
1. **Update ActiveEntry struct**:
   - Add `start_heap: Option<u64>` field
   
2. **Update CompletedEntry struct**:
   - Add `start_heap: Option<u64>` field
   - Add `end_heap: Option<u64>` field  
   - Add `total_heap: Option<u64>` field
   - Add `net_heap: Option<u64>` field

### Phase 2: Syscall Signature Updates
1. **Modify SyscallLogComputeUnitsStart**:
   - Change parameter from `_arg3: u64` to `heap_value: u64`
   - Pass heap_value to `ProfilingState::start()`

2. **Modify SyscallLogComputeUnitsEnd**:
   - Change parameter from `_arg3: u64` to `heap_value: u64`  
   - Pass heap_value to `ProfilingState::end()`

### Phase 3: ProfilingState Logic Updates
1. **Update start() method**:
   - Accept `heap_value: u64` parameter
   - Store `Some(heap_value)` if `heap_value > 0`, else `None`

2. **Update end() method**:
   - Accept `end_heap: u64` parameter
   - Calculate `total_heap` only if both start and end have heap values
   - Handle heap disable logic (start enabled, end disabled = no heap tracking)

3. **Update post_process() method**:
   - Calculate `net_heap` by subtracting children's `total_heap` from parent's `total_heap`
   - Only calculate net_heap if `total_heap.is_some()`

### Phase 4: Logging Output Updates
1. **Update flush_profiling_results()**:
   - Always log CU information first
   - Log heap information on separate line only if `total_heap.is_some()`
   - Format: `HEAP : {:>5} heap (net {:>5} heap) remaining {:>5}`
   - Use 5-digit right-alignment for all heap values

### Phase 5: Testing & Validation
1. **Test Cases**:
   - CU-only profiling (heap_value = 0)
   - CU + Heap profiling (heap_value > 0) 
   - Mixed profiling (some sections with heap, some without)
   - Nested heap profiling with net calculation
   - Interleaved heap profiling
   - Error cases (start with heap, end without heap)

2. **Output Validation**:
   - Verify proper alignment and formatting
   - Verify net heap calculations are correct
   - Verify remaining heap values are accurate

### Technical Considerations
- **Backward Compatibility**: Existing code passing 0 for heap_value will continue to work
- **Performance**: No additional syscalls or system reads - heap values passed as arguments
- **Flexibility**: Programs can choose per-section whether to enable heap tracking
- **Clarity**: Separate output lines make it easy to parse CU vs heap metrics