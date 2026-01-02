//! The Allocator keeps track of a free list of blocks `FreeBlock`. This
//! technique is called 'intrusive list' which means it's a linked list but the
//! metadata about the list, in this case the `next` pointer, is embedded in
//! the node itself and not stored elsewhere.
//!
//! The allocator works by having a block of memory available and storing it as a
//! `FreeBlock` including the size and a next pointer which is null to start with.
//!
//! When an allocation happens, we walk the freelist to find a free block which
//! is large enough. The allocation itself must be atlease
//! `size_of::<FreeBlock>()` as the allocated memory will be converted back to a
//! free block when free'd.
//!
//! The allocator will split the block if possible and give the first part to
//! the user as their allocated memory. This part previously contained the
//! metadata information of the FreeBlock so it needs to be moved to right after
//! the allocated memory for the user. The size of the free block and the
//! pointers in the free list have to be adjusted.
//!
//! When feeing some memory, the allocator overwrites the content with the meta
//! data information of the block that was free'd and then inserts it back into
//! the free list.

// use crate::log;
use core::alloc::{GlobalAlloc, Layout};
use core::cell::UnsafeCell;
use core::mem;
use core::ptr::NonNull;

use crate::logln;

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

    pub unsafe fn init(&self) {
        unsafe { (*self.inner.get()).init() };
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

#[cfg(feature = "stats")]
use alloc::collections::BTreeMap;

#[cfg(feature = "stats")]
extern crate alloc;

#[cfg(feature = "stats")]
pub struct AllocationStats {
    total_allocations: usize,
    total_allocations_bytes: usize,
    /// address to size mapping, if the mapping is in the map, it's still valid
    current_allocations: BTreeMap<usize, usize>,
}

#[cfg(feature = "stats")]
impl AllocationStats {
    pub const fn new() -> Self {
        Self {
            total_allocations: 0,
            total_allocations_bytes: 0,
            current_allocations: BTreeMap::new(),
        }
    }
    fn alloc(&mut self, addr: usize, size: usize) {
        self.total_allocations += 1;
        self.total_allocations_bytes += size;
        *self.current_allocations.entry(addr).or_default() += size;
    }

    fn dealloc(&mut self, addr: usize, size: usize) {
        let Some(entry) = self.current_allocations.get_mut(&addr) else {
            panic!("Stat is missing allocation at addr={addr:#x?}");
        };

        *entry -= size;

        if *entry == 0 {
            self.current_allocations.remove(&addr);
        }
    }
}

pub struct FreeListAllocator {
    pub head: Option<NonNull<FreeBlock>>,
    #[cfg(feature = "stats")]
    stats: AllocationStats,
}

impl FreeListAllocator {
    /// Initializes the `FreeListAllocator`.
    ///
    /// SAFETY: This requires the `_heap_start` and `_heap_end` symbols to be defined.
    pub unsafe fn init(&mut self) {
        let bounds = unsafe { HeapBounds::new() };
        let heap_size = bounds.size();

        let initial_block_ptr = bounds.start as *mut FreeBlock;

        unsafe {
            (*initial_block_ptr).size = heap_size - mem::size_of::<FreeBlock>();
            (*initial_block_ptr).next = core::ptr::null_mut();
        }

        logln!(
            "[ALLOC] Initialized allocator at {:#x} with size of {:#x} ({}KB)",
            bounds.start,
            unsafe { (*initial_block_ptr).size },
            unsafe { (*initial_block_ptr).size } / 1024,
        );

        self.head = Some(NonNull::new(initial_block_ptr).unwrap());
        #[cfg(feature = "stats")]
        {
            self.stats = AllocationStats::new();
        }
    }

    /// This function is used in tests to verify allocations and frees and
    /// returns the total number of currently free bytes in the allocator.
    pub fn free(&self) -> usize {
        let mut total = 0;

        let mut current = self.head;

        unsafe {
            while let Some(block) = current {
                let block_ptr = block.as_ptr();
                total += (*block_ptr).size;
                current = NonNull::new((*block_ptr).next);
            }
        }

        total
    }

    /// Returns the number of individual `FreeBlocks` held by the allocator.
    pub fn free_blocks(&self) -> usize {
        let mut count = 0;

        let mut current = self.head;

        unsafe {
            while let Some(block) = current {
                current = NonNull::new((*block.as_ptr()).next);
                count += 1;
            }
        }

        count
    }

    pub fn alloc(&mut self, layout: Layout) -> *mut u8 {
        let align = layout.align();
        let size = layout.size();

        // The block needs to be at least the size of the `FreeBlock`.
        let actual_size = size.max(mem::size_of::<FreeBlock>());
        let actual_size = align_up(actual_size, align);

        if self.head.is_none() {
            panic!("OOM killed, no more blocks...");
        }

        let mut prev: Option<NonNull<FreeBlock>> = None;
        let mut current: Option<NonNull<FreeBlock>> = self.head;

        logln!("[ALLOC] Allocating {actual_size} bytes");

        unsafe {
            while let Some(block) = current {
                // get raw pointer from the `NonNull`
                let block_ptr = block.as_ptr();

                if (*block_ptr).size >= actual_size {
                    let next_free_block = (*block_ptr).next;

                    // remove this `FreeBlock` from the list
                    match prev {
                        None => {
                            // we're modifying the head block here
                            self.head = NonNull::new(next_free_block);
                        }
                        Some(prev) => {
                            // modify previous `next` pointer to point to the next free block
                            (*prev.as_ptr()).next = next_free_block;
                        }
                    }

                    let remainder = (*block_ptr).size - actual_size;

                    if remainder >= mem::size_of::<FreeBlock>() {
                        let new_block = (block_ptr as usize + actual_size) as *mut FreeBlock;
                        (*new_block).size = remainder;
                        (*new_block).next = (*block_ptr).next;

                        // insert the new block where the old one was
                        match prev {
                            None => self.head = NonNull::new(new_block),
                            Some(prev_ptr) => (*prev_ptr.as_ptr()).next = new_block,
                        }
                    }

                    #[cfg(feature = "stats")]
                    self.stats.alloc(block_ptr as usize, actual_size);

                    return block_ptr as *mut u8;
                }

                prev = current;
                current = NonNull::new((*block_ptr).next);
            }

            // TODO(mt): if this returns null, we need to think about the
            // `alloc_error_handler` and deal with the error elsewhere.  So far
            // panic are fine here and a simple solution.
            panic!("OOM killed, no matching block")
        }
    }

    // TODO(mt) write tests for this?
    pub fn dealloc(&mut self, ptr: *mut u8, layout: Layout) {
        let actual_size = align_up(
            layout.size().max(mem::size_of::<FreeBlock>()),
            layout.align(),
        );

        logln!(
            "[ALLOC] deallocating {actual_size} bytes at {:#x}",
            ptr as usize
        );

        let ptr_addr = ptr as usize;

        // Create new `FreeBlock`
        let new_block = ptr as *mut FreeBlock;
        unsafe {
            (*new_block).size = actual_size;
            (*new_block).next = core::ptr::null_mut();
        }

        #[cfg(feature = "stats")]
        self.stats.dealloc(ptr as usize, actual_size);

        if self.head.is_none() {
            self.head = NonNull::new(new_block);
            return;
        }

        unsafe {
            let head_ptr = self.head.unwrap().as_ptr();

            // Need to insert before head
            if ptr_addr < (head_ptr as usize) {
                if ptr_addr + actual_size == (head_ptr as usize) {
                    (*new_block).size += (*head_ptr).size;
                    (*new_block).next = (*head_ptr).next;
                } else {
                    (*new_block).next = head_ptr;
                }
                self.head = NonNull::new(new_block);
                return;
            }

            let mut prev = head_ptr;

            // walk the list as long as prev.next is smaller then ptr_addr
            // which leaves us with the location where we need to insert the
            // new block to keep the list ordered.
            while !(*prev).next.is_null() && ((*prev).next as usize) < ptr_addr {
                prev = (*prev).next;
            }

            let next = (*prev).next;

            let merge_left = (prev as usize) + (*prev).size == ptr_addr;
            let merge_right = !next.is_null() && ptr_addr + actual_size == (next as usize);

            match (merge_left, merge_right) {
                (true, true) => {
                    (*prev).size += actual_size + (*next).size;
                    (*prev).next = (*next).next;
                }
                (true, false) => {
                    (*prev).size += actual_size;
                }
                (false, true) => {
                    (*new_block).size += (*next).size;
                    (*new_block).next = (*next).next;
                    (*prev).next = new_block;
                }
                (false, false) => {
                    (*new_block).next = next;
                    (*prev).next = new_block;
                }
            }
        }
    }

    pub fn dump_state(&self) {
        let Some(mut current) = (unsafe { self.head.map(|n| n.as_ref()) }) else {
            logln!("========== ALLOCATOR DUMP ==========");
            logln!("No more free memory :(");
            logln!("====================================");
            return;
        };

        let mut i = 0;

        let start_addr = current as *const FreeBlock as usize;

        logln!("========== ALLOCATOR DUMP ==========");
        logln!("Allocator starting at {start_addr:#x}");

        let mut total_size = 0;

        loop {
            logln!("  Block {i} size={} next={:?}", current.size, current.next);
            total_size += current.size;
            if current.next.is_null() {
                break;
            } else {
                current = unsafe { &*current.next };
                i += 1;
            }
        }

        logln!("Total free memory: {total_size}bytes");
        logln!("====================================");
    }
}

pub fn align_up(addr: usize, align: usize) -> usize {
    (addr + align - 1) & !(align - 1)
}

pub struct FreeBlock {
    size: usize,
    next: *mut FreeBlock,
}

impl FreeBlock {
    /// Returns the address of &self
    fn addr(&self) -> usize {
        self as *const FreeBlock as usize
    }
}

struct HeapBounds {
    start: usize,
    end: usize,
}

impl HeapBounds {
    unsafe fn new() -> Self {
        // Defined in `linker.ld`
        //
        // SAFETY: Those are lables inserted by the linker. We only care about
        // the address where those lables got inserted not about the content.
        // This means we don't care about the type, in this case `u8` is
        // choosen by convention. Casting the symbol to an address is safe,
        // dereferencing this pointer to read from that address is not and
        // causes undefined behaviour.
        unsafe extern "C" {
            static _heap_start: u8;
            static _heap_end: u8;
        }

        let start = unsafe { &_heap_start as *const u8 as usize };
        let end = unsafe { &_heap_end as *const u8 as usize };

        Self { start, end }
    }

    fn size(&self) -> usize {
        self.end - self.start
    }
}
