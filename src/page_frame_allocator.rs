extern crate alloc;

use alloc::vec::Vec;
use bitmap::Bitmap;
pub use virtual_memory::usable_memory_ranges;
use virtual_memory::{PAGE_SIZE, PhysAddr, PhysRange};

use crate::device_tree;
use crate::kernel_layout::KernelLayout;

static PAGE_FRAME_ALLOCATOR: spin::Mutex<Option<PageFrameAllocator>> = spin::Mutex::new(None);

struct FrameArena {
    range: PhysRange,
    used: Bitmap,
}

impl FrameArena {
    fn new(range: PhysRange) -> Self {
        debug_assert!(range.start().is_multiple_of(PAGE_SIZE));
        debug_assert!(range.end().is_multiple_of(PAGE_SIZE));

        Self {
            range,
            used: Bitmap::new(range.size() / PAGE_SIZE),
        }
    }

    fn alloc(&mut self) -> Option<PhysAddr> {
        let index = self.used.find_free()?;
        self.used.set(index);
        Some(self.range.start() + index * PAGE_SIZE)
    }

    fn contains(&self, addr: PhysAddr) -> bool {
        self.range.start() <= addr && addr < self.range.end()
    }

    fn free(&mut self, addr: PhysAddr) -> bool {
        if !addr.is_multiple_of(PAGE_SIZE) {
            log::error!("cannot free unaligned frame {addr:#x}; frame size is {PAGE_SIZE:#x}");
            return false;
        }

        if !self.contains(addr) {
            log::error!(
                "cannot free frame {addr:#x}; it is outside arena [{:#x}, {:#x})",
                self.range.start(),
                self.range.end()
            );
            return false;
        }

        let index = (addr - self.range.start()) / PAGE_SIZE;
        if !self.used.is_set(index) {
            log::error!(
                "cannot free frame {addr:#x}; frame {index} in arena [{:#x}, {:#x}) is not allocated",
                self.range.start(),
                self.range.end()
            );
            return false;
        }

        self.used.unset(index);
        true
    }
}

/// Manages all page-aligned portions of RAM which are not reserved by the
/// firmware, the live FDT, or the kernel image.
struct PageFrameAllocator {
    arenas: Vec<FrameArena>,
}

/// Returns a list of reserved memory regions, including the live FDT and kernel image.
fn fixup_reserved_memory_regions(layout: KernelLayout) -> Vec<PhysRange> {
    let mut reserved = device_tree::reserved_memory_regions().to_vec();
    reserved.push(device_tree::fdt_range());
    reserved.push(
        PhysRange::from_start_size(layout.kernel_start, layout.kernel_end - layout.kernel_start)
            .expect("kernel layout must describe a non-empty physical range"),
    );

    log::info!("Fixed up reserved memory regions: {reserved:?}");

    reserved
}

impl PageFrameAllocator {
    fn new(layout: KernelLayout) -> Self {
        let reserved = fixup_reserved_memory_regions(layout);
        let usable = usable_memory_ranges(device_tree::memory_regions(), &reserved);
        let num_pages: usize = usable.iter().map(|range| range.size() / PAGE_SIZE).sum();

        log::info!("usable physical memory: {usable:?}");
        log::info!("Found {num_pages} usable pages");

        Self {
            arenas: usable.into_iter().map(FrameArena::new).collect(),
        }
    }

    fn alloc(&mut self) -> Option<PhysAddr> {
        self.arenas.iter_mut().find_map(FrameArena::alloc)
    }

    fn free(&mut self, addr: PhysAddr) -> bool {
        let Some(arena) = self.arenas.iter_mut().find(|arena| arena.contains(addr)) else {
            log::error!("cannot free frame {addr:#x}; it is outside every managed arena");
            return false;
        };

        arena.free(addr)
    }

    fn ranges(&self) -> Vec<PhysRange> {
        self.arenas.iter().map(|arena| arena.range).collect()
    }
}

pub fn alloc_frame() -> Option<PhysAddr> {
    PAGE_FRAME_ALLOCATOR.lock().as_mut().unwrap().alloc()
}

pub fn free_frame(addr: PhysAddr) {
    PAGE_FRAME_ALLOCATOR.lock().as_mut().unwrap().free(addr);
}

/// Return a snapshot of the physical ranges managed by the allocator.
pub(crate) fn managed_ranges() -> Vec<PhysRange> {
    PAGE_FRAME_ALLOCATOR.lock().as_ref().unwrap().ranges()
}

pub fn init(layout: KernelLayout) {
    let mut allocator = PAGE_FRAME_ALLOCATOR.lock();

    if allocator.is_none() {
        allocator.replace(PageFrameAllocator::new(layout));
    } else {
        log::error!("Tried to initialize PageFrameAllocator twice");
    }
}
