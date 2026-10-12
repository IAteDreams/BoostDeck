//! The README's test: `process` makes no heap allocations.
//!
//! The README names `assert_no_alloc`; this is the same check without the
//! dependency, so `dsp` stays dependency-free. A counting allocator wraps the
//! system one and counts only on a thread that has switched counting on, so
//! the test harness's own allocations on other threads do not register.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use dsp::Chain;

struct Counting;

thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

fn note_allocation() {
    // `try_with`: the allocator can run while thread-locals are being torn
    // down, and must not panic there.
    let _ = COUNTING.try_with(|counting| {
        if counting.get() {
            let _ = ALLOCATIONS.try_with(|n| n.set(n.get() + 1));
        }
    });
}

// SAFETY: every call is passed straight to `System`; counting touches only
// const-initialised thread-locals, which never allocate.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note_allocation();
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        note_allocation();
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        note_allocation();
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// Runs `f` and returns how many allocations it made on this thread.
fn allocations_in(f: impl FnOnce()) -> usize {
    ALLOCATIONS.with(|n| n.set(0));
    COUNTING.with(|c| c.set(true));
    f();
    COUNTING.with(|c| c.set(false));
    ALLOCATIONS.with(|n| n.get())
}

/// The counter itself has to work, or a pass below would mean nothing.
#[test]
fn the_counter_sees_an_allocation() {
    let made = allocations_in(|| {
        std::hint::black_box(Vec::<u8>::with_capacity(64));
    });
    assert!(made >= 1);
}

#[test]
fn process_makes_no_heap_allocations() {
    // Everything allocated up front, as the render thread does before
    // `start_stream()`.
    let mut chain = Chain::new(48_000, 2);
    let mut buffer = vec![0.0f32; 1_056 * 2];

    let made = allocations_in(|| {
        // Quiet, boosted into the soft clip, driven hard into the limiter,
        // and non-finite: every branch of the chain.
        for (amplitude, gain_db) in [(0.1, 0.0), (0.5, 12.0), (1.0, 24.0), (f32::NAN, 6.0)] {
            for _ in 0..50 {
                buffer.iter_mut().for_each(|s| *s = amplitude);
                std::hint::black_box(chain.process(&mut buffer, gain_db));
            }
        }
    });

    assert_eq!(made, 0, "process allocated {made} times");
}
