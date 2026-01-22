//! How the filesystem works:
//!
//! The disk is split into blocks of SIZE, for now 512 bytes.
//!
//! We store i-nodes which contain metadata about files such as the size, and
//! which blocks this file is using in special i-node reserved blocks in the
//! first `INODE_BLOCKS` blocks after the superblock.
//!
//! The first entry is a superblock containing metadata about the state of the
//! filesystem and should be read when mounted and flushed when unmounted.
//! This will be written to block 0 of the data block.
//!
//! Directories are created by setting the flag of the INode. This indicates
//! that we need to interpret the entries in the associated blocks as a list
//! of `DirEntry`s instead of raw data blocks of the file.
//!
//! Layout of the blocks:
//! 0.      Superblock
//! 1-10.   INode
//! 10-end  Data

extern crate alloc;
use crate::bytereader::ByteReader;
use crate::{logln, print, println, ramdisk};
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::mem;

/// What kind of indexes do we have?
///
/// BlockIndex - actual number of the block
/// INodeIndex - 0 is the first BlockIndex of the INode range
/// DataIndex  - 0 is the first BlockIndex of the data range

/// An Index into the blocks used for the `ramdisk`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BlockIndex(pub u32);

/// A `ByteOffset` to something inside of a block.
#[derive(Debug)]
struct ByteOffset(u32);

impl ByteOffset {
    fn range<T>(&self) -> core::ops::Range<usize> {
        self.0 as usize..self.0 as usize + mem::size_of::<T>()
    }
}

/// This is the actual index of the INode.
#[derive(Debug, Copy, Clone)]
struct INodeIndex(u32);

impl INodeIndex {
    /// Returns the `BlockIndex` and the offset inside of the block for that
    /// `INode`.
    fn to_block_index(&self) -> (BlockIndex, ByteOffset) {
        let block_index = BlockIndex(INODE_START as u32 + (self.0 / INODES_PER_BLOCK as u32));
        let offset =
            ByteOffset((self.0 % INODES_PER_BLOCK as u32) * mem::size_of::<INode>() as u32);
        (block_index, offset)
    }
}

struct DirEntryIndex(u32);
impl DirEntryIndex {
    fn to_block_index(&self) -> (BlockIndex, ByteOffset) {
        let block_index = BlockIndex(DATA_START as u32 + (self.0 / DIR_ENTRY_PER_BLOCK as u32));
        let offset =
            ByteOffset((self.0 % DIR_ENTRY_PER_BLOCK as u32) * mem::size_of::<DirEntry>() as u32);
        (block_index, offset)
    }
}

#[derive(Clone, Copy, Debug)]
#[repr(transparent)]
struct DataBlockIndex(u32);
impl DataBlockIndex {
    fn to_block_index(self) -> BlockIndex {
        BlockIndex(DATA_START as u32 + self.0)
    }

    fn value(&self) -> u32 {
        self.0
    }
}

pub(crate) const BLOCK_SIZE: usize = 512;
const INODE_BLOCKS: usize = 10;
const INODE_START: usize = 1;
const INODES_PER_BLOCK: usize = BLOCK_SIZE / core::mem::size_of::<INode>();
const DIR_ENTRY_PER_BLOCK: usize = BLOCK_SIZE / core::mem::size_of::<DirEntry>();
const DATA_START: usize = INODE_START + INODE_BLOCKS + 1;
const INODE_BITMAP_SIZE: usize = (INODE_BLOCKS * INODES_PER_BLOCK) / 32;
const DATA_BITMAP_SIZE: usize = (ramdisk::total_blocks() - 1 /*superblock*/ - INODE_BLOCKS) / 32;
const MAGIC: u64 = 0x4e4f4d454c; // lemon (le)

