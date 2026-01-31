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
use crate::bitmap::Bitmap;
use crate::bytereader::ByteReader;
use crate::{logln, print, println, ramdisk};
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::cell::UnsafeCell;
use core::fmt::Debug;
use core::mem;
use core::sync::atomic::{AtomicBool, Ordering};

pub(crate) use types::BlockIndex;

#[derive(Debug, PartialEq)]
pub enum Error {
    DuplicatedEntry,
    DirectoryDoesNotExist,
    NoSpaceForDirEntry,
    NotAFile,
    NoSpaceInFile,
    NotADirectory,
}

impl core::error::Error for Error {}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}

pub const BLOCK_SIZE: usize = 512;
const INODE_BLOCKS: usize = 10;
const INODE_START: usize = 1;
const INODES_PER_BLOCK: usize = BLOCK_SIZE / core::mem::size_of::<INode>();
pub const DIR_ENTRY_PER_BLOCK: usize = BLOCK_SIZE / core::mem::size_of::<DirEntry>();
const DATA_START: usize = INODE_START + INODE_BLOCKS + 1;
const INODE_BITMAP_SIZE: usize = (INODE_BLOCKS * INODES_PER_BLOCK) / 32;
const DATA_BITMAP_SIZE: usize = (ramdisk::total_blocks() - 1 /*superblock*/ - INODE_BLOCKS) / 32;
const MAGIC: u64 = 0x4e4f4d454c; // lemon (le)

mod types {
    use core::mem;
    use core::num::NonZeroU32;

    use super::{DATA_START, INODE_START, INODES_PER_BLOCK, INode};

    /// An Index into the blocks used for the `ramdisk`.
    #[derive(Debug, Clone, Copy)]
    pub(crate) struct BlockIndex(pub u32);

    /// A `ByteOffset` to something inside of a block.
    #[derive(Debug)]
    pub(crate) struct ByteOffset(u32);

    impl ByteOffset {
        pub(crate) fn range<T>(&self) -> core::ops::Range<usize> {
            self.0 as usize..self.0 as usize + mem::size_of::<T>()
        }
    }

    /// This is the actual index of the INode.
    #[derive(Debug, Copy, Clone)]
    pub struct INodeIndex(pub u32);

    impl INodeIndex {
        /// Returns the `BlockIndex` and the offset inside of the block for that
        /// `INode`.
        pub(crate) fn to_block_index(self) -> (BlockIndex, ByteOffset) {
            let block_index = BlockIndex(INODE_START as u32 + (self.0 / INODES_PER_BLOCK as u32));
            let offset =
                ByteOffset((self.0 % INODES_PER_BLOCK as u32) * mem::size_of::<INode>() as u32);
            (block_index, offset)
        }
    }

    // pub(crate) struct DirEntryIndex(u32);
    // impl DirEntryIndex {
    //     pub(crate) fn to_block_index(&self) -> (BlockIndex, ByteOffset) {
    //         let block_index = BlockIndex(DATA_START as u32 + (self.0 / DIR_ENTRY_PER_BLOCK as u32));
    //         let offset = ByteOffset(
    //             (self.0 % DIR_ENTRY_PER_BLOCK as u32) * mem::size_of::<DirEntry>() as u32,
    //         );
    //         (block_index, offset)
    //     }
    // }

    /// `DataBlockIndex` is an index into the blocks of the ramdisk but is restricted to
    /// indexes into the data segment. This is used to enforce this invariant in the
    /// typesystem.
    #[derive(Clone, Copy, Debug, Default)]
    #[repr(transparent)]
    pub(crate) struct DataBlockIndex(Option<NonZeroU32>);
    impl DataBlockIndex {
        /// Creates a new `DataBlockIndex` from a raw `DataBlockIndex` when read from disk
        /// and should not be used otherwise.
        pub(crate) fn from_raw(val: u32) -> Self {
            if val != 0 && val < DATA_START as u32 {
                panic!("Invalid DataBlockIndex {val}. Must be >= {DATA_START}");
            }

            Self(NonZeroU32::new(val))
        }

        /// Creates a new `DataBlockIndex` from an index into the data segment.
        pub(crate) fn from_index(val: u32) -> Self {
            Self(NonZeroU32::new(DATA_START as u32 + val))
        }

