use core::alloc::{GlobalAlloc, Layout};
use core::cell::UnsafeCell;
use core::mem;

use crate::logln;
use crate::println;

use core::ops::ControlFlow;

pub fn align_up(addr: usize, align: usize) -> usize {
    (addr + align - 1) & !(align - 1)
}

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

    pub unsafe fn dump_state(&self) {
        unsafe { (*self.inner.get()).dump_state() };
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

#[repr(transparent)]
#[derive(Clone, Copy, Debug)]
pub struct AlignedPtr<T>(*mut T);

impl<T> AlignedPtr<T> {
    /// Minimum alignment for data pointers to make `AllocationMetaData`
    /// automatically aligned right before the pointer.
    const MIN_ALIGN: usize = 8;

    fn new(addr: usize) -> Self {
        Self(align_up(addr, mem::align_of::<T>().max(Self::MIN_ALIGN)) as *mut T)
    }

    fn new_with(addr: usize, align: usize) -> Self {
        Self(align_up(addr, align.max(Self::MIN_ALIGN)) as *mut T)
    }

    fn as_ptr(&self) -> *mut T {
        self.0
    }

    fn as_addr(&self) -> usize {
        self.0 as usize
    }

    /// This enforces the correct alignment of `AllocationMetaData` as
    /// `Self::MIN_ALIGN` is 8 and hence writing the metadata right before
    /// and aligned address automatically aligns it as well.
    ///
    /// SAFETY: Caller must ensure there is enough valid memory right infront
    /// of the ptr.
    unsafe fn write_metadata(&self, metadata: AllocationMetaData) {
        let md_addr = self.as_addr() - mem::size_of::<AllocationMetaData>();

        unsafe {
            *(md_addr as *mut AllocationMetaData) = metadata;
        }
    }

    unsafe fn read_metadata(&self) -> &AllocationMetaData {
        let md_addr = self.as_addr() - mem::size_of::<AllocationMetaData>();
        unsafe { &*(md_addr as *mut AllocationMetaData) }
    }
}

pub struct FreeListAllocator {
    pub head: Option<AlignedPtr<FreeBlock>>,
}

impl FreeListAllocator {
    pub unsafe fn init(&mut self) {
        let bounds = unsafe { HeapBounds::new() };
        let heap_size = bounds.size();

        let aligned_ptr: AlignedPtr<FreeBlock> = AlignedPtr::new(bounds.start);

        unsafe {
            (*aligned_ptr.as_ptr()).size = heap_size - mem::size_of::<FreeBlock>();
            (*aligned_ptr.as_ptr()).next = None;
        }

        logln!(
            "[ALLOC] Initialized allocator at {:#x} with size of {:#x} ({}KB)",
            bounds.start,
            unsafe { (*aligned_ptr.as_ptr()).size },
            unsafe { (*aligned_ptr.as_ptr()).size } / 1024,
        );

        self.head = Some(aligned_ptr);
    }

    pub fn free(&self) -> usize {
        let mut total = 0;

        let _ = Self::walk_list(self.head, |current, _| {
            total += unsafe { (*current.as_ptr()).size };
            ControlFlow::<(), _>::Continue(())
        });

        total
    }

    pub fn free_blocks(&self) -> usize {
        let mut total = 0;

        let _ = Self::walk_list(self.head, |_, _| {
            total += 1;
            ControlFlow::<(), _>::Continue(())
        });

        total
    }

    fn walk_list<F, R>(start: Option<AlignedPtr<FreeBlock>>, mut f: F) -> Option<R>
    where
        F: FnMut(AlignedPtr<FreeBlock>, Option<AlignedPtr<FreeBlock>>) -> ControlFlow<R>,
    {
        let mut prev: Option<AlignedPtr<FreeBlock>> = None;
        let mut current: Option<AlignedPtr<FreeBlock>> = start;

        while let Some(block) = current {
            if let ControlFlow::Break(res) = f(block, prev) {
                return Some(res);
            };
            prev = current;
            current = unsafe { (*block.as_ptr()).next };
        }
        None
    }

    /// Size is always aligned to 8
    /// Each pointer is aligned correctly
    /// The FreeBlock's in the list are all aligned.
    ///
    /// Because the DataPointer & Size are both aligned, the FreeBlock should be automatically aligned.
    pub fn alloc(&mut self, layout: Layout) -> *mut u8 {
        let align = layout.align().max(8);
        let size = align_up(layout.size(), 8);

        logln!(
            "[ALLOC] allocating {size} (req: {}) bytes with alignment {align} (req: {})",
            layout.size(),
            layout.align()
        );

        let result = unsafe {
            Self::walk_list(self.head, |block, prev| {
                let Some(aligned_ptr) = (*block.as_ptr()).can_allocate(align, size) else {
                    return ControlFlow::Continue(());
                };

                logln!("[ALLOC] Found block to allocate");

                let block_ptr = *block.as_ptr();

                // Required size for a valid new block
                let required_size =
                    mem::size_of::<FreeBlock>().max(mem::size_of::<AllocationMetaData>());

                // Number of bytes left of the allocation
                let bytes_left = (aligned_ptr.as_addr() - mem::size_of::<AllocationMetaData>())
                    - block.as_addr();

                let bytes_right =
                    (block.as_addr() + block_ptr.size) - (aligned_ptr.as_addr() + size);

                if bytes_left >= required_size {
                    logln!("[ALLOC] Splitting block to the left");
                }

                if bytes_right >= required_size {
                    logln!("[ALLOC] Splitting block to the right");
                }

                let metadata = match (bytes_left >= required_size, bytes_right >= required_size) {
                    (false, false) => {
                        // remove this `FreeBlock` from the list
                        match prev {
                            Some(prev) => (*prev.as_ptr()).next = block_ptr.next,
                            None => self.head = block_ptr.next,
                        }

                        // No need to change the prev/head as it's already set
                        AllocationMetaData {
                            start_addr: block.as_addr(),
                            size: block_ptr.size,
                        }
                    }
                    (true, false) => {
                        let left_block: AlignedPtr<FreeBlock> = AlignedPtr::new(block.as_addr());
                        (*left_block.as_ptr()).size = bytes_left;
                        (*left_block.as_ptr()).next = block_ptr.next;

                        // prev or head is still set correctly as the left block hasn't moved.

                        AllocationMetaData {
                            start_addr: block.as_addr() + bytes_left,
                            size: block_ptr.size - bytes_left,
                        }
                    }
                    (false, true) => {
                        let right_block: AlignedPtr<FreeBlock> =
                            AlignedPtr::new(aligned_ptr.as_addr() + size);

                        (*right_block.as_ptr()).size = bytes_right;
                        (*right_block.as_ptr()).next = block_ptr.next;

                        match prev {
                            Some(prev) => (*prev.as_ptr()).next = Some(right_block),
                            None => self.head = Some(right_block),
                        }

                        AllocationMetaData {
                            start_addr: block.as_addr(),
                            size: bytes_left + mem::size_of::<AllocationMetaData>() + size,
                        }
                    }
                    (true, true) => {
                        let left_block: AlignedPtr<FreeBlock> = AlignedPtr::new(block.as_addr());
                        let right_block: AlignedPtr<FreeBlock> =
                            AlignedPtr::new(aligned_ptr.as_addr() + size);

                        (*left_block.as_ptr()).size = bytes_left;
                        (*right_block.as_ptr()).size = bytes_right;

                        (*left_block.as_ptr()).next = Some(right_block);
                        (*right_block.as_ptr()).next = block_ptr.next;

                        AllocationMetaData {
                            start_addr: block.as_addr() + bytes_left,
                            size: block_ptr.size - bytes_left - bytes_right,
                        }
                    }
                };

                logln!("[ALLOC] AllocationMetaData: {metadata:#0x?}");

                aligned_ptr.write_metadata(metadata);
                logln!("[ALLOC] wrote metadata");
                ControlFlow::Break(aligned_ptr)
            })
        };

        result.expect("Out of memory").as_ptr()
    }

    pub fn dealloc(&mut self, ptr: *mut u8, _layout: Layout) {
        // Don't re-align! The ptr is already the aligned address we returned from alloc()
        let aligned_ptr: AlignedPtr<u8> = AlignedPtr(ptr);
        let (size, start_addr) = {
            // NOTE: limit scope of metadata as we're writing into that memory
            // and thus change the value.
            let metadata = unsafe { aligned_ptr.read_metadata() };
            (metadata.size, metadata.start_addr)
        };

        let new_block: AlignedPtr<FreeBlock> = AlignedPtr::new(start_addr);

        logln!("[ALLOC] deallocating {size} bytes at {start_addr:#x}",);

        unsafe {
            (*new_block.as_ptr()).size = size;
            (*new_block.as_ptr()).next = None;
        }

        if self.head.is_none() {
            logln!("Head is none!");
            self.head = Some(new_block);
            return;
        }

        unsafe {
            let head = self.head.unwrap();

            if start_addr < head.as_addr() {
                logln!(
                    "start_addr={start_addr} size={size} head={:#x?} diff={}",
                    head.as_addr(),
                    head.as_addr() - (start_addr + size),
                );
                if start_addr + size == head.as_addr() {
                    (*new_block.as_ptr()).size += (*head.as_ptr()).size;
                    (*new_block.as_ptr()).next = (*head.as_ptr()).next;
                } else {
                    (*new_block.as_ptr()).next = Some(head);
                }

                self.head = Some(new_block);
                return;
            }

            let mut prev = head;

            while (*prev.as_ptr()).next.is_some()
                && ((*prev.as_ptr())
                    .next
                    .is_some_and(|next| next.as_addr() < start_addr))
            {
                prev = (*prev.as_ptr()).next.unwrap();
            }

            let next = (*prev.as_ptr()).next;

            let merge_left = (prev.as_addr()) + (*prev.as_ptr()).size == start_addr;
            let merge_right = next.is_some_and(|next| start_addr + size == next.as_addr());

            logln!("merge_left: {}, merge_right: {}", merge_left, merge_right);

            match (merge_left, merge_right) {
                (true, true) => {
                    (*prev.as_ptr()).size += size + (*next.unwrap().as_ptr()).size;
                    (*prev.as_ptr()).next = (*next.unwrap().as_ptr()).next;
                }
                (true, false) => {
                    (*prev.as_ptr()).size += size;
                }
                (false, true) => {
                    (*new_block.as_ptr()).size += (*next.unwrap().as_ptr()).size;
                    (*new_block.as_ptr()).next = (*next.unwrap().as_ptr()).next;
                    (*prev.as_ptr()).next = Some(new_block);
                }
                (false, false) => {
                    (*new_block.as_ptr()).next = next;
                    (*prev.as_ptr()).next = Some(new_block);
                }
            }
        }
    }

    pub fn dump_state(&self) {
        if self.head.is_none() {
            println!("========== ALLOCATOR DUMP ==========");
            println!("No more free memory :(");
            println!("====================================");
            return;
        }

        let mut i = 0;

        let start_addr = self.head.unwrap().as_addr();

        println!("========== ALLOCATOR DUMP ==========");
        println!("Allocator starting at {start_addr:#x}");

        let mut total_size = 0;

        unsafe {
            Self::walk_list(self.head, |current, _| {
                println!(
                    "  Block {i} at={:#0x} size={} next={:?}",
                    current.as_addr(),
                    (*current.as_ptr()).size,
                    (*current.as_ptr()).next
                );
                total_size += (*current.as_ptr()).size;
                if (*current.as_ptr()).next.is_none() {
                    return ControlFlow::Break(());
                }
                i += 1;
                ControlFlow::Continue(())
            });
        }

        println!("Total free memory: {total_size} bytes");
        println!("====================================");
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct FreeBlock {
    pub size: usize,
    pub next: Option<AlignedPtr<FreeBlock>>,
}

impl FreeBlock {
    fn can_allocate(&self, align: usize, size: usize) -> Option<AlignedPtr<u8>> {
        let self_addr = self as *const Self as usize;
        let self_end_addr = self_addr + self.size;

        // Need space to store the MetaData
        let next_possible_addr = self_addr + mem::size_of::<AllocationMetaData>();

        // Align the address
        let next_possible_addr_aligned = AlignedPtr::new_with(next_possible_addr, align);

        // Check if the aligned address + the requested size fits into this block
        if self_end_addr >= next_possible_addr_aligned.as_addr() + size {
            Some(next_possible_addr_aligned)
        } else {
            None
        }
    }
}

#[repr(C)]
#[derive(Debug)]
struct AllocationMetaData {
    /// Start address of this allocation including padding and meta data.
    start_addr: usize,

    /// Actual size of the allocation from the Allocators PoV.
    size: usize,
}

pub struct HeapBounds {
    pub start: usize,
    pub end: usize,
}

impl HeapBounds {
    pub unsafe fn new() -> Self {
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

    pub fn size(&self) -> usize {
        self.end - self.start
    }
}