/// The `INode` contains metadata about a file.
///
/// Memory layout:
/// `size`          4 bytes
/// `blocks`        64 bytes
/// `is_directory`  1 byte
/// `padding`       3 bytes
///
/// *Important* do not change the layout of this struct or reorder the fields.
#[repr(C)]
#[derive(Debug)]
struct INode {
    size: u32,
    /// Used blocks of this file. TODO(mt): should be option<nonzeroU32>
    blocks: [DataBlockIndex; 16],
    is_directory: bool,
}

impl INode {
    fn empty_directory() -> Self {
        INode {
            size: 0,
            is_directory: true,
            blocks: core::array::from_fn(|_| DataBlockIndex(0)),
        }
    }

    fn from_bytes(bytes: &[u8]) -> Self {
        let size = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let mut blocks: [DataBlockIndex; 16] = [DataBlockIndex(0); 16];
        let mut i = 4;

        (0..16).for_each(|idx| {
            let value = u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
            blocks[idx] = DataBlockIndex(value);
            i += 4;
        });

        let is_directory = bytes[68] != 0;

        Self {
            size,
            is_directory,
            blocks,
        }
    }

    fn to_bytes(&self) -> [u8; mem::size_of::<INode>()] {
        let mut bytes = [0u8; mem::size_of::<INode>()];

        bytes[0..4].copy_from_slice(&self.size.to_le_bytes());
        let current_offset = 4;
        for i in 0..16 {
            let start = current_offset + (i * 4);
            bytes[start..start + 4].copy_from_slice(&self.blocks[i].0.to_le_bytes());
        }
        bytes[68] = if self.is_directory { 1 } else { 0 };

        bytes
    }
}

/// The `DirEntry` contains metadata about a directory.
#[repr(C)]
struct DirEntry {
    /// Name of the directory
    name: [u8; 24],
    /// INode index of this directory
    inode: INodeIndex,
}
use core::fmt::Debug;

impl core::fmt::Debug for DirEntry {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> Result<(), core::fmt::Error> {
        write!(
            f,
            "DirEntry name=\"{}\" inode={:?}",
            self.name(),
            self.inode
        )
    }
}

impl DirEntry {
    fn new(name_string: String, inode: INodeIndex) -> Self {
        let mut name = [0u8; 24];
        let bytes = name_string.as_bytes();
        let len = bytes.len().min(24);

        name[..len].copy_from_slice(&bytes[..len]);

        DirEntry { name, inode }
    }

    fn name(&self) -> String {
        String::from_utf8(self.name.to_vec()).unwrap()
    }

    fn from_bytes(bytes: &[u8]) -> Self {
        let mut reader = ByteReader::new(bytes);
        let name = reader.read_bytes(24).try_into().unwrap();
        let inode = INodeIndex(reader.read_u32());
        Self { name, inode }
    }

    fn to_bytes(&self) -> [u8; mem::size_of::<DirEntry>()] {
        let mut bytes = [0u8; mem::size_of::<DirEntry>()];

        bytes[0..24].copy_from_slice(self.name.as_slice());
        bytes[24..28].copy_from_slice(&self.inode.0.to_le_bytes());

        bytes
    }
}

/// The `Superblock` contains counts & pointers to strucutres used and metadata
/// about the state of the allocator.
#[repr(C)]
#[derive(Debug, PartialEq)]
pub struct SuperBlock {
    // 32
    magic: u64,
    block_size: u32,
    total_blocks: u32,

    inode_table_start: u32,
    inode_table_blocks: u32,

    data_start: u32,

    // 8 bytes
    inode_bitmap: Bitmap<INODE_BITMAP_SIZE>,
    // 252 bytes + 4 bytes padding at the end
    data_bitmap: Bitmap<DATA_BITMAP_SIZE>,
}

impl SuperBlock {
    pub const fn default_superblock() -> Self {
        SuperBlock {
            magic: MAGIC,
            block_size: BLOCK_SIZE as u32,
            total_blocks: ramdisk::total_blocks() as u32,
            inode_table_start: INODE_START as u32,
            inode_table_blocks: INODE_BLOCKS as u32,
            data_start: DATA_START as u32,
            inode_bitmap: Bitmap::<INODE_BITMAP_SIZE>::new(),
            data_bitmap: Bitmap::<DATA_BITMAP_SIZE>::new(),
        }
    }

