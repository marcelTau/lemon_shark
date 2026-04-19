# Virtual memory

The next big topic in this series is virtual memory. This will be a broader
introduction to virtual memory before we implement it later on.

When a user space program is reading or writing memory at some address it is
not the actual physical address which it is accessing.

If you want to read from an address, you ask the kernel for the data at this
address and it returns it to you. You (as the user-space program) have no idea
if the data is actually stored at this address or not, an neither should you!

Virtual memory is an abstraction as only the kernel should have access to the
physical memory to avoid user-space programs accessing other processes memory
or overwriting data of another process.

But how does the kernel know where to look when a user-space program asks for
data from a specific address?

## Pages

We already know the idea of dividing a big blob of memory into blocks (or
pages) from the filesystem implementation.

We use the same technique here and divide all of RAM into aligned pages. Each
page is 4096 bytes in size and has to be aligned to that boundary.

This is important to know because all the structures that we're going to
discuss here are implemented around or working with pages.

## Page Table

The first and probably most important component that we will look at are called
page tables.

A page table is just a simple lookup table. The kernel keeps a page table for
every running process and it uses the page table to store the mapping from
virtual addresses and physical addresses.

A page table is exactly 4096 bytes in size and can store 512 pointers on a
64-bit system.

Today we usually have multiple levels of page tables to create more accessible
address space. We're going to implement the `sv39` standard which has 3 levels
of page tables.

This means that each process has a single page table. Each entry of this can
point to another 512-entry page table. Each of those entries again point to
another page table, and entries in this table point to physical addresses.

So why is this done like this?

1. This structure is very scalable and only requires used page-tables to exist
   as it doens't allocate all the possible page tables. If a program only uses
   memory that can fit on one page, it might only have a single entry in it's
   'level 2' (outmost) page table which points to another page table that only
   has a single entry and so forth until it reached the physical address.

2. This format works really well with the encoding of the indexes inside of the
   virtual address.

## Encoding

Alright cool, now we know that there is a multi-level lookup table and somehow
we can use this to map a virtual address to a physical address. But how do we
actually do this?

The idea is that the virtual address that the kernel hands out to the
user-space process already contains all the indexes into the page tables of
this process along with an offset that is used to index into the pysical page
at the end.

In simple terms we can say that a 64-bit virtual address consist of 3 * 9bit
blocks that are the indexes into the 3 tables plus an offset at the end into
the page.

## MMU

Alright now that we know how the translation works, we just need to implement
this in the kernel for each memory access right?

No. Not really. This is done by hardware and is done inside of the MMU (memory
management unit) on the CPU. On RISC-V we need to set the `satp` register to
point to the `level 2` (in our case) page table of the currently running
process.

The management of this will be done in our scheduler when we get around to
implementing processes in one of the next chapters.

## TLB (Translation look-aside buffer)

So the MMU needs to do this expensive translation with multiple indirections
and pointer jumps each time when a process is accessing memory?

Yes and no. The MMU stores recently translated addresses in a cache, called
the translation look-aside buffer or short TLB. This is one of the reasons we
care about spartial locality in our user-space programs so that our access
patters are TLB friendly and the MMU doesn't need to do the translation over
and over again.

Now if you thought about virtual addresses for a bit you realized that it's
entirely possible that 2 user-space processes access the same virtual address
which the kernel mapped to a different physical address. 

So how does the MMU handle this?

It's actually pretty simple and it just stores the ASID (Adress space ID) of 
each process along with the translation. This is an optimization as otherwise
the TLB would have to be fully flushed on each process switch to invalidate
all the translations. This would be costly.

Invalidating the TLB is an important step that we also have to do when writing
to the `satp` register. It tells the MMU that we're now using a different ASID.
