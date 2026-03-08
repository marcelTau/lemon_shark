use core::alloc::{GlobalAlloc, Layout};
use core::cell::UnsafeCell;

pub use allocator::FreeListAllocator;

use crate::println::UartWriter;

pub struct HeapBounds {
    pub start: usize,
    pub end: usize,
}

impl HeapBounds {
    /// Read heap extents from linker-inserted symbols.
    ///
    /// SAFETY: `_heap_start` and `_heap_end` must be defined by `linker.ld`.
    pub unsafe fn new() -> Self {
        unsafe extern "C" {
            static _heap_start: u8;
            static _heap_end: u8;
        }

        let start = unsafe { &_heap_start as *const u8 as usize };
        let end = unsafe { &_heap_end as *const u8 as usize };

        Self { start, end }
    }

    pub fn size(&self) -> usize {
        self.end - self.start
    }
}

#[derive(Default)]
pub struct LockedAllocator {
    inner: UnsafeCell<FreeListAllocator>,
}

unsafe impl Sync for LockedAllocator {}

impl LockedAllocator {
    pub const fn new() -> Self {
        Self {
            inner: UnsafeCell::new(FreeListAllocator {
                head: None,
                #[cfg(feature = "stats")]
                stats: AllocationStats::new(),
            }),
        }
    }

    /// Initialize using linker-symbol heap bounds.
    ///
    /// SAFETY: Must be called exactly once before any allocation.
    pub unsafe fn init(&self) {
        let bounds = unsafe { HeapBounds::new() };
        unsafe { (*self.inner.get()).init(bounds.start, bounds.end) };
    }

    pub unsafe fn dump_state(&self) {
        let mut writer = UartWriter;
        unsafe { (*self.inner.get()).dump_state(&mut writer) };
    }
}

unsafe impl GlobalAlloc for LockedAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        unsafe { (*self.inner.get()).alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { (*self.inner.get()).dealloc(ptr, layout) }
    }
}