    fn from_bytes(bytes: &[u8]) -> Self {
        let mut reader = ByteReader::new(bytes);

        let magic = reader.read_u64();
        let block_size = reader.read_u32();
        let total_blocks = reader.read_u32();
        let inode_table_start = reader.read_u32();
        let inode_table_blocks = reader.read_u32();
        let data_start = reader.read_u32();

        let inode_bitmap =
            Bitmap::<INODE_BITMAP_SIZE>::from_bytes(reader.read_bytes(INODE_BITMAP_SIZE * 4));
        let data_bitmap =
            Bitmap::<DATA_BITMAP_SIZE>::from_bytes(reader.read_bytes(DATA_BITMAP_SIZE * 4));

        Self {
            magic,
            block_size,
            total_blocks,
            inode_table_start,
            inode_table_blocks,
            data_start,
            inode_bitmap,
            data_bitmap,
        }
    }

    fn to_bytes(&self) -> [u8; mem::size_of::<SuperBlock>()] {
        let mut bytes = [0u8; mem::size_of::<SuperBlock>()];

        bytes[0..8].copy_from_slice(&self.magic.to_le_bytes());
        bytes[8..12].copy_from_slice(&self.block_size.to_le_bytes());
        bytes[12..16].copy_from_slice(&self.total_blocks.to_le_bytes());
        bytes[16..20].copy_from_slice(&self.inode_table_start.to_le_bytes());
        bytes[20..24].copy_from_slice(&self.inode_table_blocks.to_le_bytes());
        bytes[24..28].copy_from_slice(&self.data_start.to_le_bytes());

        let start = 28;
        let end = start + INODE_BITMAP_SIZE * 4;
        bytes[start..end].copy_from_slice(self.inode_bitmap.to_bytes());

        let start = end;
        let end = start + DATA_BITMAP_SIZE * 4;
        bytes[start..end].copy_from_slice(self.data_bitmap.to_bytes());

        bytes
    }
}

// enum Offset {
//     /// INode offset, *not block offset*.
//     INode(usize),
//     SuperBlock,
//     Data(usize),
//     DirEntry(usize),
//     Block(usize),
// }
//
// #[derive(Copy, Clone, Debug)]
// struct BlockIndex(usize);
//
// impl BlockIndex {
//     fn from_inode_index(inode_index: usize) -> Self {
//         let block_index = INODE_START + (inode_index / INODES_PER_BLOCK);
//         if block_index > INODE_START + INODE_BLOCKS {
//             panic!("No more inode blocks");
//         }
//         Self(block_index)
//     }
//
//     fn from_dir_entry_index(dir_entry_index: usize) -> Self {
//         let block_index = DATA_START + (dir_entry_index / DIR_ENTRY_PER_BLOCK);
//         if block_index > ramdisk::total_blocks() {
//             panic!("No free block to store DirEntry");
//         }
//         Self(block_index)
//     }
//
//     fn superblock() -> Self {
//         Self(0)
//     }
// }

#[derive(Debug, PartialEq)]
struct Bitmap<const WORDS: usize> {
    arr: [u32; WORDS],
}

impl<const WORDS: usize> Bitmap<WORDS> {
    const fn new() -> Self {
        Self { arr: [0u32; WORDS] }
    }

    fn from_bytes(bytes: &[u8]) -> Self {
        assert_eq!(bytes.len(), WORDS * 4);

        let mut arr = [0u32; WORDS];
        for (i, chunk) in bytes.chunks(4).enumerate() {
            arr[i] = u32::from_le_bytes(chunk.try_into().unwrap());
        }
        Self { arr }
    }

    fn to_bytes(&self) -> &[u8] {
        unsafe { core::slice::from_raw_parts(self.arr.as_ptr() as *const u8, WORDS * 4) }
    }

