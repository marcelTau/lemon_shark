# 03 Allocators & Testing

## Testing

Testing can be quite difficult when running in `#![no_std]` mode as we can't
rely on Rust's standard test framework. Another constraint is that we need
to execute the code inside of QEMU to work correctly as we're building
for a different target.

For this to work, we need to tweak the `.cargo/config.toml` and our 
`Cargo.toml` files and we need to create a `lib.rs` which exports
the components that we want to test.

Inside of the `Cargo.toml` file, we need to create a `[lib]` tag and make sure
it's not testable by defining `test = false`. If tests are not disabled it
usually causes issues when running normal `cargo test` command without 
specifying which test to run.

Inside of the `lib.rs` file, we can create common functions that can be used
by the tests such as the `panic_handler` and functions to exit QEMU.

### Custom test framework
The `#![custom_test_runner()]` let's us define a function that is used to run
tests when we can't use Rust's standard test runner.

Therefore we can just define a function which takes a `&[&dyn Fn()]` and calls
all of the functions and then exits QEMU.

We also need to `#![reexport_test_harness_main = "test_main"]` to get a symbol
which we can call which represents rust's test harness main function.

Now we can define a custom `_start` entry point in each of our test cases, set
up the things we need there and then call out to the `test_main()` function
and let rust take over.

All the tests that we want to be run, need to be annotated with the 
`#[test_case]` attribute instead of `#[test]` as we don't have access to the
standard test functionality of Rust.

### Running the tests with QEMU
To run the tests using QEMU in a basic `cargo test` run, we need to define a
custom runner.

For that we define the `runner` field of the `target` object in the 
`.cargo/config.toml` file to use `qemu-system-riscv64` with our basic
configuration that we used for running it in the first place.

Now running `cargo test` should work and run our defined test cases inside
of QEMU.

### Exiting QEMU

In our case, the easiest way to exit from QEMU is to use the SBI function call
to the system reset method which allows us to shutdown the system. See the
docs [here](https://lists.riscv.org/g/tech-brs/attachment/361/0/riscv-sbi.pdf)
chapter 10.

## Allocations

The next big goal of this kernel is to allow dynamic memory allocations. We
can do this even though we don't have virtual memory so far but we can just
use physical memory just like we did for the stack and the trap stack.

We can reserve some space in the linker script and call this our heap.

Then we can write a custom allocator which reads the boundaries of the heap
that are set by the linker script and allocate memory for us inside of this
chunk of memory.

### FreeListAllocator

A simple allocator to use here would be a bump allocator. This works by just
allocating new blocks of memory until the memory is full or cleared completely.

This is a very simple way of doing memory allocations but not quite right for
our kernel as we want to be able to free memory and reuse it later.

A `FreeListAllocator` is a simple way of doing this. It stores blocks of free
memory in `FreeBlock`s and let's us allocate chunks out of this block.

Each block contains some metadata at the beginning of the block such as its
size and a pointer to the next `FreeBlock`.

To start with the allocator has a single `FreeBlock` of the total size of our
reserved memory.

When we want to allocate, we split the block up and return a pointer to
the available memory to the user. We then need to move our `FreeBlock` up by
the requested number of bytes and restore and modify our metadata.

For example:

Allocator starts with:
```
+------------------------------+
| md |  ...                    |
+------------------------------+

A single block with some metadata at the beginning. 
Let's say size = 1024 & next = null.

User allocates 80 bytes.

+-----------------------------+
| user | md |  ...            |
+-----------------------------+

Now the first 80 bytes are allocated memory for the user and a pointer was
returned to the start of this memory.

`md` needs to be updated and moved as now it's size is it's previous size -= 80
for the allocation.

NOTE: the user allocated block does not contain any metadata anymore. This is
an optimization to save space and has an important invariant. It is not 
possible to allocate blocks < size_of<FreeBlock> as the free'd block, will
insert it's metadata right in the allocated memory. Other then that blocks
need to be aligned.

Freeing the blocks is a bit tricky and involves some pointer arithmetic as
we want to merge adjacent blocks together to avoid fragmentation.

For example in the above example, if the user free's the block, we don't want
to end up with 2 free blocks in our list as this limits the size of allocations
that we can manage. We'd rather have a single block where we can split up as
much as we need.
```

### Alignments

Rust requires the global allocator to follow the alignment requirements of
the `Layout` passed to the `alloc()` and `dealloc()` functions. This means
we have another constraint for the allocator that we need to take care of.

We also need to align all the structs that we write to memory, we can set a
minimum alignment for our allocator which ensures that all user pointers and
internal `FreeBlock` are alinged.

### Global alloc

Using the `#[global_allocator]` annotation on a static instance of our 
allocation sets the global allocator and allows us to finally use the core
collections of the rust standard library. Well at least most of them, to be
specific the ones that are re-exported in the `alloc` crate.

We can use `extern crate alloc` and `use alloc::vec::Vec` to pull in the `Vec`
type and we can finally use vectors in our kernel!

