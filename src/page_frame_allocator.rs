#![allow(unused)]
use crate::device_tree;
use bitmap::Bitmap;

type PhysAddr = usize;
const PAGE_SIZE: usize = 4096;

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

    pub fn alloc(&mut self) -> Option<PhysAddr> {
        let idx = self.free.find_free();

        if let Some(idx) = idx {
            self.free.set(idx);
        }

        idx.map(|idx| self.start + PAGE_SIZE * idx as usize)
    }

    pub fn free(&mut self, addr: PhysAddr) {
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

/// Sv39 - Virtual Address
/// +---------+--------+--------+--------+-------------+
/// | 63 - 39 | 9-bits | 9-bits | 9-bits | 12-bits     |
/// +---------+--------+--------+--------+-------------+
/// |    0    | L2 idx | L1 idx | L0 idx | page offset |
/// +---------+--------+--------+--------+-------------+
struct VirtAddr(usize);

enum Level {
    L0,
    L1,
    L2,
}

impl VirtAddr {
    fn vpn(&self, level: Level) -> usize {
        let val = self.0;
        match level {
            Level::L0 => (val >> 12) & 0x1FF,
            Level::L1 => (val >> 21) & 0x1FF,
            Level::L2 => (val >> 30) & 0x1FF,
        }
    }

    fn offset(&self) -> usize {
        self.0 & 0xFFF
    }
}

/// Terminology
/// PPN: Physical Page Number

/// A page table entry always has the following format.
///
/// 63           54 53        28 27        19 18        10 9     8 7 6 5 4 3 2 1 0
/// +---------------+------------+------------+------------+-----+-+-+-+-+-+-+-+-+
/// |    Reserved   | PPN[2]     | PPN[1]     | PPN[0]     | RSW |D|A|G|U|X|W|R|V|
/// +---------------+------------+------------+------------+-----+-+-+-+-+-+-+-+-+
///
/// bit 0: valid (must be 1, if not MMU ignores this page)
/// bit 1: read
/// bit 2: write
/// bit 3: execute
/// bit 4: user - accessible from U-mode (userspace)
/// bit 5: global - mapping exists in all address spaces (useful for the kernel pages as discussed
///                 below)
/// bit 6: accessed - hardware sets this when page is read/written
/// bit 7: dirty - hardware sets this when page is written
///
/// Non-leaf nodes have all permissions 0 - leaf nodes must have at least one non-zero of (R/W/X)
///
/// The fact that pages are always 4k aligned, allows us to use the lower bits for something else.
/// In the case of the PTE, the lower 10 bits are used for flags. Because we know that the page is
/// 4k aligned, we can just << 12 the address and get the correct address without wasting space.
///
/// The PPN take up 44 bits here, plus the lower 12 that we know are 0, this gives us 2^56 bytes
/// address space.
struct PageTableEntry(usize);

mod pte_flags {
    const VALID: u8 = 1;
    const READ: u8 = 1 << 1;
    const WRITE: u8 = 1 << 2;
    const EXECUTE: u8 = 1 << 3;
    const USER: u8 = 1 << 4;
    const GLOBAL: u8 = 1 << 5;
    const ACCESSED: u8 = 1 << 6;
    const DIRTY: u8 = 1 << 7;
}

impl PageTableEntry {
    fn new_leaf(addr: PhysAddr, flags: usize) -> Self {
        let ppn = addr >> 12;
        PageTableEntry((ppn << 10) | flags | 1)
    }

    /// NOTE: Permissions are ignored by the hardware on non-leaf nodes, hence we don't care here.
    fn new_branch(addr: PhysAddr) -> Self {
        let ppn = addr >> 12;
        PageTableEntry((ppn << 10) | 1)
    }

    /// A PTE is valid if its `valid` bit is set. Otherwise the MMU will ignore this entry.
    fn is_valid(&self) -> bool {
        self.0 & 1 == 1
    }

    /// A node is a leaf-node if at least one of the permission bits are set (R/W/X)
    fn is_leaf(&self) -> bool {
        self.0 & 0b0111 != 0
    }

    /// The PPN (physical page number) is where this entry points to.
    ///
    /// For non-leaf nodes this always points to the physical address of the next page table.
    /// For leaf nodes, this points to the actual physical frame.
    fn ppn(&self) -> usize {
        (self.0 >> 10) << 12
    }
}

#[repr(C)]
struct PageTable {
    entries: [PageTableEntry; 512],
}

// Translation Lookaside Buffer (TLB) - hardware
// Is a cache for recent Virtual Memory Translation.
// IMPORTANT: Need to invalidate via `sfence.vma` when processes switch (i.e

/*

Each page table is 4k in size and holds 512 entries (each 8byte)

A physical Frame is the actual memory (size=4k)
A virtual Page is also 4k

We add 3 levels for the page table.

Each of the levels has 512 entries that point to other page tables.

The first page table L2 points to L1 tables which points to L0 tables.
The entries in the L0 table point to a phyiscal page.

Each process has it's own page table.

The last entry in the L2 page table is a pointer to the kernels page L1 table. This is done so that the trap handler access is still valid through
the kernels page tables.

This is also why we're loading the kernel into a high memory address.

Translating an address would mean
1. Get page table at index [L2]
2. In there get page table at index [L1]
3. In there get page table at index [L0]
4. Now we have the physical page - get the address at [offset]

 */