    fn set(&mut self, index: u32) {
        assert!(index < WORDS as u32 * 32);

        let arr_index = index / 32;
        let bit_index = index % 32;

        self.arr[arr_index as usize] |= 1 << bit_index;
    }

    fn unset(&mut self, index: u32) {
        assert!(index < WORDS as u32 * 32);

        let arr_index = index / 32;
        let bit_index = index % 32;

        self.arr[arr_index as usize] &= !(1 << bit_index);
    }

    fn is_set(&self, index: u32) -> bool {
        assert!(index < WORDS as u32 * 32);

        let arr_index = index / 32;
        let bit_index = index % 32;

        self.arr[arr_index as usize] & (1 << bit_index) > 0
    }

    /// Find the first free block in the bitmap.
    fn find_free(&self) -> Option<u32> {
        for (arr_idx, bits) in self.arr.iter().enumerate() {
            if bits != &u32::MAX {
                let res = (!bits).trailing_zeros();
                logln!("[FS] find free found at {arr_idx} {res}");
                return Some(arr_idx as u32 * 32 + res);
            }
        }

        None
    }
}

// TODO(mt): make it impossible to re-call init and break things.
static FS: spin::Mutex<LockedFilesystem> = spin::Mutex::new(LockedFilesystem::new());

use core::cell::UnsafeCell;

struct LockedFilesystem {
    inner: UnsafeCell<Option<Filesystem>>,
}

impl LockedFilesystem {
    pub const fn new() -> Self {
        Self {
            inner: UnsafeCell::new(None),
        }
    }

    pub fn init(&mut self, filesystem: Filesystem) {
        *self.inner.get_mut() = Some(filesystem);
    }

    pub fn inner(&mut self) -> &mut Filesystem {
        self.inner.get_mut().as_mut().unwrap()
    }
}

mod fs {
    use super::{INode, ramdisk};
    use core::mem;

    use super::*;

    /// Writes the superblock to block_index 0
    pub fn write_superblock(superblock: &SuperBlock) {
        let mut buf = [0u8; BLOCK_SIZE];
        buf[0..mem::size_of::<SuperBlock>()].copy_from_slice(&superblock.to_bytes());
        ramdisk::write_block(BlockIndex(0), &buf).unwrap();
    }

    /// Reads the superblock from block_index 0
    pub fn read_superblock() -> SuperBlock {
        let mut buf = [0u8; BLOCK_SIZE];
        ramdisk::read_block(BlockIndex(0), &mut buf).unwrap();
        let superblock = SuperBlock::from_bytes(&buf[0..mem::size_of::<SuperBlock>()]);

        if superblock.magic == MAGIC {
            superblock
        } else {
            SuperBlock::default_superblock()
        }
    }

    /// Writes an `INode` into the block at the specific offset into the inode
    /// blocks.
    fn write_inode_inner(inode_index: INodeIndex, inode: &INode) {
        // Creating the buffer to write the INode to.
        let mut buf = [0u8; BLOCK_SIZE];

        // Calculate the `BlockIndex` and the `ByteOffset` for the `INode`
        // to be written to.
        let (block_index, byte_offset) = inode_index.to_block_index();

        // Reading the block into `buf` to append the `INode` to it.
        ramdisk::read_block(block_index, &mut buf).unwrap();

        // Calculating the byte_offset inside of the block.
        logln!("[FS] Writing to block index {block_index:?} at byte_offset={byte_offset:?}");

        buf[byte_offset.range::<INode>()].copy_from_slice(inode.to_bytes().as_slice());

        // Writing the block to memory.
        ramdisk::write_block(block_index, &buf).unwrap();
    }

    /// Writes a new `INode` to the ramdisk.
    pub(crate) fn write_inode(inode: &INode) -> INodeIndex {
        // Finds the next free block in the `INodeBitmap`.
        let free_block = INodeIndex((*FS.lock()).inner().inode_bitmap.find_free().unwrap());

        logln!("[FS] Writing INode to next_free_block={free_block:?}");

        // Set this block to be used.
        (*FS.lock()).inner().inode_bitmap.set(free_block.0);

        // Write the `INode` to the block
        write_inode_inner(free_block, inode);

        free_block
    }

