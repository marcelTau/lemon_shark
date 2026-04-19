use core::alloc::{GlobalAlloc, Layout};

pub use allocator::FreeListAllocator;

use crate::interrupts;
use crate::kernel_layout::KernelLayout;
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
    pub unsafe fn init(&self, layout: KernelLayout) {
        let bounds = HeapBounds::new(layout);
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
