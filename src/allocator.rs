use core::alloc::{GlobalAlloc, Layout};
use core::cell::UnsafeCell;

pub use allocator::FreeListAllocator;

use crate::KernelLayout;
use crate::println::UartWriter;

pub struct HeapBounds {
    pub start: usize,
    pub end: usize,
}

impl HeapBounds {
    pub fn new(layout: KernelLayout) -> Self {
        Self {
            start: layout.heap_start,
            end: layout.heap_end,
        }
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
    pub unsafe fn init(&self, layout: KernelLayout) {
        let bounds = HeapBounds::new(layout);
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