    /// Read the `INode` at offset
    pub(crate) fn read_inode_inner(inode_index: INodeIndex) -> INode {
        let mut buf = [0u8; BLOCK_SIZE];
        let (block_index, byte_offset) = inode_index.to_block_index();
        ramdisk::read_block(block_index, &mut buf).unwrap();
        INode::from_bytes(&buf[byte_offset.range::<INode>()])
    }

    pub(crate) fn update_inode(inode_index: INodeIndex, inode: &INode) {
        write_inode_inner(inode_index, inode);
    }

    /// Writing a `DirEntry` needs to check if the `INode` already has a block
    /// which has some free space and we can write the new `DirEntry` to that
    /// block. If not then we need to allocate a new block and attach this to
    /// the `INode`.
    ///
    /// TODO(mt): here we're modifying the INode and not the memory on the
    /// ramdisk. This is fine as long as we store some Nodes locally but
    /// eventually we need to write them to memory.
    pub(crate) fn write_dir_entry(entry: DirEntry, inode: &mut INode) {
        let mut buf = [0u8; BLOCK_SIZE];

        // Calculate the currently used entries based on the size of the `INode`
        let current_entries = inode.size as usize / mem::size_of::<DirEntry>();

        // Get the index into the blocks of the `INode`
        let inode_internal_block_index = current_entries / DIR_ENTRY_PER_BLOCK;

        logln!("[FS] Writing DirEntry {entry:?} at block_index={inode_internal_block_index:?}");

        if inode_internal_block_index > 16 {
            panic!("Can not hold more than 16 blocks of DirEntries");
        }

        // println!("** {inode_internal_block_index}");

        if inode.blocks[inode_internal_block_index].0 == 0 {
            let free = (*FS.lock()).inner().data_bitmap.find_free().unwrap();
            let free = DataBlockIndex(free);
            inode.blocks[inode_internal_block_index] = free;
            (*FS.lock()).inner().data_bitmap.set(free.0);
        }

        let data_block_index = inode.blocks[inode_internal_block_index];

        // println!("** {data_block_index:?}");
        // Get the offset inside of this block
        let offset_in_block = (current_entries % DIR_ENTRY_PER_BLOCK) * mem::size_of::<DirEntry>();

        let block_index = data_block_index.to_block_index();

        // Read this block into `buf`.
        ramdisk::read_block(block_index, &mut buf).unwrap();

        // Write `DirEntry` into the `buf`.
        buf[offset_in_block..offset_in_block + mem::size_of::<DirEntry>()]
            .copy_from_slice(entry.to_bytes().as_slice());

        // Write `buf` to memory.
        ramdisk::write_block(block_index, &buf).unwrap();

        // Increment the `size` by the size of the `DirEntry`.
        inode.size += mem::size_of::<DirEntry>() as u32;
    }

    /// Reads all the directory entries for that INode and returns them as a
    /// Vec.
    pub(crate) fn read_dir_entry(inode: &INode) -> Vec<DirEntry> {
        if inode.size == 0 {
            return Vec::new();
        }

        let max_items = inode.size as usize / mem::size_of::<DirEntry>();
        let mut res = Vec::with_capacity(max_items);
        let mut buf = [0u8; BLOCK_SIZE];
        let mut offset = 0;

        let mut read_items = 0;

        // This loop is safe as the size of
        while read_items < max_items {
            let data_block_index = inode.blocks[read_items];
            let block_index = data_block_index.to_block_index();

            // Read the block into `buf`.
            ramdisk::read_block(block_index, &mut buf).unwrap();

            // Read all the entries of that block.
            while read_items < max_items {
                let entry = DirEntry::from_bytes(&buf[offset..offset + mem::size_of::<DirEntry>()]);
                offset += mem::size_of::<DirEntry>();
                res.push(entry);
                read_items += 1;
            }
        }

        res
    }

