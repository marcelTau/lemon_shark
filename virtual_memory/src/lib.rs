#![cfg_attr(not(test), no_std)]

pub type PhysAddr = usize;
pub const PAGE_SIZE: usize = 4096;

/// Sv39 - Virtual Address
/// +---------+--------+--------+--------+-------------+
/// | 63 - 39 | 9-bits | 9-bits | 9-bits | 12-bits     |
/// +---------+--------+--------+--------+-------------+
/// |    0    | L2 idx | L1 idx | L0 idx | page offset |
/// +---------+--------+--------+--------+-------------+
pub struct VirtAddr(pub usize);

pub enum Level {
    L0,
    L1,
    L2,
}

impl VirtAddr {
    pub fn vpn(&self, level: Level) -> usize {
        let val = self.0;
        match level {
            Level::L0 => (val >> 12) & 0x1FF,
            Level::L1 => (val >> 21) & 0x1FF,
            Level::L2 => (val >> 30) & 0x1FF,
        }
    }

    pub fn offset(&self) -> usize {
        self.0 & 0xFFF
    }

    pub fn from_parts(l2: usize, l1: usize, l0: usize, offset: usize) -> Self {
        VirtAddr((l2 << 30) | (l1 << 21) | (l0 << 12) | offset)
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
pub struct PageTableEntry(usize);

pub mod pte_flags {
    pub const VALID: usize = 1;
    pub const READ: usize = 1 << 1;
    pub const WRITE: usize = 1 << 2;
    pub const EXECUTE: usize = 1 << 3;
    pub const USER: usize = 1 << 4;
    pub const GLOBAL: usize = 1 << 5;
    pub const ACCESSED: usize = 1 << 6;
    pub const DIRTY: usize = 1 << 7;
}

impl PageTableEntry {
    pub fn new_leaf(addr: PhysAddr, flags: usize) -> Self {
        let ppn = addr >> 12;
        PageTableEntry((ppn << 10) | flags | pte_flags::VALID)
    }

    /// NOTE: Permissions are ignored by the hardware on non-leaf nodes, hence we don't care here.
    pub fn new_branch(addr: PhysAddr) -> Self {
        let ppn = addr >> 12;
        PageTableEntry((ppn << 10) | pte_flags::VALID)
    }

    /// A PTE is valid if its `valid` bit is set. Otherwise the MMU will ignore this entry.
    pub fn is_valid(&self) -> bool {
        self.0 & pte_flags::VALID == pte_flags::VALID
    }

    /// A node is a leaf-node if at least one of the permission bits are set (R/W/X)
    pub fn is_leaf(&self) -> bool {
        self.0 & (pte_flags::READ | pte_flags::WRITE | pte_flags::EXECUTE) != 0
    }

    /// The PPN (physical page number) is where this entry points to.
    ///
    /// For non-leaf nodes this always points to the physical address of the next page table.
    /// For leaf nodes, this points to the actual physical frame.
    pub fn ppn(&self) -> usize {
        (self.0 >> 10) << 12
    }
}

#[repr(C)]
#[repr(align(4096))]
pub struct PageTable {
    entries: [PageTableEntry; 512],
}

impl PageTable {
    pub const fn new() -> PageTable {
        Self {
            entries: [const { PageTableEntry(0) }; 512],
        }
    }

    pub fn get_mut(&mut self, idx: usize) -> &mut PageTableEntry {
        &mut self.entries[idx]
    }

    /// Allocate and zero a new frame using the provided allocator, returning it as a PageTable.
    ///
    /// SAFETY: `alloc` must return a valid, writable, 4KB-aligned physical address.
    unsafe fn new_table<F>(alloc: &F) -> PhysAddr
    where
        F: Fn() -> PhysAddr,
    {
        let frame = alloc();
        unsafe {
            (frame as *mut PageTable).write_bytes(0, 1);
        }
        frame
    }

    /// Map `virt` to `phys` with the given flags.
    ///
    /// Intermediate page tables are allocated on demand using `alloc`.
    ///
    /// SAFETY: `alloc` must return valid 4KB-aligned physical frames. All physical addresses must
    /// be accessible (identity-mapped or otherwise reachable) at the time of the call.
    pub unsafe fn map<F>(&mut self, virt: VirtAddr, phys: PhysAddr, flags: usize, alloc: F)
    where
        F: Fn() -> PhysAddr,
    {
        let l2_entry = self.get_mut(virt.vpn(Level::L2));

        if !l2_entry.is_valid() {
            let frame = unsafe { Self::new_table(&alloc) };
            *l2_entry = PageTableEntry::new_branch(frame);
        }

        let l1_table = unsafe { &mut *(l2_entry.ppn() as *mut PageTable) };
        let l1_entry = l1_table.get_mut(virt.vpn(Level::L1));

        if !l1_entry.is_valid() {
            let frame = unsafe { Self::new_table(&alloc) };
            *l1_entry = PageTableEntry::new_branch(frame);
        }

        let l0_table = unsafe { &mut *(l1_entry.ppn() as *mut PageTable) };
        let l0_entry = l0_table.get_mut(virt.vpn(Level::L0));

        // At this point, the entry should not be valid as we're creating the mapping.
        assert!(!l0_entry.is_valid());

        *l0_entry = PageTableEntry::new_leaf(phys, flags);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::alloc::{Layout, alloc_zeroed};

    /// Allocate a single zeroed 4KB-aligned frame from the host allocator.
    fn alloc_test_frame() -> PhysAddr {
        let layout = Layout::from_size_align(PAGE_SIZE, PAGE_SIZE).unwrap();
        unsafe { alloc_zeroed(layout) as PhysAddr }
    }

    #[test]
    fn virtaddr_vpn_all_zero() {
        let va = VirtAddr(0x0);
        assert_eq!(va.vpn(Level::L2), 0);
        assert_eq!(va.vpn(Level::L1), 0);
        assert_eq!(va.vpn(Level::L0), 0);
        assert_eq!(va.offset(), 0);
    }

    #[test]
    fn virtaddr_from_parts_roundtrips() {
        let va = VirtAddr::from_parts(1, 2, 3, 0x100);
        assert_eq!(va.vpn(Level::L2), 1);
        assert_eq!(va.vpn(Level::L1), 2);
        assert_eq!(va.vpn(Level::L0), 3);
        assert_eq!(va.offset(), 0x100);
    }

    #[test]
    fn virtaddr_vpn_max_index() {
        let va = VirtAddr::from_parts(0x1FF, 0x1FF, 0x1FF, 0xFFF);
        assert_eq!(va.vpn(Level::L2), 0x1FF);
        assert_eq!(va.vpn(Level::L1), 0x1FF);
        assert_eq!(va.vpn(Level::L0), 0x1FF);
        assert_eq!(va.offset(), 0xFFF);
    }

    #[test]
    fn pte_zero_is_invalid() {
        let entry = PageTableEntry(0);
        assert!(!entry.is_valid());
    }

    #[test]
    fn pte_new_branch_is_valid_non_leaf() {
        let frame = alloc_test_frame();
        let entry = PageTableEntry::new_branch(frame);
        assert!(entry.is_valid());
        assert!(!entry.is_leaf());
        assert_eq!(entry.ppn(), frame);
    }

    #[test]
    fn pte_new_leaf_is_valid_leaf() {
        let frame = alloc_test_frame();
        let entry = PageTableEntry::new_leaf(frame, pte_flags::READ | pte_flags::WRITE);
        assert!(entry.is_valid());
        assert!(entry.is_leaf());
        assert_eq!(entry.ppn(), frame);
    }

    #[test]
    fn pte_leaf_read_only() {
        let frame = alloc_test_frame();
        let entry = PageTableEntry::new_leaf(frame, pte_flags::READ);
        assert!(entry.is_leaf());
        assert_eq!(entry.ppn(), frame);
    }

    #[test]
    fn pte_ppn_roundtrips() {
        let frame = alloc_test_frame();
        let entry = PageTableEntry::new_leaf(frame, pte_flags::READ);
        assert_eq!(entry.ppn(), frame);
    }

    #[test]
    fn map_creates_correct_three_level_walk() {
        let phys = alloc_test_frame();
        let root_frame = alloc_test_frame();
        let root = unsafe { &mut *(root_frame as *mut PageTable) };

        unsafe {
            root.map(
                VirtAddr::from_parts(1, 2, 3, 0x100),
                phys,
                pte_flags::READ | pte_flags::WRITE,
                alloc_test_frame,
            );
        }

        // L2 entry at index 1 should be a valid branch
        let l2_entry = root.get_mut(1);
        assert!(l2_entry.is_valid());
        assert!(!l2_entry.is_leaf());

        // L1 entry at index 2 should be a valid branch
        let l1_table = unsafe { &mut *(l2_entry.ppn() as *mut PageTable) };
        let l1_entry = l1_table.get_mut(2);
        assert!(l1_entry.is_valid());
        assert!(!l1_entry.is_leaf());

        // L0 entry at index 3 should be a valid leaf pointing to phys
        let l0_table = unsafe { &mut *(l1_entry.ppn() as *mut PageTable) };
        let l0_entry = l0_table.get_mut(3);
        assert!(l0_entry.is_valid());
        assert!(l0_entry.is_leaf());
        assert_eq!(l0_entry.ppn(), phys);
    }

    #[test]
    fn map_reuses_existing_intermediate_tables() {
        // Map two addresses that share the same L2 and L1 table (L2=1, L1=2)
        // but different L0 indices (L0=3 and L0=4)
        let phys_a: PhysAddr = alloc_test_frame();
        let phys_b: PhysAddr = alloc_test_frame();
        let root_frame = alloc_test_frame();
        let root = unsafe { &mut *(root_frame as *mut PageTable) };

        let alloc_count = std::cell::RefCell::new(0usize);
        let counting_alloc = || {
            *alloc_count.borrow_mut() += 1;
            alloc_test_frame()
        };

        unsafe {
            root.map(
                VirtAddr::from_parts(1, 2, 3, 0),
                phys_a,
                pte_flags::READ,
                &counting_alloc,
            );
            root.map(
                VirtAddr::from_parts(1, 2, 4, 0),
                phys_b,
                pte_flags::READ,
                &counting_alloc,
            );
        }

        // First map allocates 2 intermediate tables (L1 and L0 level).
        // Second map reuses both, so only 2 allocations total.
        assert_eq!(*alloc_count.borrow(), 2);

        // Both leaves should point to their respective physical frames
        let l2_entry = root.get_mut(1);
        let l1_table = unsafe { &mut *(l2_entry.ppn() as *mut PageTable) };
        let l1_entry = l1_table.get_mut(2);
        let l0_table = unsafe { &mut *(l1_entry.ppn() as *mut PageTable) };

        assert_eq!(l0_table.get_mut(3).ppn(), phys_a);
        assert_eq!(l0_table.get_mut(4).ppn(), phys_b);
    }
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
