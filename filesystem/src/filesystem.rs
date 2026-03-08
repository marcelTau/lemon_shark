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
use crate::layout::{DataBlockIndex, Layout};
use crate::{BlockIndex, INodeIndex};
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::mem;

/// Number of `DirEntry` per block.
const DIR_ENTRY_PER_BLOCK: usize = BLOCK_SIZE / core::mem::size_of::<DirEntry>();

// ----------------------------------------------------------------------------
/// BlockSize of the Filesystem.
pub const BLOCK_SIZE: usize = 512;

/// Number of INodes per block.
pub(crate) const INODES_PER_BLOCK: usize = BLOCK_SIZE / core::mem::size_of::<INode>();

/// Max number of INodes supported by the Filesystem
pub(crate) const MAX_INODES: usize = 4096;

/// Magic value written to the start of the block device.
const MAGIC: u64 = 0x4e4f4d454c; // lemon (le)

/// The trait that any block device backend must implement.
pub trait BlockDevice {
    fn read_block(&mut self, block_idx: BlockIndex, buf: &mut [u8]);
    fn write_block(&mut self, block_idx: BlockIndex, data: &[u8]);
    fn total_blocks(&mut self) -> usize;
}

/// Filesystem Errors
#[derive(Debug, PartialEq)]
pub enum Error {
    DuplicatedEntry,
    DirectoryDoesNotExist,
    NotADirectory,
    NoSpaceForDirEntry,
    NotAFile,
    NoSpaceInFile,
    OutOfMemory,
    NoNameProvided,
    NameTooLong,
}

impl core::error::Error for Error {}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}

/// The `INode` contains metadata about a file.
///
/// Memory layout:
/// `size`          4 bytes
/// `blocks`        64 bytes
/// `is_directory`  1 byte
/// `padding`       3 bytes
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub(crate) struct INode {
    /// Size of the data in `.blocks`.
    size: u32,

    /// Used blocks of this `INode`.
    blocks: [DataBlockIndex; 16],

    /// Flag indicating if this is a directory.
    is_directory: bool,
}

impl INode {
    fn new_empty_directory() -> Self {
        INode {
            size: 0,
            is_directory: true,
            blocks: core::array::from_fn(|_| Default::default()),
        }
    }

    fn new_empty_file() -> Self {
        INode {
            size: 0,
            is_directory: false,
            blocks: core::array::from_fn(|_| Default::default()),
        }
    }

    fn has_space(&self) -> bool {
        self.blocks.iter().any(DataBlockIndex::is_none)
    }

