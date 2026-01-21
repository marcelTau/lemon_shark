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
use alloc::string::String;
use core::{ mem, };
use crate::{logln, print, println};

struct ByteReader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> ByteReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 } 
    }

    fn read_u32(&mut self) -> u32 {
        let value = u32::from_le_bytes(self.bytes[self.pos..self.pos+4].try_into().unwrap());
        self.pos += 4;
        value
    }

    fn read_u64(&mut self) -> u64 {
        let value = u64::from_le_bytes(self.bytes[self.pos..self.pos+8].try_into().unwrap());
        self.pos += 8;
        value
    }

    fn read_bytes(&mut self, len: usize) -> &'a [u8] {
        let slice = &self.bytes[self.pos..self.pos+len];
        self.pos += len;
        slice
    }
}

// Layout of blocks:
//
// 0. Superblock
// 1..=11. inode
// 12-. data blocks

const BLOCK_SIZE: usize = 512;
const INODE_BLOCKS: usize = 10;
const INODE_START: usize = 1;
const INODES_PER_BLOCK: usize = BLOCK_SIZE / core::mem::size_of::<INode>();
const DIR_ENTRY_PER_BLOCK: usize = BLOCK_SIZE / core::mem::size_of::<DirEntry>();
const DATA_START: usize = INODE_START + INODE_BLOCKS + 1;
const INODE_BITMAP_SIZE: usize = INODE_BLOCKS * INODES_PER_BLOCK / 32;
const DATA_BITMAP_SIZE: usize = (ramdisk::total_blocks() - 1 /*superblock*/ - INODE_BLOCKS) / 32;
const MAGIC: u64 = 0x4e4f4d454c; // lemon (le)
const SUPERBLOCK_SIZE: usize = core::mem::size_of::<SuperBlock>();

mod ramdisk {
    use crate::filesystem::{BLOCK_SIZE, BlockIndex};

    const RAMDISK_SIZE: usize = 1024 * 1024;
    static mut RAMDISK: [u8; RAMDISK_SIZE] = [0; RAMDISK_SIZE];

    pub(crate) const fn total_blocks() -> usize {
        RAMDISK_SIZE / BLOCK_SIZE
    }

    /// Read block `block_num` into `buf`.
    pub(crate) fn read_block(block_idx: BlockIndex, buf: &mut [u8]) -> Result<(), &'static str> {
        if buf.len() != BLOCK_SIZE {
            return Err("Buffer must be BLOCK_SIZE bytes");
        }

        let start = block_idx.0 * BLOCK_SIZE;

        if start + BLOCK_SIZE >= RAMDISK_SIZE {
            return Err("Block number out of range");
        }

        unsafe {
            buf.copy_from_slice(&RAMDISK[start..start + BLOCK_SIZE]);
        }

        Ok(())
    }

    pub(crate) fn write_block(block_idx: BlockIndex, data: &[u8]) -> Result<(), &'static str> {
        if data.len() != BLOCK_SIZE {
            return Err("Data must be BLOCK_SIZE bytes");
        }

        let start = block_idx.0 * BLOCK_SIZE;

        if start + BLOCK_SIZE >= RAMDISK_SIZE {
            return Err("Block number out of range");
        }

        unsafe {
            RAMDISK[start..start + BLOCK_SIZE].copy_from_slice(data);
        }

        Ok(())
    }
}

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
    /// Used blocks of this file.
    blocks: [u32; 16],
    is_directory: bool,
}

impl INode {
    fn empty_directory() -> Self {
        INode {
            size: 0,
            is_directory: true,
            blocks: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        }
    }

    fn from_bytes(bytes: &[u8]) -> Self {
        let size = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let mut blocks: [u32; 16] = [0; 16];
        let mut i = 4;

        (0..16).for_each(|idx| {
            blocks[idx] = u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
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
            bytes[start..start + 4].copy_from_slice(&self.blocks[i].to_le_bytes());
        }
        bytes[68] = if self.is_directory { 1 } else { 0 };

        bytes
    }
}

/// The `DirEntry` contains metadata about a directory.
#[repr(C)]
#[derive(Debug)]
struct DirEntry {
    /// Name of the directory
    name: [u8; 24],
    /// INode index of this directory
    inode: u32,
}

impl DirEntry {
    fn new(name_string: String, inode: u32) -> Self {
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
        let inode = reader.read_u32();
        Self { name, inode }
    }

    fn to_bytes(&self) -> [u8; mem::size_of::<DirEntry>()] {
        let mut bytes = [0u8; mem::size_of::<DirEntry>()];

        bytes[0..24].copy_from_slice(self.name.as_slice());
        bytes[24..28].copy_from_slice(&self.inode.to_le_bytes());

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

        let inode_bitmap = Bitmap::<INODE_BITMAP_SIZE>::from_bytes(reader.read_bytes(INODE_BITMAP_SIZE * 4));
        let data_bitmap = Bitmap::<DATA_BITMAP_SIZE>::from_bytes(reader.read_bytes(DATA_BITMAP_SIZE * 4));

        Self { magic, block_size, total_blocks, inode_table_start, inode_table_blocks, data_start, inode_bitmap, data_bitmap }
    }