    /// To create a new directory, we need to know about the parent directory.
    /// We can do this either via the INode, or the index of the INode.
    ///
    /// For now let's only allow complete paths inside of the root directory.
    ///
    /// Creating a new directory, we need to do a couple of things:
    /// 1. Create a new INode for that new directory.
    /// 2. Write a `DirEntry` to the parents data blocks.
    /// 3. Create the two directories '.' & '..' for the new directory and
    pub(crate) fn mkdir(name: String) {
        println!("===== Creating directory {name} =====");

        // Get the `INode` of the root directory.

        // Start traversing at root
        let mut current = fs::read_inode_inner(INodeIndex(0));
        let mut current_index = INodeIndex(0);
        let mut prev_index = INodeIndex(0);
        let mut prev = None;

        let mut peekable_iter = name.split('/').skip(1).peekable();

        // Iterate over all the nested directories skipping the first empty
        // entry.
        while let Some(entry) = peekable_iter.next() {
            println!("Looking for path: {entry:?} in {current_index:?}");
            let is_last = peekable_iter.peek().is_none();
            let entries = fs::read_dir_entry(&current);

            let cmp = |bytes: &[u8]| -> bool {
                let len = bytes.iter().take_while(|&&b| b != 0).count();
                let offset = if bytes[0] == b'/' { 1 } else { 0 };
                entry.as_bytes() == &bytes[offset..len]
            };

            if let Some(next_dir) = entries.iter().find(|e| cmp(&e.name[..])) {
                if is_last {
                    panic!("Duplicated directory: {name} with {next_dir:?}");
                } else {
                    println!("** Found match {next_dir:?}");
                    prev.replace(current);
                    prev_index = current_index;
                    current = fs::read_inode_inner(next_dir.inode);
                    current_index = next_dir.inode;
                    println!(
                        "Found matching subdirectory at {} = inode={:?}",
                        next_dir.name(),
                        next_dir.inode
                    );
                }
            } else {
                if is_last {
                    // Create the `INode` for the new empty directory.
                    let mut new_inode = INode::empty_directory();
                    // Write that `INode` to disk to get the index.
                    let inode_index = fs::write_inode(&new_inode);
                    // Create a `DirEntry` with `name` for the new directory and link it
                    // to root. TODO(mt): also should not be root here.
                    let new_directory = DirEntry::new(entry.to_string(), inode_index);
                    fs::write_dir_entry(new_directory, &mut current);
                    fs::update_inode(current_index, &current);

                    // Create the "." & ".." directories referencing the current & root
                    // directory.
                    let this = DirEntry::new(String::from("."), inode_index);
                    let parent = DirEntry::new(String::from(".."), prev_index);
                    fs::write_dir_entry(this, &mut new_inode);
                    fs::write_dir_entry(parent, &mut new_inode);
                    // Update the `INode` after adding the `DirEntries` to it.
                    fs::update_inode(inode_index, &new_inode);

                    println!("Created new directory {name} at inode {inode_index:?}");

                    break;
                } else {
                    panic!("Subdirectory doens't exist {name} entry={entry}");
                }
            }
        }

        println!("========================================");

        // Write the new directories `INode` into the data block of the root
        // directory
        // fs::write_dir_entry(new_directory, &mut root_directory_inode);
        // fs::update_inode(0, &root_directory_inode);
    }

    pub(crate) fn create_empty_root() {
        // Create the root INode
        let mut root_inode = INode::empty_directory();

        let root_inode_index = fs::write_inode(&root_inode);

        // Create the default directories in the root directory.
        let this = DirEntry::new(String::from("."), root_inode_index);
        let this_too = DirEntry::new(String::from(".."), root_inode_index);

        fs::write_dir_entry(this, &mut root_inode);
        fs::write_dir_entry(this_too, &mut root_inode);

        fs::update_inode(root_inode_index, &root_inode);

        logln!("[FS] Filesystem initialized with empty root directory");
    }
}

