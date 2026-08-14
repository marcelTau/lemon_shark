use core::alloc::{GlobalAlloc, Layout};

pub use allocator::FreeListAllocator;

use crate::interrupts;
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
    inner: spin::Mutex<FreeListAllocator>,
}

impl LockedAllocator {
    pub const fn new() -> Self {
        Self {
            inner: spin::Mutex::new(FreeListAllocator {
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
        unsafe { (*self.inner.lock()).init(bounds.start, bounds.end) };
    }

    pub fn dump_state(&self) {
        let mut writer = UartWriter;
        (*self.inner.lock()).dump_state(&mut writer);
    }
}

unsafe impl GlobalAlloc for LockedAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        interrupts::without_interrupts(|| {
            let mut allocator = self.inner.lock();
            (*allocator).alloc(layout)
        })
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        interrupts::without_interrupts(|| {
            let mut allocator = self.inner.lock();
            (*allocator).dealloc(ptr, layout)
        })
    }
}