     fn to_bytes(&self) -> [u8; SUPERBLOCK_SIZE] {
         let mut bytes = [0u8; SUPERBLOCK_SIZE];

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

enum Offset {
    /// INode offset, *not block offset*.
    INode(usize),
    SuperBlock,
    Data(usize),
    DirEntry(usize),
    Block(usize),
}

#[derive(Copy, Clone, Debug)]
struct BlockIndex(usize);

impl BlockIndex {
    fn from_inode_index(inode_index: usize) -> Self {
        let block_index = INODE_START + (inode_index / INODES_PER_BLOCK);
        if block_index > INODE_START + INODE_BLOCKS {
            panic!("No more inode blocks");
        }
        Self(block_index)
    }

    fn from_dir_entry_index(dir_entry_index: usize) -> Self {
        let block_index = DATA_START + (dir_entry_index / DIR_ENTRY_PER_BLOCK);
        if block_index > ramdisk::total_blocks() {
            panic!("No free block to store DirEntry");
        }
        Self(block_index)
    }

    fn superblock() -> Self {
        Self(0)
    }
}

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
        unsafe {
            core::slice::from_raw_parts(self.arr.as_ptr() as *const u8, WORDS * 4)
        }
    }

    fn set(&mut self, index: usize) {
        assert!(index < WORDS * 32);

        let arr_index = index / 32;
        let bit_index = index % 32;

        self.arr[arr_index] |= 1 << bit_index;
    }

    fn unset(&mut self, index: usize) {
        assert!(index < WORDS * 32);

        let arr_index = index / 32;
        let bit_index = index % 32;

        self.arr[arr_index] &= !(1 << bit_index);
    }

    fn is_set(&self, index: usize) -> bool {
        assert!(index < WORDS * 32);

        let arr_index = index / 32;
        let bit_index = index % 32;

        self.arr[arr_index] & (1 << bit_index) > 0
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
        buf[0..mem::size_of::<SuperBlock>()].copy_from_slice(superblock.to_bytes().as_slice());
        ramdisk::write_block(BlockIndex::superblock(), &buf).unwrap();
    }

    /// Reads the superblock from block_index 0
    pub fn read_superblock() -> SuperBlock {
        let mut buf = [0u8; BLOCK_SIZE];
        ramdisk::read_block(BlockIndex::superblock(), &mut buf);
        let superblock = SuperBlock::from_bytes(&buf[0..mem::size_of::<SuperBlock>()]);

        if superblock.magic == MAGIC {
            superblock
        } else {
            SuperBlock::default_superblock()
        }
    }

    /// Writes an `INode` into the block at the specific offset into the inode
    /// blocks.
    fn write_inode_inner(offset: usize, inode: &INode) {
        // Creating the buffer to write the INode to.
        let mut buf = [0u8; BLOCK_SIZE];

        // Calculate the `BlockIndex` for the `INode` to be written to.
        let block_index = BlockIndex::from_inode_index(offset);

        // Reading the block into `buf` to append the `INode` to it.
        ramdisk::read_block(block_index, &mut buf).unwrap();

        // Calculating the byte_offset inside of the block.
        let byte_offset = (offset % INODES_PER_BLOCK) * mem::size_of::<INode>();
        logln!("[FS] Writing to block index {block_index:?} at byte_offset={byte_offset}");

        // Writing the `INode` to the block.
        buf[byte_offset..byte_offset + mem::size_of::<INode>()]
            .copy_from_slice(inode.to_bytes().as_slice());

        // Writing the block to memory.
        ramdisk::write_block(block_index, &buf);
    }

    /// Writes a new `INode` to the ramdisk.
    pub(crate) fn write_inode(inode: &INode) -> u32 {
        // Finds the next free block in the `INodeBitmap`.
        let free_block = (*FS.lock()).inner().inode_bitmap.find_free().unwrap();
        logln!("[FS] Writing INode to next_free_block={free_block}");

        // Set this block to be used.
        (*FS.lock()).inner().inode_bitmap.set(free_block as usize);

        // Write the `INode` to the block
        write_inode_inner(free_block as usize, inode);

        free_block
    }

    /// Read the `INode` at offset
    pub(crate) fn read_inode_inner(offset: Offset) -> INode {
        let mut buf = [0u8; BLOCK_SIZE];
        let Offset::INode(inode_offset) = offset else {
            panic!("Received wrong offset type");
        };

        let block_index = BlockIndex::from_inode_index(inode_offset);
        let byte_offset = (inode_offset % INODES_PER_BLOCK) * mem::size_of::<INode>();

        ramdisk::read_block(block_index, &mut buf).unwrap();
        INode::from_bytes(&buf[byte_offset..byte_offset + mem::size_of::<INode>()])
    }