        pub(crate) fn to_block_index(self) -> BlockIndex {
            BlockIndex(self.0.unwrap().get())
        }

        pub(crate) fn value(&self) -> Option<u32> {
            self.0.map(|v| v.get())
        }

        pub(crate) fn is_none(&self) -> bool {
            self.0.is_none()
        }
    }
}

use types::{DataBlockIndex, INodeIndex};

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
#[derive(Debug, Copy, Clone)]
pub struct INode {
    /// Size of the data in `.blocks`.
    size: u32,

    /// Used blocks of this `INode`.
    blocks: [DataBlockIndex; 16],

    /// Flag indicating if this is a directory.
    is_directory: bool,
}

impl INode {
    fn empty_directory() -> Self {
        INode {
            size: 0,
            is_directory: true,
            blocks: core::array::from_fn(|_| Default::default()),
        }
    }

    fn empty_file() -> Self {
        INode {
            size: 0,
            is_directory: false,
            blocks: core::array::from_fn(|_| Default::default()),
        }
    }

    fn from_bytes(bytes: &[u8]) -> Self {
        let size = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let mut blocks: [DataBlockIndex; 16] = [Default::default(); 16];
        let mut i = 4;

        (0..16).for_each(|idx| {
            let value = u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
            blocks[idx] = DataBlockIndex::from_raw(value);
            i += 4;
        });

        let is_directory = bytes[68] != 0;

        Self {
            size,
            is_directory,
            blocks,
        }
    }