/// Terminology:
/// * INode      - is a block of metadata about a file - written to INode blocks
/// * DirEntry   - contains a name and the associated INode ID - is written to
///                Data block
/// * RawData    - contains raw file content - written to Data block
/// * Superblock - the first block in the filesystem containing metadata
///                about the state of the filesystem
pub struct Filesystem {
    inode_bitmap: Bitmap<INODE_BITMAP_SIZE>,
    data_bitmap: Bitmap<DATA_BITMAP_SIZE>,
    // TODO(mt): add caches for INodes etc here, then write them periodically and before shutdown - same as superblock.
}

impl Filesystem {
    const fn new() -> Self {
        Self {
            inode_bitmap: Bitmap::<INODE_BITMAP_SIZE>::new(),
            data_bitmap: Bitmap::<DATA_BITMAP_SIZE>::new(),
        }
    }

    /// This handle the initialization of the Filesystem based on the passed
    /// `SuperBlock`.
    ///
    /// If the superblock is valid, i.e the magic is correct, then the
    /// `FileSystem` will build it's state from it.
    ///
    /// If not, then the `Filesystem` will initialize to an empty default,
    /// creating the root directory and nothing else.
    fn init_from_superblock(_superblock: SuperBlock) -> Self {
        Self::new()
    }
}

pub fn dump() {
    let mut buf = [0u8; BLOCK_SIZE];
    let total = ramdisk::total_blocks() as u32;

    println!(
        "=== RAMDISK DUMP ({} blocks, {} bytes total) ===",
        total,
        total * BLOCK_SIZE as u32
    );

    for block_idx in 0u32..total {
        if ramdisk::read_block(BlockIndex(block_idx), &mut buf).is_err() {
            continue;
        }

        // Skip empty blocks
        if buf.iter().all(|&b| b == 0) {
            continue;
        }

        println!(
            "\n--- Block {block_idx} (offset 0x{:06x}) ---",
            block_idx * BLOCK_SIZE as u32
        );

        for row in 0..(BLOCK_SIZE / 16) {
            let offset = row * 16;
            let addr = block_idx * BLOCK_SIZE as u32 + offset as u32;

            // Print address
            print!("{:06x}  ", addr);

            // Print hex bytes
            for i in 0..16 {
                print!("{:02X} ", buf[offset + i]);
                if i == 7 {
                    print!(" ");
                }
            }

            // Print ASCII representation
            print!(" |");
            for i in 0..16 {
                let b = buf[offset + i];
                if (0x20..0x7f).contains(&b) {
                    print!("{}", b as char);
                } else {
                    print!(".");
                }
            }
            println!("|");
        }
    }

    println!("\n=== END DUMP ===");
}

pub fn dump_dir(index: u32) {
    let inode_index = INodeIndex(index);
    let mut buf = [0u8; BLOCK_SIZE];

    let inode = fs::read_inode_inner(inode_index);
    assert!(inode.is_directory);

    for &block in inode.blocks.iter().filter(|&&b| b.0 > 0) {
        ramdisk::read_block(block.to_block_index(), &mut buf).unwrap();

        for i in 0..DIR_ENTRY_PER_BLOCK {
            let entry = DirEntry::from_bytes(&buf[i * mem::size_of::<DirEntry>()..]);

            // only print the directories that have a name
            if entry.name.iter().any(|c| *c != 0) {
                println!("\t{entry:?}");
            }
        }
    }

}

pub fn mkdir(name: String) {
    fs::mkdir(name);
}

/// Initializes the Filesystem by reading the superblock or defaulting it
/// if it doesn't exist.
pub fn init() {
    let superblock = fs::read_superblock();
    let filesystem = Filesystem::init_from_superblock(superblock);

    (*FS.lock()).init(filesystem);
    fs::create_empty_root();

    dump_dir(4)
}

/*
TODO(mt)
    * load from disk on startup
    * cache things in memory and write to disk periodically
    * catch a shutdown and flush to disk
*/