    fn from_bytes(bytes: &[u8]) -> Self {
        let size = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let mut blocks: [DataBlockIndex; 16] = [Default::default(); 16];
        let mut i = 4;

        (0..16).for_each(|idx| {
            let value = u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
            blocks[idx] = DataBlockIndex::from_raw_unchecked(value);
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
struct DirEntry {
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
        let len = self.name.iter().take_while(|&&b| b != 0).count();
        String::from_utf8(self.name[..len].to_vec()).unwrap_or_default()
    }

    fn from_bytes(bytes: &[u8]) -> Self {
        let mut reader = ByteReader::new(bytes);
        let name = reader.read_bytes(24).try_into().unwrap();
        let inode = INodeIndex::new(reader.read_u32());
        Self { name, inode }
    }

    fn to_bytes(&self) -> [u8; mem::size_of::<DirEntry>()] {
        let mut bytes = [0u8; mem::size_of::<DirEntry>()];

        bytes[0..24].copy_from_slice(self.name.as_slice());
        bytes[24..28].copy_from_slice(&self.inode.inner().to_le_bytes());

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
pub(crate) struct SuperBlock {
    magic: u64,
    block_size: u32,
    total_blocks: u32,
}

impl SuperBlock {
    fn from_bytes(bytes: &[u8]) -> Self {
        let mut reader = ByteReader::new(bytes);

        let magic = reader.read_u64();
        let block_size = reader.read_u32();
        let total_blocks = reader.read_u32();

        Self {
            magic,
            block_size,
            total_blocks,
        }
    }

    fn to_bytes(&self) -> [u8; mem::size_of::<SuperBlock>()] {
        let mut bytes = [0u8; mem::size_of::<SuperBlock>()];

        bytes[0..8].copy_from_slice(&self.magic.to_le_bytes());
        bytes[8..12].copy_from_slice(&self.block_size.to_le_bytes());
        bytes[12..16].copy_from_slice(&self.total_blocks.to_le_bytes());

        bytes
    }
}

#[derive(Default)]
struct INodeCache {
    /// A `vec` holding all used `INodes` mapped by their `INodeIndex`. If an
    /// `INode` has not been read yet, the value in it's spot is `None`.
    inodes: Vec<Option<INode>>,
    dirty: Bitmap,
    layout: Option<Layout>,
}

impl INodeCache {
    pub fn new(layout: Layout) -> Self {
        let size = layout.inode_bitmap_blocks * BLOCK_SIZE;
        log::debug!("INodeCache size={size}");
        Self {
            inodes: Vec::new(),
            layout: Some(layout),
            dirty: Bitmap::new(size as u32),
        }
    }

    /// Reads the `INode` from disk if it's not already in the cache.
    fn read_from_disk<D: BlockDevice>(&self, index: INodeIndex, device: &mut D) -> INode {
        let mut buf = [0u8; BLOCK_SIZE];
        let (block_index, byte_offset) = self.layout.as_ref().unwrap().inode_to_block(index);
        device.read_block(block_index, &mut buf);
        INode::from_bytes(&buf[byte_offset.range::<INode>()])
    }

    /// Get a `&INode` from the cache, fetching it from disk when not present.
    pub fn get<D: BlockDevice>(&mut self, index: INodeIndex, device: &mut D) -> &INode {
        if self.inodes.len() <= index.inner() as usize {
            self.inodes
                .resize_with(index.inner() as usize + 1, Default::default);
        }

        if self.inodes[index.inner() as usize].is_none() {
            self.inodes[index.inner() as usize] = Some(self.read_from_disk(index, device));
        }

        self.inodes
            .get(index.inner() as usize)
            .unwrap()
            .as_ref()
            .unwrap()
    }

    /// Get a `&mut INode` from the cache, fetching it from disk when not
    /// present.
    pub fn get_mut<D: BlockDevice>(&mut self, index: INodeIndex, device: &mut D) -> &mut INode {
        if self.inodes.len() <= index.inner() as usize {
            self.inodes
                .resize_with(index.inner() as usize + 1, Default::default);
        }

        if self.inodes[index.inner() as usize].is_none() {
            self.inodes[index.inner() as usize] = Some(self.read_from_disk(index, device));
        }

        // When handing out a mutable reference, consider it dirty.
        self.dirty.set(index.inner());

        self.inodes
            .get_mut(index.inner() as usize)
            .unwrap()
            .as_mut()
            .unwrap()
    }

    pub fn register_new_inode(&mut self, index: INodeIndex, inode: INode) {
        if self.inodes.len() <= index.inner() as usize {
            self.inodes
                .resize_with(index.inner() as usize + 1, Default::default);
        }

        self.inodes[index.inner() as usize] = Some(inode);
    }

    pub fn drain(&mut self) -> impl Iterator<Item = (INodeIndex, INode)> {
        self.dirty
            .drain_set()
            .map(INodeIndex::new)
            .map(|idx| (idx, self.inodes[idx.inner() as usize].unwrap()))
    }
}

/// Terminology:
/// * INode      - is a block of metadata about a file - written to INode blocks
/// * DirEntry   - contains a name and the associated INode ID - is written to
///   Data block
/// * RawData    - contains raw file content - written to Data block
/// * Superblock - the first block in the filesystem containing metadata
///   about the state of the filesystem
pub struct Filesystem<D> {
    block_device: D,
    inode_bitmap: Bitmap,
    data_bitmap: Bitmap,
    inode_cache: INodeCache,
    layout: Layout,
}

impl<Dev: BlockDevice> Filesystem<Dev> {
    /// Reads the superblock from block_index 0
    fn read_superblock(block_device: &mut Dev) -> (SuperBlock, bool) {
        let mut buf = [0u8; BLOCK_SIZE];
        block_device.read_block(BlockIndex::from_raw(0), &mut buf);
        let sb = SuperBlock::from_bytes(&buf[0..mem::size_of::<SuperBlock>()]);

        match sb.magic {
            0 => {
                log::info!("empty disk, creating new superblock");
                let total_blocks = block_device.total_blocks() as u32;
                (
                    SuperBlock {
                        magic: MAGIC,
                        block_size: BLOCK_SIZE as u32,
                        total_blocks,
                    },
                    false,
                )
            }
            MAGIC => {
                log::info!("found superblock on disk: {sb:?}");
                (sb, true)
            }
            _ => panic!("Disk has wrong format"),
        }
    }

    pub fn new(mut block_device: Dev) -> Self {
        let (sb, is_initialized) = Self::read_superblock(&mut block_device);

        let layout = Layout::new(sb.total_blocks);

        log::info!("generated layout: {layout:?}");
        log::info!(
            "Inode bitmap size = {}",
            BLOCK_SIZE * layout.inode_bitmap_blocks
        );

        let mut inode_bitmap_raw = vec![0u8; BLOCK_SIZE * layout.inode_bitmap_blocks];

        for i in 0..layout.inode_bitmap_blocks {
            let start = i * BLOCK_SIZE;
            block_device.read_block(
                BlockIndex::from_raw((layout.inode_bitmap_start + i) as u32),
                &mut inode_bitmap_raw[start..start + BLOCK_SIZE],
            );
        }

        log::info!(
            "Data bitmap size = {}",
            BLOCK_SIZE * layout.data_bitmap_blocks
        );

        let mut data_bitmap_raw = vec![0u8; BLOCK_SIZE * layout.data_bitmap_blocks];

        for i in 0..layout.data_bitmap_blocks {
            let start = i * BLOCK_SIZE;
            block_device.read_block(
                BlockIndex::from_raw((layout.data_bitmap_start + i) as u32),
                &mut data_bitmap_raw[start..start + BLOCK_SIZE],
            );
        }

        let (inode_bitmap, data_bitmap) = if is_initialized {
            log::info!("Reading inode_bitmap");
            let inode_bitmap = Bitmap::from_bytes(&inode_bitmap_raw);
            log::info!("Reading data_bitmap");
            let data_bitmap = Bitmap::from_bytes(&data_bitmap_raw);

            (inode_bitmap, data_bitmap)
        } else {
            (
                Bitmap::new((BLOCK_SIZE * layout.inode_bitmap_blocks) as u32),
                Bitmap::new((BLOCK_SIZE * layout.data_bitmap_blocks) as u32),
            )
        };

        let mut fs = Self {
            inode_bitmap,
            data_bitmap,
            inode_cache: INodeCache::new(layout),
            block_device,
            layout,
        };

        if !is_initialized {
            fs.create_empty_root();
        } else {
            fs.validate_root_inode();
        }

        fs
    }

    /// Returns a mutable reference to the underlying block device.
    /// Useful for device-specific operations like debug dumps.
    pub fn block_device_mut(&mut self) -> &mut Dev {
        &mut self.block_device
    }

    fn validate_root_inode(&mut self) {
        let mut buf = [0u8; BLOCK_SIZE];

        let (index, _) = self.layout.inode_to_block(INodeIndex::new(0));
        self.block_device.read_block(index, &mut buf);

        let root_node = INode::from_bytes(&buf[0..mem::size_of::<INode>()]);

        if !root_node.is_directory {
            panic!("Root INode validation failed");
        }
    }

    /// Writes an `INode` to disk.
    fn write_inode_to_disk(&mut self, inode_index: INodeIndex, inode: &INode) {
        let mut buf = [0u8; BLOCK_SIZE];

        let (block_index, byte_offset) = self.layout.inode_to_block(inode_index);

        self.block_device.read_block(block_index, &mut buf);

        log::trace!("writing to block index {block_index:?} at byte_offset={byte_offset:?}");

        buf[byte_offset.range::<INode>()].copy_from_slice(inode.to_bytes().as_slice());

        self.block_device.write_block(block_index, &buf);
    }

    /// Writes a new `INode` to disk.
    fn new_inode(&mut self, inode: &INode) -> Option<INodeIndex> {
        let free = INodeIndex::new(self.inode_bitmap.find_free()?);

        log::trace!("writing inode to {free:?} in {:?}", self.inode_bitmap);

        self.inode_bitmap.set(free.inner());

        self.write_inode_to_disk(free, inode);

        self.inode_cache.register_new_inode(free, *inode);

        Some(free)
    }

    fn byte_compare(s: &str, bytes: &[u8; 24]) -> bool {
        let len = bytes.iter().take_while(|&&b| b != 0).count();
        let offset = if bytes[0] == b'/' { 1 } else { 0 };
        s.as_bytes() == &bytes[offset..len]
    }

    /// Adds a new `DirEntry` based on the input path.
    ///
    /// A `DirEntry` can point to either a file or a directory.
    ///
    /// When adding a directory - this should also set the default directories
    /// '.' and '..'. TODO(mt): this should not happen in here tho.
    fn new_dir_entry(&mut self, path: &str, entry_type: Entry) -> Result<INodeIndex, Error> {
        // Path is separated by '/'. Split to get the parts.
        let mut parts: Vec<_> = path.split('/').filter(|s| !s.is_empty()).collect();

        // Start traversing at root index.
        let mut current = INodeIndex::new(0);

        let new_entry_name = parts.pop().ok_or(Error::NoNameProvided)?;

        if new_entry_name.len() > 24 {
            return Err(Error::NameTooLong);
        }

        // Iterate over parts of the path to walk the filesystem.
        for part in parts {
            // Read all `DirEntry` from the current INode.
            let dir_entries = self.read_dir_entry(current);

            // Find the entry for `part`.
            let next = dir_entries
                .iter()
                .find(|e| Self::byte_compare(part, &e.name))
                .ok_or(Error::DirectoryDoesNotExist)?;

            // If the INode that matches the `part` name is not a directory
            // return an error as we can't go in there.
            if !self
                .inode_cache
                .get(next.inode, &mut self.block_device)
                .is_directory
            {
                return Err(Error::NotADirectory);
            }

            // Update the current index.
            current = next.inode;
        }

        let parent_inode = self.inode_cache.get(current, &mut self.block_device);

        if !parent_inode.has_space() {
            return Err(Error::NoSpaceForDirEntry);
        }

        if self
            .read_dir_entry(current)
            .iter()
            .any(|e| Self::byte_compare(new_entry_name, &e.name))
        {
            return Err(Error::DuplicatedEntry);
        }

        // Create new `INode`.
        let new_inode = match entry_type {
            Entry::File => INode::new_empty_file(),
            Entry::Directory => INode::new_empty_directory(),
        };

        // Write that `INode` to disk to get the index.
        let inode_index = self
            .new_inode(&new_inode)
            .ok_or(Error::NoSpaceForDirEntry)?;

        // Create a `DirEntry` with `name` for the new directory and link it
        // to root.
        let new_directory = DirEntry::new(new_entry_name.to_string(), inode_index);

        self.write_dir_entry(new_directory, current).unwrap();

        // Create the "." & ".." directories for a new directory.
        if entry_type == Entry::Directory {
            let this = DirEntry::new(String::from("."), inode_index);
            let parent = DirEntry::new(String::from(".."), current);
            self.write_dir_entry(this, inode_index).unwrap();
            self.write_dir_entry(parent, inode_index).unwrap();
        }

        Ok(inode_index)
    }

    /// Writing a `DirEntry` needs to check if the `INode` already has a block
    /// which has some free space and we can write the new `DirEntry` to that
    /// block. If not then we need to allocate a new block and attach this to
    /// the `INode`.
    fn write_dir_entry(&mut self, entry: DirEntry, inode_index: INodeIndex) -> Result<(), Error> {
        let mut buf = [0u8; BLOCK_SIZE];

        let inode = self
            .inode_cache
            .get_mut(inode_index, &mut self.block_device);

        if !inode.is_directory {
            return Err(Error::NotADirectory);
        }

        // Calculate the currently used entries based on the size of the `INode`
        let current_entries = inode.size as usize / mem::size_of::<DirEntry>();

        // Get the index into the blocks of the `INode`
        let inode_internal_block_index = current_entries / DIR_ENTRY_PER_BLOCK;

        log::trace!("writing dir entry {entry:?} at block_index={inode_internal_block_index:?}");

        if inode_internal_block_index >= 16 {
            return Err(Error::NoSpaceForDirEntry);
        }

        if inode.blocks[inode_internal_block_index].is_none() {
            let free = self.data_bitmap.find_free().unwrap();
            let free_block_index = self.layout.data_block(free);
            inode.blocks[inode_internal_block_index] = free_block_index;
            self.data_bitmap.set(free);
        }

        let data_block_index = inode.blocks[inode_internal_block_index];

        // Get the offset inside of this block
        let offset_in_block = (current_entries % DIR_ENTRY_PER_BLOCK) * mem::size_of::<DirEntry>();

        let block_index = self.layout.data_to_block(data_block_index);

        // Read this block into `buf`.
        self.block_device.read_block(block_index, &mut buf);

        // Write `DirEntry` into the `buf`.
        buf[offset_in_block..offset_in_block + mem::size_of::<DirEntry>()]
            .copy_from_slice(entry.to_bytes().as_slice());

        // Write `buf` to memory.
        self.block_device.write_block(block_index, &buf);

        // Increment the `size` by the size of the `DirEntry`.
        inode.size += mem::size_of::<DirEntry>() as u32;

        Ok(())
    }

    /// Reads all the `DirEntry`s for that INode and returns them in a Vec.
    fn read_dir_entry(&mut self, inode_index: INodeIndex) -> Vec<DirEntry> {
        // Get the `INode`
        let inode = self.inode_cache.get(inode_index, &mut self.block_device);

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
            .map(|b| self.layout.data_to_block(*b))
        {
            self.block_device.read_block(block_index, &mut buf);

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
        let root_inode = INode::new_empty_directory();

        // Write the node to disk to get the `INodeIndex`.
        let root_inode_index = self
            .new_inode(&root_inode)
            .expect("There is space when creating the root");

        // Create the default directories in the root directory.
        let this = DirEntry::new(String::from("."), root_inode_index);
        let this_too = DirEntry::new(String::from(".."), root_inode_index);

        self.write_dir_entry(this, root_inode_index).unwrap();
        self.write_dir_entry(this_too, root_inode_index).unwrap();

        log::info!("initialized with empty root directory");
    }

    fn append_to_file(&mut self, inode_index: INodeIndex, bytes: &[u8]) -> Result<usize, Error> {
        let inode = self
            .inode_cache
            .get_mut(inode_index, &mut self.block_device);

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
                let free = self.data_bitmap.find_free().ok_or(Error::OutOfMemory)?;
                let free_block_index = self.layout.data_block(free);
                inode.blocks[last_used_block_index] = free_block_index;
                self.data_bitmap.set(free);
            }

            let data_block_index = inode.blocks[last_used_block_index];
            let byte_offset = inode.size % BLOCK_SIZE as u32;

            let bytes_to_write = total_bytes.min(BLOCK_SIZE - byte_offset as usize);
            log::debug!("writing {}/{} bytes", bytes_to_write, total_bytes);

            let block_index = self.layout.data_to_block(data_block_index);

            self.block_device.read_block(block_index, &mut buf);

            buf[byte_offset as usize..byte_offset as usize + bytes_to_write]
                .copy_from_slice(&bytes[bytes_written..bytes_written + bytes_to_write]);

            self.block_device.write_block(block_index, &buf);
            inode.size += bytes_to_write as u32;

            total_bytes -= bytes_to_write;
            bytes_written += bytes_to_write;
        }

        Ok(bytes_written)
    }

    pub fn read_file(&mut self, inode_index: INodeIndex) -> String {
        let inode = self
            .inode_cache
            .get_mut(inode_index, &mut self.block_device);

        if inode.is_directory {
            panic!("Can't read from directory");
        }

        let mut buf = [0u8; BLOCK_SIZE];

        let mut total_bytes = inode.size as usize;
        let mut string = String::with_capacity(total_bytes);

        for block in inode.blocks.iter().filter(|b| !b.is_none()) {
            let b = self.layout.data_to_block(*block);
            self.block_device.read_block(b, &mut buf);

            let valid_bytes = total_bytes.min(BLOCK_SIZE);
            total_bytes -= valid_bytes;

            string.push_str(str::from_utf8(&buf[..valid_bytes]).unwrap());
        }

        string
    }

    /// Writes the superblock to block_index 0
    fn write_superblock(&mut self, superblock: &SuperBlock) {
        let mut buf = [0u8; BLOCK_SIZE];
        buf[0..mem::size_of::<SuperBlock>()].copy_from_slice(&superblock.to_bytes());
        self.block_device.write_block(BlockIndex::from_raw(0), &buf);
    }

    pub fn dump_dir(&mut self, index: u32, out: &mut impl core::fmt::Write) {
        let inode_index = INodeIndex::new(index);
        let mut buf = [0u8; BLOCK_SIZE];

        let inode = self.inode_cache.get(inode_index, &mut self.block_device);
        assert!(inode.is_directory);

        for &block in inode.blocks.iter().filter(|&&b| !b.is_none()) {
            self.block_device
                .read_block(self.layout.data_to_block(block), &mut buf);

            for i in 0..DIR_ENTRY_PER_BLOCK {
                let entry = DirEntry::from_bytes(&buf[i * mem::size_of::<DirEntry>()..]);

                // only print the directories that have a name
                if entry.name.iter().any(|c| *c != 0) {
                    let _ = writeln!(out, "\t{entry:?}");
                }
            }
        }
    }

    pub fn tree(&mut self, out: &mut impl core::fmt::Write) {
        fn inner(
            fs: &mut Filesystem<impl BlockDevice>,
            entry: &DirEntry,
            indent: u8,
            out: &mut impl core::fmt::Write,
        ) {
            let entries = fs.read_dir_entry(entry.inode);

            if !entries.iter().any(|e| {
                fs.inode_cache
                    .get(e.inode, &mut fs.block_device)
                    .is_directory
            }) {
                return;
            }

            let inode = fs.inode_cache.get(entry.inode, &mut fs.block_device);

            if inode.is_directory {
                let _ = writeln!(out, "{}{}", " ".repeat(indent as usize), entry.name());
            }

            for entry in entries.iter().filter(|e| !e.name().starts_with('.')) {
                inner(fs, entry, indent + 2, out);
                if !fs
                    .inode_cache
                    .get(entry.inode, &mut fs.block_device)
                    .is_directory
                {
                    let _ = writeln!(out, "{}{}", " ".repeat(indent as usize + 2), entry.name());
                }
            }
        }

        let root_entries = self.read_dir_entry(INodeIndex::new(0));

        for root_entry in root_entries.iter().filter(|e| !e.name().starts_with('.')) {
            inner(self, root_entry, 0, out);
        }
    }

    pub fn flush(&mut self) {
        // Write the `INodeCache` to disk
        let mut inode_cache = core::mem::take(&mut self.inode_cache);

        for (idx, inode) in inode_cache.drain() {
            self.write_inode_to_disk(idx, &inode);
        }

        self.inode_cache = inode_cache;

        // Write the superblock to disk
        let superblock = SuperBlock {
            magic: MAGIC,
            block_size: BLOCK_SIZE as u32,
            total_blocks: self.block_device.total_blocks() as u32,
        };

        self.write_superblock(&superblock);

        // Write the INodeBitmap to disk
        let mut inode_bitmap_vec = self.inode_bitmap.to_bytes();

        if !inode_bitmap_vec.len().is_multiple_of(BLOCK_SIZE) {
            inode_bitmap_vec
                .extend([0u8].repeat(BLOCK_SIZE - (inode_bitmap_vec.len() % BLOCK_SIZE)));
        }

        let start_block = self.layout.inode_bitmap_start as u32;

        for (i, chunk) in inode_bitmap_vec[..].chunks(BLOCK_SIZE).enumerate() {
            let block = BlockIndex::from_raw(start_block + i as u32);
            self.block_device.write_block(block, chunk);
        }

        // Write the DataBitmap to disk
        let mut data_bitmap_vec = self.data_bitmap.to_bytes();
        if !data_bitmap_vec.len().is_multiple_of(BLOCK_SIZE) {
            data_bitmap_vec.extend([0u8].repeat(BLOCK_SIZE - (data_bitmap_vec.len() % BLOCK_SIZE)));
        }
        let start_block = self.layout.data_bitmap_start as u32;

        for (i, chunk) in data_bitmap_vec[..].chunks(BLOCK_SIZE).enumerate() {
            let block = BlockIndex::from_raw(start_block + i as u32);
            self.block_device.write_block(block, chunk);
        }

        log::debug!("flushed");
    }

    pub fn mkdir(&mut self, path: &str) -> Result<INodeIndex, Error> {
        self.new_dir_entry(path, Entry::Directory)
    }

    pub fn create_file(&mut self, path: &str) -> Result<INodeIndex, Error> {
        self.new_dir_entry(path, Entry::File)
    }

    pub fn write_to_file(&mut self, inode_index: INodeIndex, bytes: &[u8]) -> Result<usize, Error> {
        self.append_to_file(inode_index, bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RAMDISK_SIZE: usize = 1024 * 1024;
    const MAX_FILE_SIZE: usize = 16 * BLOCK_SIZE;

    struct Ramdisk {
        data: Vec<u8>,
    }

    impl Ramdisk {
        fn new() -> Self {
            Self {
                data: vec![0; RAMDISK_SIZE],
            }
        }
    }

    impl BlockDevice for Ramdisk {
        fn read_block(&mut self, block_idx: BlockIndex, buf: &mut [u8]) {
            assert_eq!(buf.len(), BLOCK_SIZE);

            let start = block_idx.inner() as usize * BLOCK_SIZE;
            let end = start + BLOCK_SIZE;

            assert!(end <= self.data.len());
            buf.copy_from_slice(&self.data[start..end]);
        }

        fn write_block(&mut self, block_idx: BlockIndex, data: &[u8]) {
            assert_eq!(data.len(), BLOCK_SIZE);

            let start = block_idx.inner() as usize * BLOCK_SIZE;
            let end = start + BLOCK_SIZE;

            assert!(end <= self.data.len());
            self.data[start..end].copy_from_slice(data);
        }

        fn total_blocks(&mut self) -> usize {
            self.data.len() / BLOCK_SIZE
        }
    }

    fn make_fs() -> Filesystem<Ramdisk> {
        Filesystem::new(Ramdisk::new())
    }

    fn inode_copy(fs: &mut Filesystem<Ramdisk>, idx: INodeIndex) -> INode {
        *fs.inode_cache.get(idx, &mut fs.block_device)
    }

    fn remount(fs: Filesystem<Ramdisk>) -> Filesystem<Ramdisk> {
        let Filesystem { block_device, .. } = fs;
        Filesystem::new(block_device)
    }

    fn find_entry_inode(
        fs: &mut Filesystem<Ramdisk>,
        dir_inode: INodeIndex,
        name: &str,
    ) -> Option<INodeIndex> {
        fs.read_dir_entry(dir_inode)
            .into_iter()
            .find(|entry| entry.name() == name)
            .map(|entry| entry.inode)
    }

    fn first_data_block(inode: &INode) -> Option<u32> {
        inode.blocks.iter().find_map(|block| block.value())
    }

    fn bitmap_capacity_bits(bitmap: &Bitmap) -> u32 {
        let bytes = bitmap.to_bytes();
        let words = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
        words * 32
    }

    fn bitmap_set_count(bitmap: &Bitmap) -> u32 {
        let bits = bitmap_capacity_bits(bitmap);
        (0..bits).filter(|&idx| bitmap.is_set(idx)).count() as u32
    }

    #[test]
    fn root_initialized_with_dot_entries() {
        let mut fs = make_fs();

        let root = inode_copy(&mut fs, INodeIndex::new(0));
        assert!(root.is_directory);

        let entries = fs.read_dir_entry(INodeIndex::new(0));
        assert_eq!(entries.len(), 2);

        let dot = entries.iter().find(|e| e.name() == ".").unwrap();
        let dotdot = entries.iter().find(|e| e.name() == "..").unwrap();

        assert_eq!(dot.inode.inner(), 0);
        assert_eq!(dotdot.inode.inner(), 0);
    }

    #[test]
    fn create_file_and_read_back_small_content() {
        let mut fs = make_fs();

        let idx = fs.create_file("/hello.txt").unwrap();
        let written = fs.write_to_file(idx, b"hello filesystem").unwrap();

        assert_eq!(written, 16);
        assert_eq!(fs.read_file(idx), "hello filesystem");
    }

    #[test]
    fn append_file_within_single_block() {
        let mut fs = make_fs();

        let idx = fs.create_file("/append.txt").unwrap();
        fs.write_to_file(idx, b"hello ").unwrap();
        fs.write_to_file(idx, b"world").unwrap();

        let inode = inode_copy(&mut fs, idx);
        let used_blocks = inode.blocks.iter().filter(|b| !b.is_none()).count();

        assert_eq!(fs.read_file(idx), "hello world");
        assert_eq!(inode.size, 11);
        assert_eq!(used_blocks, 1);
    }

    #[test]
    fn append_file_across_block_boundary() {
        let mut fs = make_fs();

        let idx = fs.create_file("/boundary.txt").unwrap();
        let first = vec![b'A'; BLOCK_SIZE - 1];
        let second = vec![b'B'; 10];

        fs.write_to_file(idx, &first).unwrap();
        fs.write_to_file(idx, &second).unwrap();

        let inode = inode_copy(&mut fs, idx);
        let used_blocks = inode.blocks.iter().filter(|b| !b.is_none()).count();
        let content = fs.read_file(idx);

        assert_eq!(content.len(), BLOCK_SIZE - 1 + 10);
        assert!(content.starts_with(&"A".repeat(BLOCK_SIZE - 1)));
        assert!(content.ends_with(&"B".repeat(10)));
        assert_eq!(used_blocks, 2);
    }

    #[test]
    fn write_zero_bytes_is_noop() {
        let mut fs = make_fs();

        let idx = fs.create_file("/noop.txt").unwrap();
        fs.write_to_file(idx, b"abc").unwrap();
        let before = inode_copy(&mut fs, idx);

        let written = fs.write_to_file(idx, b"").unwrap();
        let after = inode_copy(&mut fs, idx);

        assert_eq!(written, 0);
        assert_eq!(before.size, after.size);
        assert_eq!(fs.read_file(idx), "abc");
    }

    #[test]
    fn write_max_file_size_then_overflow() {
        let mut fs = make_fs();

        let idx = fs.create_file("/max.txt").unwrap();
        let content = vec![b'Z'; MAX_FILE_SIZE];

        let written = fs.write_to_file(idx, &content).unwrap();
        let overflow = fs.write_to_file(idx, b"!");
        let read_back = fs.read_file(idx);

        assert_eq!(written, MAX_FILE_SIZE);
        assert_eq!(overflow.err(), Some(Error::NoSpaceInFile));
        assert_eq!(read_back.len(), MAX_FILE_SIZE);
        assert!(read_back.as_bytes().iter().all(|b| *b == b'Z'));
    }

    #[test]
    fn writing_to_file() {
        let mut fs = make_fs();

        let index = fs.create_file("/text.txt").expect("Unable to create file");
        let content = "A".repeat(511);
        let bytes_written = fs
            .write_to_file(index, content.as_bytes())
            .expect("Failed to write to file");
        assert_eq!(bytes_written, 511);

        let failed = fs.create_file("/text.txt");
        assert_eq!(failed.err(), Some(Error::DuplicatedEntry));

        let other_file = fs
            .create_file("/other-text.txt")
            .expect("Unable to create file");
        let content = "B".repeat(513);
        let bytes_written = fs
            .write_to_file(other_file, content.as_bytes())
            .expect("Failed to write to file");
        assert_eq!(bytes_written, 513);

        let content = "C".repeat(5);
        let bytes_written = fs
            .write_to_file(index, content.as_bytes())
            .expect("Failed to write to file");
        assert_eq!(bytes_written, 5);
    }

    #[test]
    fn create_directory_structure() {
        let mut fs = make_fs();

        fs.mkdir("/test").expect("Could not create directory");
        fs.mkdir("/test/foo")
            .expect("Could not create nested directory");
        fs.mkdir("/foo")
            .expect("Could not create directory with same name in root");
    }

    #[test]
    fn duplicate_directory_name_in_same_dir_returns_error() {
        let mut fs = make_fs();

        fs.mkdir("/dup").unwrap();
        let res = fs.mkdir("/dup");

        assert_eq!(res.err(), Some(Error::DuplicatedEntry));
    }

    #[test]
    fn file_then_directory_same_name_in_same_parent_returns_error() {
        let mut fs = make_fs();

        fs.create_file("/same").unwrap();
        let res = fs.mkdir("/same");

        assert_eq!(res.err(), Some(Error::DuplicatedEntry));
    }

    #[test]
    fn directory_then_file_same_name_in_same_parent_returns_error() {
        let mut fs = make_fs();

        fs.mkdir("/same").unwrap();
        let res = fs.create_file("/same");

        assert_eq!(res.err(), Some(Error::DuplicatedEntry));
    }

    #[test]
    fn same_name_in_different_directories_is_allowed() {
        let mut fs = make_fs();

        fs.mkdir("/a").unwrap();
        fs.mkdir("/b").unwrap();

        assert!(fs.create_file("/a/x").is_ok());
        assert!(fs.create_file("/b/x").is_ok());
    }

    #[test]
    fn create_in_missing_parent_returns_directory_does_not_exist() {
        let mut fs = make_fs();

        let file_res = fs.create_file("/missing/file.txt");
        let dir_res = fs.mkdir("/missing/subdir");

        assert_eq!(file_res.err(), Some(Error::DirectoryDoesNotExist));
        assert_eq!(dir_res.err(), Some(Error::DirectoryDoesNotExist));
    }

    #[test]
    fn create_file_into_file_returns_not_a_directory() {
        let mut fs = make_fs();

        fs.create_file("/test.txt").unwrap();
        let res = fs.create_file("/test.txt/huh");

        assert_eq!(res.err(), Some(Error::NotADirectory));
    }

    #[test]
    fn mkdir_multi_level_into_file_component_returns_not_a_directory() {
        let mut fs = make_fs();

        fs.mkdir("/a").unwrap();
        fs.create_file("/a/file").unwrap();

        let res = fs.mkdir("/a/file/c");
        assert_eq!(res.err(), Some(Error::NotADirectory));
    }

    #[test]
    fn dot_points_to_self_for_new_directories() {
        let mut fs = make_fs();

        let a = fs.mkdir("/a").unwrap();
        let b = fs.mkdir("/a/b").unwrap();

        let a_entries = fs.read_dir_entry(a);
        let b_entries = fs.read_dir_entry(b);

        let a_dot = a_entries.iter().find(|e| e.name() == ".").unwrap();
        let b_dot = b_entries.iter().find(|e| e.name() == ".").unwrap();

        assert_eq!(a_dot.inode.inner(), a.inner());
        assert_eq!(b_dot.inode.inner(), b.inner());
    }

    #[test]
    fn dotdot_points_to_actual_parent_for_nested_dirs() {
        let mut fs = make_fs();

        let parent = fs.mkdir("/a").unwrap();
        let child = fs.mkdir("/a/b").unwrap();
        let child_entries = fs.read_dir_entry(child);

        let dotdot = child_entries.iter().find(|e| e.name() == "..").unwrap();

        assert_eq!(dotdot.inode.inner(), parent.inner());
    }

    #[test]
    fn directory_size_updates_with_entries() {
        let mut fs = make_fs();

        let dir = fs.mkdir("/size").unwrap();
        let initial = inode_copy(&mut fs, dir);

        fs.create_file("/size/one").unwrap();
        let after_one = inode_copy(&mut fs, dir);

        fs.create_file("/size/two").unwrap();
        let after_two = inode_copy(&mut fs, dir);

        let entry_size = core::mem::size_of::<DirEntry>() as u32;

        assert_eq!(after_one.size, initial.size + entry_size);
        assert_eq!(after_two.size, initial.size + 2 * entry_size);
    }

    #[test]
    fn directory_entry_capacity_limit() {
        let mut fs = make_fs();

        let cap = fs.mkdir("/cap").unwrap();
        let max_entries_for_inode = 16 * DIR_ENTRY_PER_BLOCK;
        let full_size = (max_entries_for_inode * core::mem::size_of::<DirEntry>()) as u32;

        let mut inode = fs.inode_cache.get_mut(cap, &mut fs.block_device);

        inode.size = full_size;
        for block in inode.blocks.iter_mut().filter(|b| b.is_none()) {
            *block = DataBlockIndex::from_raw_unchecked(1);
        }

        fs.inode_cache.get_mut(cap, &mut fs.block_device).size = full_size;

        let overflow = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            fs.create_file("/cap/overflow")
        }));

        assert!(
            overflow.is_ok(),
            "create_file should return Err(NoSpaceForDirEntry), not panic"
        );
        assert_eq!(overflow.unwrap().err(), Some(Error::NoSpaceForDirEntry));
    }

    #[test]
    fn name_exactly_24_bytes_roundtrip() {
        let mut fs = make_fs();

        let name = "abcdefghijklmnopqrstuvwx";
        assert_eq!(name.len(), 24);

        fs.create_file(&format!("/{name}")).unwrap();

        let names: Vec<_> = fs
            .read_dir_entry(INodeIndex::new(0))
            .into_iter()
            .map(|entry| entry.name())
            .collect();

        assert!(names.iter().any(|entry| entry == name));
    }

    #[test]
    fn name_longer_than_24_bytes_is_rejected() {
        let mut fs = make_fs();

        let name = "abcdefghijklmnopqrstuvwxy";
        assert_eq!(name.len(), 25);

        let res = fs.create_file(&format!("/{name}"));
        assert!(
            res.is_err(),
            "names longer than 24 bytes should be rejected"
        );
    }

    #[test]
    fn empty_path_is_rejected_without_panic() {
        let mut fs = make_fs();

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| fs.create_file("")));
        assert!(result.is_ok(), "empty path should not panic");
        assert!(
            result.unwrap().is_err(),
            "empty path should return an error"
        );
    }

    // TODO(mt): this is not supported right now
    // #[test]
    // fn relative_path_is_rejected_without_panic() {
    //     let mut fs = make_fs();

    //     let result =
    //         std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| fs.create_file("relative")));
    //     assert!(result.is_ok(), "relative path should not panic");
    //     assert!(
    //         result.unwrap().is_err(),
    //         "relative path should return an error"
    //     );
    // }

    #[test]
    fn root_path_is_rejected() {
        let mut fs = make_fs();

        let res = fs.create_file("/");
        assert!(res.is_err(), "`/` should not be a valid creatable entry");
    }

    #[test]
    fn trailing_slash_path_is_rejected() {
        let mut fs = make_fs();

        fs.mkdir("/dir").unwrap();
        let res = fs.create_file("/dir/");

        assert!(res.is_err(), "trailing slash path should be rejected");
    }

    #[test]
    fn flush_and_remount_preserves_structure_and_content() {
        let mut fs = make_fs();

        fs.mkdir("/dir").unwrap();
        let file = fs.create_file("/dir/notes.txt").unwrap();
        fs.write_to_file(file, b"persisted").unwrap();
        fs.flush();

        let mut fs = remount(fs);
        let dir = find_entry_inode(&mut fs, INodeIndex::new(0), "dir").expect("dir missing");
        let note = find_entry_inode(&mut fs, dir, "notes.txt").expect("notes missing");

        assert_eq!(note.inner(), file.inner());
        assert_eq!(fs.read_file(file), "persisted");
    }

    #[test]
    fn flush_persists_bitmaps_for_future_allocations() {
        let mut fs = make_fs();

        let first = fs.create_file("/one").unwrap();
        fs.write_to_file(first, b"hello").unwrap();
        let first_block = first_data_block(&inode_copy(&mut fs, first)).unwrap();
        fs.flush();

        let mut fs = remount(fs);
        let second = fs.create_file("/two").unwrap();
        fs.write_to_file(second, b"world").unwrap();

        let second_block = first_data_block(&inode_copy(&mut fs, second)).unwrap();

        assert!(second.inner() > first.inner());
        assert_ne!(second_block, first_block);
    }

    #[test]
    fn flush_is_idempotent_without_mutations() {
        let mut fs = make_fs();

        fs.mkdir("/idempotent").unwrap();
        let file = fs.create_file("/idempotent/file.txt").unwrap();
        fs.write_to_file(file, b"hello").unwrap();

        fs.flush();
        let after_first_flush = fs.block_device.data.clone();

        fs.flush();
        let after_second_flush = fs.block_device.data.clone();

        assert_eq!(after_first_flush, after_second_flush);
    }

    #[test]
    fn remount_multiple_times_preserves_state() {
        let mut fs = make_fs();

        let idx = fs.create_file("/loop.txt").unwrap();
        fs.write_to_file(idx, b"stable").unwrap();
        fs.flush();

        for _ in 0..3 {
            fs = remount(fs);
            assert_eq!(fs.read_file(idx), "stable");
            fs.flush();
        }
    }

    #[test]
    fn append_after_remount_preserves_and_extends() {
        let mut fs = make_fs();

        let idx = fs.create_file("/append-remount.txt").unwrap();
        fs.write_to_file(idx, b"abc").unwrap();
        fs.flush();

        let mut fs = remount(fs);
        fs.write_to_file(idx, b"def").unwrap();
        assert_eq!(fs.read_file(idx), "abcdef");
        fs.flush();

        let mut fs = remount(fs);
        assert_eq!(fs.read_file(idx), "abcdef");
    }

    #[test]
    fn inode_exhaustion_returns_error_not_panic() {
        let mut fs = make_fs();

        let bits = bitmap_capacity_bits(&fs.inode_bitmap);
        for idx in 0..bits {
            fs.inode_bitmap.set(idx);
        }

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            fs.create_file("/inode-oom")
        }));

        assert!(
            result.is_ok(),
            "inode exhaustion should return Err, not panic"
        );
        assert!(result.unwrap().is_err());
    }

    #[test]
    fn data_block_exhaustion_returns_error_not_panic() {
        let mut fs = make_fs();

        let file = fs.create_file("/data-oom.txt").unwrap();

        let bits = bitmap_capacity_bits(&fs.data_bitmap);
        for idx in 0..bits {
            fs.data_bitmap.set(idx);
        }

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            fs.write_to_file(file, b"x")
        }));

        assert!(
            result.is_ok(),
            "data exhaustion should return Err, not panic"
        );
        assert!(result.unwrap().is_err());
    }

    #[test]
    fn failed_create_does_not_leak_inode_allocation() {
        let mut fs = make_fs();

        let cap = fs.mkdir("/cap-leak").unwrap();
        let max_entries = 16 * DIR_ENTRY_PER_BLOCK;
        let full_size = (max_entries * core::mem::size_of::<DirEntry>()) as u32;

        let mut inode = fs.inode_cache.get_mut(cap, &mut fs.block_device);

        inode.size = full_size;
        for block in inode.blocks.iter_mut().filter(|b| b.is_none()) {
            *block = DataBlockIndex::from_raw_unchecked(1);
        }

        let before = bitmap_set_count(&fs.inode_bitmap);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            fs.create_file("/cap-leak/new")
        }));

        let after = bitmap_set_count(&fs.inode_bitmap);

        assert_eq!(
            before, after,
            "failed create must not leak inode allocations"
        );
        assert!(result.is_ok(), "failed create should return Err, not panic");
        assert_eq!(result.unwrap().err(), Some(Error::NoSpaceForDirEntry));
    }

    #[test]
    fn remount_without_flush_drops_unflushed_namespace_changes() {
        let mut fs = make_fs();

        fs.mkdir("/tmp").unwrap();
        fs.create_file("/tmp/ephemeral.txt").unwrap();

        let mut fs = remount(fs);
        assert!(find_entry_inode(&mut fs, INodeIndex::new(0), "tmp").is_none());
    }

    // TODO(mt): come back to this case
    // #[test]
    // fn write_to_unallocated_inode_is_rejected() {
    //     let mut fs = make_fs();

    //     let res = fs.write_to_file(INodeIndex::new(123), b"x");
    //     assert!(res.is_err(), "writing to unallocated inode should fail");
    // }

    // TODO(mt): come back to this case
    // #[test]
    // fn read_from_unallocated_inode_should_not_succeed() {
    //     let mut fs = make_fs();

    //     let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
    //         fs.read_file(INodeIndex::new(222))
    //     }));

    //     assert!(
    //         result.is_err(),
    //         "reading from an unallocated inode should panic or return an error"
    //     );
    // }

    #[test]
    fn file_data_blocks_are_not_shared_between_files() {
        let mut fs = make_fs();

        let first = fs.create_file("/first.txt").unwrap();
        let second = fs.create_file("/second.txt").unwrap();

        fs.write_to_file(first, b"aaa").unwrap();
        fs.write_to_file(second, b"bbb").unwrap();

        let first_block = first_data_block(&inode_copy(&mut fs, first)).unwrap();
        let second_block = first_data_block(&inode_copy(&mut fs, second)).unwrap();

        assert_ne!(first_block, second_block);
    }

    #[test]
    fn writing_multiple_blocks() {
        let mut fs = make_fs();

        let content = "A".repeat(MAX_FILE_SIZE);
        let index = fs.create_file("/test.txt").unwrap();
        fs.write_to_file(index, content.as_bytes()).unwrap();

        let res = fs.write_to_file(index, b"overflow");
        assert_eq!(res.err(), Some(Error::NoSpaceInFile));
    }

    #[test]
    fn file_and_directory_with_same_name() {
        let mut fs = make_fs();

        fs.mkdir("/test").unwrap();
        fs.mkdir("/test/x").unwrap();
        fs.create_file("/test/x.txt").unwrap();
    }

    #[test]
    fn writing_to_directory_returns_error() {
        let mut fs = make_fs();

        let index = fs.mkdir("/test").unwrap();
        let res = fs.write_to_file(index, b"xd");
        assert_eq!(res.err(), Some(Error::NotAFile));
    }

    #[test]
    fn mkdir_into_file_returns_error() {
        let mut fs = make_fs();

        fs.create_file("/test.txt").unwrap();
        let res = fs.mkdir("/test.txt/huh");
        assert_eq!(res.err(), Some(Error::NotADirectory));
    }
}