    fn to_bytes(self) -> [u8; mem::size_of::<INode>()] {
        let mut bytes = [0u8; mem::size_of::<INode>()];

        bytes[0..4].copy_from_slice(&self.size.to_le_bytes());
        let current_offset = 4;
        for i in 0..16 {
            let start = current_offset + (i * 4);
            let value = self.blocks[i].value().unwrap_or_default();
            bytes[start..start + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes[68] = if self.is_directory { 1 } else { 0 };

        bytes
    }
}

/// The `DirEntry` contains metadata about an entry in a directory such as a
/// file or another directory which is pointed to by the `INodeIndex`.
#[repr(C)]
pub struct DirEntry {
    /// Name of the directory
    name: [u8; 24],

    /// INode index of this directory
    inode: INodeIndex,
}

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

#[derive(PartialEq)]
enum Entry {
    File,
    Directory,
}

/// The `Superblock` contains counts & pointers to strucutres used and metadata
/// about the state of the allocator.
#[repr(C)]
#[derive(Debug, PartialEq)]
pub struct SuperBlock {
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

static FS: spin::Mutex<LockedFilesystem> = spin::Mutex::new(LockedFilesystem::new());
static FS_INITIALIZED: AtomicBool = AtomicBool::new(false);

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

    pub fn mkdir(&mut self, path: &str) -> Result<INodeIndex, Error> {
        self.inner
            .get_mut()
            .as_mut()
            .unwrap()
            .new_dir_entry(path, Entry::Directory)
    }

    pub fn create_file(&mut self, path: &str) -> Result<INodeIndex, Error> {
        self.inner
            .get_mut()
            .as_mut()
            .unwrap()
            .new_dir_entry(path, Entry::File)
    }

    fn dump_dir(&mut self, index: u32) {
        self.inner.get_mut().as_mut().unwrap().dump_dir(index)
    }

    fn create_empty_root(&mut self) {
        self.inner.get_mut().as_mut().unwrap().create_empty_root();
    }

    fn write_to_file(&mut self, inode_index: INodeIndex, bytes: &[u8]) -> Result<usize, Error> {
        self.inner
            .get_mut()
            .as_mut()
            .unwrap()
            .append_to_file(inode_index, bytes)
    }

    pub(crate) fn read_file(&mut self, inode_index: INodeIndex) -> String {
        self.inner
            .get_mut()
            .as_mut()
            .unwrap()
            .read_file(inode_index)
    }

    fn reset(&mut self) {
        self.inner.get_mut().as_mut().unwrap().reset();
    }
}

/// Terminology:
/// * INode      - is a block of metadata about a file - written to INode blocks
/// * DirEntry   - contains a name and the associated INode ID - is written to
///   Data block
/// * RawData    - contains raw file content - written to Data block
/// * Superblock - the first block in the filesystem containing metadata
///   about the state of the filesystem
pub struct Filesystem {
    inode_bitmap: Bitmap<INODE_BITMAP_SIZE>,
    data_bitmap: Bitmap<DATA_BITMAP_SIZE>,
    inode_cache: INodeCache,
}

struct INodeCache {
    /// A `vec` holding all used `INodes` mapped by their `INodeIndex`. If an
    /// `INode` has not been read yet, the value in it's spot is `None`.
    inodes: Vec<Option<INode>>,
    // TODO(mt): store dirty nodes to know what to flush periodically.
}

impl INodeCache {
    pub const fn new() -> Self {
        Self { inodes: Vec::new() }
    }

    /// Reads the `INode` from disk if it's not already in the cache.
    fn read_from_disk(index: INodeIndex) -> INode {
        let mut buf = [0u8; BLOCK_SIZE];
        let (block_index, byte_offset) = index.to_block_index();
        ramdisk::read_block(block_index, &mut buf);
        INode::from_bytes(&buf[byte_offset.range::<INode>()])
    }

    /// Get a `&INode` from the cache, fetching it from disk when not present.
    pub fn get(&mut self, index: INodeIndex) -> &INode {
        if self.inodes.len() <= index.0 as usize {
            self.inodes
                .resize_with(index.0 as usize + 1, Default::default);
        }

        if self.inodes[index.0 as usize].is_none() {
            self.inodes[index.0 as usize] = Some(Self::read_from_disk(index));
        }

        self.inodes.get(index.0 as usize).unwrap().as_ref().unwrap()
    }

    /// Get a `&mut INode` from the cache, fetching it from disk when not
    /// present.
    pub fn get_mut(&mut self, index: INodeIndex) -> &mut INode {
        if self.inodes.len() <= index.0 as usize {
            self.inodes
                .resize_with(index.0 as usize + 1, Default::default);
        }

        if self.inodes[index.0 as usize].is_none() {
            self.inodes[index.0 as usize] = Some(Self::read_from_disk(index));
        }

        self.inodes
            .get_mut(index.0 as usize)
            .unwrap()
            .as_mut()
            .unwrap()
    }

    pub fn register_new_inode(&mut self, index: INodeIndex, inode: INode) {
        if self.inodes.len() <= index.0 as usize {
            self.inodes
                .resize_with(index.0 as usize + 1, Default::default);
        }

        self.inodes[index.0 as usize] = Some(inode);
    }
}

impl Filesystem {
    const fn new() -> Self {
        Self {
            inode_bitmap: Bitmap::<INODE_BITMAP_SIZE>::new(),
            data_bitmap: Bitmap::<DATA_BITMAP_SIZE>::new(),
            inode_cache: INodeCache::new(),
        }
    }

    pub fn reset(&mut self) {
        *self = Filesystem::new();
        ramdisk::reset();

        self.create_empty_root();
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

    /// Writes an `INode` to `ramdisk`.
    fn write_inode_to_disk(&self, inode_index: INodeIndex, inode: &INode) {
        // Creating the buffer to write the INode to.
        let mut buf = [0u8; BLOCK_SIZE];

        // Calculate the `BlockIndex` and the `ByteOffset` for the `INode`
        // to be written to.
        let (block_index, byte_offset) = inode_index.to_block_index();

        // Reading the block into `buf` to append the `INode` to it.
        ramdisk::read_block(block_index, &mut buf);

        // Calculating the byte_offset inside of the block.
        logln!("[FS] Writing to block index {block_index:?} at byte_offset={byte_offset:?}");

        buf[byte_offset.range::<INode>()].copy_from_slice(inode.to_bytes().as_slice());

        // Writing the block to memory.
        ramdisk::write_block(block_index, &buf);
    }

    /// Writes a new `INode` to the ramdisk.
    fn new_inode(&mut self, inode: &INode) -> INodeIndex {
        // Finds the next free block in the `INodeBitmap`.
        let free = INodeIndex(self.inode_bitmap.find_free().unwrap());

        logln!("[FS] Writing INode to {free:?} in {:?}", self.inode_bitmap);

        // Set this block to be used.
        self.inode_bitmap.set(free.0);

        // Write the `INode` to the block.
        self.write_inode_to_disk(free, inode);

        // Make the cache aware of this `INode`.
        self.inode_cache.register_new_inode(free, *inode);

        free
    }

    fn byte_compare(s: &str, bytes: &[u8; 24]) -> bool {
        let len = bytes.iter().take_while(|&&b| b != 0).count();
        let offset = if bytes[0] == b'/' { 1 } else { 0 };
        s.as_bytes() == &bytes[offset..len]
    }

    fn new_dir_entry(&mut self, name: &str, entry_type: Entry) -> Result<INodeIndex, Error> {
        // Start traversing at root
        let mut current_index = INodeIndex(0);
        let mut prev_index = INodeIndex(0);

        let mut path: Vec<_> = name.split('/').skip(1).collect();

        // The new entry - last part of the path.
        let new_entry = path.pop().unwrap();

        // Walk the path until the target directory to add the `new_entry`.
        for dir in &path {
            // Check that this part of the path is valid.
            let dir_entries = self.read_dir_entry(current_index);
            let next = dir_entries
                .iter()
                .find(|e| Self::byte_compare(dir, &e.name))
                .ok_or(Error::DirectoryDoesNotExist)?;

            prev_index = current_index;
            current_index = next.inode;
        }

        // Check that there is no entry with the same name.
        if self
            .read_dir_entry(current_index)
            .iter()
            .any(|e| Self::byte_compare(new_entry, &e.name))
        {
            return Err(Error::DuplicatedEntry);
        }

        // Create new `INode`.
        let new_inode = match entry_type {
            Entry::File => INode::empty_file(),
            Entry::Directory => INode::empty_directory(),
        };

        // Write that `INode` to disk to get the index.
        let inode_index = self.new_inode(&new_inode);

        // Create a `DirEntry` with `name` for the new directory and link it
        // to root.
        let new_directory = DirEntry::new(new_entry.to_string(), inode_index);

        self.write_dir_entry(new_directory, current_index).unwrap();

        // Create the "." & ".." directories for a new directory.
        if new_inode.is_directory {
            let this = DirEntry::new(String::from("."), inode_index);
            let parent = DirEntry::new(String::from(".."), prev_index);
            self.write_dir_entry(this, inode_index).unwrap();
            self.write_dir_entry(parent, inode_index).unwrap();
            println!("Created new directory {name} at inode {inode_index:?}");
        } else {
            println!("Created new file {name} at inode {inode_index:?}");
        }

        Ok(inode_index)
    }

    /// Writing a `DirEntry` needs to check if the `INode` already has a block
    /// which has some free space and we can write the new `DirEntry` to that
    /// block. If not then we need to allocate a new block and attach this to
    /// the `INode`.
    pub(crate) fn write_dir_entry(
        &mut self,
        entry: DirEntry,
        inode_index: INodeIndex,
    ) -> Result<(), Error> {
        let mut buf = [0u8; BLOCK_SIZE];

        let inode = self.inode_cache.get_mut(inode_index);

        // Calculate the currently used entries based on the size of the `INode`
        let current_entries = inode.size as usize / mem::size_of::<DirEntry>();

        // Get the index into the blocks of the `INode`
        let inode_internal_block_index = current_entries / DIR_ENTRY_PER_BLOCK;

        logln!("[FS] Writing DirEntry {entry:?} at block_index={inode_internal_block_index:?}");

        if inode_internal_block_index >= 16 {
            return Err(Error::NoSpaceForDirEntry);
        }

        if inode.blocks[inode_internal_block_index].is_none() {
            let free = self.data_bitmap.find_free().unwrap();
            let free_block_index = DataBlockIndex::from_index(free);
            inode.blocks[inode_internal_block_index] = free_block_index;
            self.data_bitmap.set(free);
        }

        let data_block_index = inode.blocks[inode_internal_block_index];

        // Get the offset inside of this block
        let offset_in_block = (current_entries % DIR_ENTRY_PER_BLOCK) * mem::size_of::<DirEntry>();

        let block_index = data_block_index.to_block_index();

        // Read this block into `buf`.
        ramdisk::read_block(block_index, &mut buf);

        // Write `DirEntry` into the `buf`.
        buf[offset_in_block..offset_in_block + mem::size_of::<DirEntry>()]
            .copy_from_slice(entry.to_bytes().as_slice());

        // Write `buf` to memory.
        ramdisk::write_block(block_index, &buf);

        // Increment the `size` by the size of the `DirEntry`.
        inode.size += mem::size_of::<DirEntry>() as u32;

        Ok(())
    }

    /// Reads all the `DirEntry`s for that INode and returns them in a Vec.
    fn read_dir_entry(&mut self, inode_index: INodeIndex) -> Vec<DirEntry> {
        // Get the `INode`
        let inode = self.inode_cache.get(inode_index);

        // If the `INode` is empty there is nothing to do here.
        if inode.size == 0 {
            return Vec::new();
        }

        // Calculate the number of `DirEntry`s that this `INode` is holding.
        let max_items = inode.size as usize / mem::size_of::<DirEntry>();

        let mut res = Vec::with_capacity(max_items);
        let mut buf = [0u8; BLOCK_SIZE];

        // Loop and read all `DirEntry`s into `res`.
        for block_index in inode
            .blocks
            .iter()
            .filter(|b| !b.is_none())
            .map(|b| b.to_block_index())
        {
            ramdisk::read_block(block_index, &mut buf);

            let items_in_block = (max_items - res.len()).min(DIR_ENTRY_PER_BLOCK);

            for i in 0..items_in_block {
                let start = i * mem::size_of::<DirEntry>();
                let end = start + mem::size_of::<DirEntry>();

                let entry = DirEntry::from_bytes(&buf[start..end]);
                res.push(entry);
            }

            if res.len() == max_items {
                break;
            }
        }

        res
    }

    pub(crate) fn create_empty_root(&mut self) {
        // Create the root INode.
        let root_inode = INode::empty_directory();

        // Write the node to disk to get the `INodeIndex`.
        let root_inode_index = self.new_inode(&root_inode);

        // Create the default directories in the root directory.
        let this = DirEntry::new(String::from("."), root_inode_index);
        let this_too = DirEntry::new(String::from(".."), root_inode_index);

        self.write_dir_entry(this, root_inode_index).unwrap();
        self.write_dir_entry(this_too, root_inode_index).unwrap();

        logln!("[FS] Filesystem initialized with empty root directory");
    }

    fn append_to_file(&mut self, inode_index: INodeIndex, bytes: &[u8]) -> Result<usize, Error> {
        let inode = self.inode_cache.get_mut(inode_index);

        if inode.is_directory {
            return Err(Error::NotAFile);
        }

        if inode.size as usize + bytes.len() > 16 * BLOCK_SIZE {
            return Err(Error::NoSpaceInFile);
        }

        let mut buf = [0u8; BLOCK_SIZE];
        let mut total_bytes = bytes.len();
        let mut bytes_written = 0;

        while total_bytes > 0 {
            let last_used_block_index = inode.size as usize / BLOCK_SIZE;

            if inode.blocks[last_used_block_index].is_none() {
                let free = self.data_bitmap.find_free().unwrap();
                let free_block_index = DataBlockIndex::from_index(free);
                inode.blocks[last_used_block_index] = free_block_index;
                self.data_bitmap.set(free);
            }

            let data_block_index = inode.blocks[last_used_block_index];
            let byte_offset = inode.size % BLOCK_SIZE as u32;

            let bytes_to_write = total_bytes.min(BLOCK_SIZE - byte_offset as usize);
            logln!("[FS] Writing {}/{} bytes", bytes_to_write, total_bytes);

            let block_index = data_block_index.to_block_index();

            ramdisk::read_block(block_index, &mut buf);

            buf[byte_offset as usize..byte_offset as usize + bytes_to_write]
                .copy_from_slice(&bytes[bytes_written..bytes_written + bytes_to_write]);

            ramdisk::write_block(block_index, &buf);
            inode.size += bytes_to_write as u32;

            total_bytes -= bytes_to_write;
            bytes_written += bytes_to_write;
        }

        Ok(bytes_written)
    }

    pub(crate) fn read_file(&mut self, inode_index: INodeIndex) -> String {
        let inode = self.inode_cache.get_mut(inode_index);

        if inode.is_directory {
            panic!("Can't write to directory");
        }

        let mut buf = [0u8; BLOCK_SIZE];

        let mut total_bytes = inode.size as usize;
        let mut string = String::with_capacity(total_bytes);

        for block in inode.blocks.iter().filter(|b| !b.is_none()) {
            let b = block.to_block_index();
            ramdisk::read_block(b, &mut buf);

            let valid_bytes = total_bytes.min(BLOCK_SIZE);
            total_bytes -= valid_bytes;

            string.push_str(str::from_utf8(&buf[..valid_bytes]).unwrap());
        }

        string
    }

    /// Writes the superblock to block_index 0
    pub fn write_superblock(&self, superblock: &SuperBlock) {
        let mut buf = [0u8; BLOCK_SIZE];
        buf[0..mem::size_of::<SuperBlock>()].copy_from_slice(&superblock.to_bytes());
        ramdisk::write_block(BlockIndex(0), &buf);
    }

    /// Reads the superblock from block_index 0
    pub fn read_superblock() -> SuperBlock {
        let mut buf = [0u8; BLOCK_SIZE];
        ramdisk::read_block(BlockIndex(0), &mut buf);
        let superblock = SuperBlock::from_bytes(&buf[0..mem::size_of::<SuperBlock>()]);

        if superblock.magic == MAGIC {
            superblock
        } else {
            SuperBlock::default_superblock()
        }
    }

    fn dump_dir(&mut self, index: u32) {
        let inode_index = INodeIndex(index);
        let mut buf = [0u8; BLOCK_SIZE];

        let inode = self.inode_cache.get(inode_index);
        assert!(inode.is_directory);

        for &block in inode.blocks.iter().filter(|&&b| !b.is_none()) {
            ramdisk::read_block(block.to_block_index(), &mut buf);

            for i in 0..DIR_ENTRY_PER_BLOCK {
                let entry = DirEntry::from_bytes(&buf[i * mem::size_of::<DirEntry>()..]);

                // only print the directories that have a name
                if entry.name.iter().any(|c| *c != 0) {
                    println!("\t{entry:?}");
                }
            }
        }
    }
}

/// Memmory dump of all the blocks. Handy for debugging.
pub fn dump() {
    let mut buf = [0u8; BLOCK_SIZE];
    let total = ramdisk::total_blocks() as u32;

    println!(
        "=== RAMDISK DUMP ({} blocks, {} bytes total) ===",
        total,
        total * BLOCK_SIZE as u32
    );

    for block_idx in 0u32..total {
        ramdisk::read_block(BlockIndex(block_idx), &mut buf);

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

/// Those functions are wrappers around the `LockedFilesystem` for the shell
/// to do some filesystem operations.
///
/// This is the only place where the `.lock()` should be called to avoid
/// deadlocks.
pub mod api {
    use super::*;

    pub fn dump_dir(index: u32) {
        (*FS.lock()).dump_dir(index);
    }

    pub fn mkdir(name: &str) -> Result<INodeIndex, Error> {
        (*FS.lock()).mkdir(name)
    }

    pub fn create_file(name: &str) -> Result<INodeIndex, Error> {
        (*FS.lock()).create_file(name)
    }

    pub fn write_to_file(inode_index: usize, text: String) -> Result<usize, Error> {
        (*FS.lock()).write_to_file(INodeIndex(inode_index as u32), text.as_bytes())
    }

    pub fn read_file(inode_index: usize) -> String {
        (*FS.lock()).read_file(INodeIndex(inode_index as u32))
    }

    pub fn reset() {
        (*FS.lock()).reset();
    }
}

/// Initializes the Filesystem by reading the superblock or defaulting it
/// if it doesn't exist.
pub fn init() {
    // Guard against re-initializing the Filesystem by setting the atomic
    // flag.
    if FS_INITIALIZED
        .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
        .is_err()
    {
        logln!("[FS] Not initializing the Filesystem again!");
        return;
    }

    let superblock = Filesystem::read_superblock();
    let filesystem = Filesystem::init_from_superblock(superblock);

    (*FS.lock()).init(filesystem);
    (*FS.lock()).create_empty_root();

    logln!("[FS] Initialized");

    // dump();
}
