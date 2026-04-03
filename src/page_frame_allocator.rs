use crate::device_tree;
use bitmap::Bitmap;
use virtual_memory::{PhysAddr, PAGE_SIZE};

static PAGE_FRAME_ALLOCATOR: spin::Mutex<Option<PageFrameAllocator>> = spin::Mutex::new(None);

/// This allocator maps the available RAM into 4kb pages and manages their lifecycle.
struct PageFrameAllocator {
    start: PhysAddr,
    free: Bitmap,
}

impl PageFrameAllocator {
    /// SAFETY: This requires the `_kernel_end` sybmol to be set and valid. It has to be
    /// page-aligned and point to an address right after the kernels binary and in unused RAM.
    unsafe fn new() -> Self {
        unsafe extern "C" {
            static _kernel_end: usize;
        }
        let size = device_tree::total_memory();

        let kernel_end = unsafe { _kernel_end };
        let ram_end = device_tree::ram_base() + size;

        let num_pages = ((ram_end - kernel_end) / PAGE_SIZE) & !31;

        log::info!("[PageFrameAllocator] Found {num_pages} pages");

        Self {
            start: kernel_end,
            free: Bitmap::new(num_pages as u32),
        }
    }

    fn alloc(&mut self) -> Option<PhysAddr> {
        let idx = self.free.find_free();

        if let Some(idx) = idx {
            self.free.set(idx);
        }

        idx.map(|idx| self.start + PAGE_SIZE * idx as usize)
    }

    fn free(&mut self, addr: PhysAddr) {
        let idx = (addr - self.start) / PAGE_SIZE;
        self.free.unset(idx as u32);
    }
}

pub fn alloc_frame() -> Option<PhysAddr> {
    PAGE_FRAME_ALLOCATOR.lock().as_mut().unwrap().alloc()
}

pub fn free_frame(addr: PhysAddr) {
    PAGE_FRAME_ALLOCATOR.lock().as_mut().unwrap().free(addr)
}

pub fn init() {
    unsafe {
        let mut alloc = PAGE_FRAME_ALLOCATOR.lock();

        if alloc.is_none() {
            alloc.replace(PageFrameAllocator::new());
        } else {
            log::error!("Tried to intitalized PageFrameAllocator twice");
        }
    }
}