    pub(crate) fn update_inode(inode_index: usize, inode: &INode) {
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
        let block_index = current_entries / DIR_ENTRY_PER_BLOCK;

        logln!("[FS] Writing DirEntry {entry:?}");

        // If the block at `block_index` is 0, this means we need to allocate
        // a new block here.
        if inode.blocks[block_index] == 0 {
            if block_index > 16 {
                panic!("Can not hold more than 16 blocks of DirEntries");
            }

            // Find the next free block in the data bitmap.
            let free = (*FS.lock()).inner().data_bitmap.find_free().unwrap();

            let offset = BlockIndex::from_dir_entry_index(free as usize);
            let free = offset.0 as u32;

            // Assign this block to this `INode`.
            inode.blocks[block_index] = free;

            // Mark this block as used.
            (*FS.lock()).inner().data_bitmap.set(free as usize);
        }

        // Get the offset inside of this block
        let offset_in_block = (current_entries % DIR_ENTRY_PER_BLOCK) * mem::size_of::<DirEntry>();

        // Get the block_num in the data blocks.
        let block_num = inode.blocks[block_index] as usize;

        // Read this block into `buf`.
        ramdisk::read_block(BlockIndex(block_num), &mut buf).unwrap();

        // Write `DirEntry` into the `buf`.
        buf[offset_in_block..offset_in_block + mem::size_of::<DirEntry>()]
            .copy_from_slice(entry.to_bytes().as_slice());

        // Write `buf` to memory.
        ramdisk::write_block(BlockIndex(block_num), &buf);

        // Increment the `size` by the size of the `DirEntry`.
        inode.size += mem::size_of::<DirEntry>() as u32;
    }

    pub(crate) fn read_dir_entry() {}

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
        // Create the `INode` for the new empty directory.
        let mut new_inode = INode::empty_directory();

        // Write that `INode` to disk to get the index.
        let inode_index = fs::write_inode(&new_inode);

        // Create the "." & ".." directories referencing the current & root
        // directory.
        let this = DirEntry::new(String::from("."), inode_index);
        let parent = DirEntry::new(String::from(".."), 0);

        // Write those `DirEntries` to disk into the blocks of the `INode`.
        fs::write_dir_entry(this, &mut new_inode);
        fs::write_dir_entry(parent, &mut new_inode);

        // Update the `INode` after adding the `DirEntries` to it.
        fs::update_inode(inode_index as usize, &new_inode);

        // Create a `DirEntry` with `name` for the new directory and link it
        // to root.
        let new_directory = DirEntry::new(name, 0);

        // TODO(mt): here we should parse the whole path and be able to create subdirectories.
        // For now we can just make this work from the root and if we want to create a subdir it's
        // a/b etc

        // Get the `INode` of the root directory.
        let mut root_directory_inode = fs::read_inode_inner(Offset::INode(0));

        // Write the new directories `INode` into the data block of the root
        // directory
        fs::write_dir_entry(new_directory, &mut root_directory_inode);
        fs::update_inode(0, &root_directory_inode);
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

        fs::update_inode(root_inode_index as usize, &root_inode);

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
    fn init_from_superblock(superblock: SuperBlock) -> Self {
        Self::new()
    }
}

pub fn dump() {
    let mut buf = [0u8; BLOCK_SIZE];
    let total = ramdisk::total_blocks();

    println!(
        "=== RAMDISK DUMP ({} blocks, {} bytes total) ===",
        total,
        total * BLOCK_SIZE
    );

    for block_idx in 0..total {
        if ramdisk::read_block(BlockIndex(block_idx), &mut buf).is_err() {
            continue;
        }

        // Skip empty blocks
        if buf.iter().all(|&b| b == 0) {
            continue;
        }

        println!(
            "\n--- Block {} (offset 0x{:06x}) ---",
            block_idx,
            block_idx * BLOCK_SIZE
        );

        for row in 0..(BLOCK_SIZE / 16) {
            let offset = row * 16;
            let addr = block_idx * BLOCK_SIZE + offset;

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

pub fn dump_directories() {
    let mut buf = [0u8; BLOCK_SIZE];
    let root_inode = fs::read_inode_inner(Offset::INode(0));

    println!("=== Directories ===");
    println!("/");

    for &block in root_inode.blocks.iter().filter(|b| b > &&0) {
        ramdisk::read_block(BlockIndex::from_dir_entry_index(block as usize), &mut buf);

        for i in 0..DIR_ENTRY_PER_BLOCK {
            let entry = DirEntry::from_bytes(&buf[i * mem::size_of::<DirEntry>()..]);
            if entry.name[0] != 0 {
                let name = String::from_utf8(entry.name.to_vec()).unwrap();
                println!("{name}");
            }
        }
    }

    println!("=== End of Directories ===");
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

    fs::mkdir(String::from("hallo-test"));

    dump();
}

/*
TODO(mt)
    * load from disk on startup
    * cache things in memory and write to disk periodically
    * catch a shutdown and flush to disk
*/
