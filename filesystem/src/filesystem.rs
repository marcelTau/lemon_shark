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
use crate::dir_entry::DirEntry;
use crate::inode::{INODE_BLOCKS, INode};
use crate::inode_cache::INodeCache;
use crate::layout::Layout;
use crate::{BlockIndex, INodeIndex};
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::mem;

/// Number of `DirEntry` per block.
pub(crate) const DIR_ENTRY_PER_BLOCK: usize = BLOCK_SIZE / core::mem::size_of::<DirEntry>();

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
    NotADirectory,
    NotAFile,
    NoSpaceInFile,
    OutOfMemory,

    // -- new errors --
    DuplicatedEntry,
    NoNameProvided,
    NameTooLong,
    NotFound,
    NotEmpty,
    INodeBlocksExhausted,
    INodeBitmapExhausted,
    Unsupported,
    InvalidRootNode,
    InvalidSuperblock,
}

impl core::error::Error for Error {}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}

struct Buffer {
    buf: [u8; BLOCK_SIZE],
}

impl Buffer {
    fn new() -> Self {
        Self {
            buf: [0u8; BLOCK_SIZE],
        }
    }

    fn read_dir_entry_from(&self, start: usize) -> DirEntry {
        let end = start + mem::size_of::<DirEntry>();
        DirEntry::from_bytes(&self.buf[start..end])
    }

    fn write_dir_entry(&mut self, start: usize, entry: DirEntry) {
        let end = start + mem::size_of::<DirEntry>();
        self.buf[start..end].copy_from_slice(&entry.to_bytes());
    }

    fn dir_entries(&self) -> impl Iterator<Item = DirEntry> {
        let mut i = 0;
        core::iter::from_fn(move || {
            if i + mem::size_of::<DirEntry>() > BLOCK_SIZE {
                return None;
            }
            let entry = self.read_dir_entry_from(i);
            i += mem::size_of::<DirEntry>();
            Some(entry)
        })
    }

    fn remove_dir_entry(&mut self, start: usize) -> DirEntry {
        let end = start + mem::size_of::<DirEntry>();
        let entry = self.read_dir_entry_from(start);
        self.buf[start..end].fill(0);
        entry
    }

    fn clear(&mut self) {
        self.buf.fill(0);
    }

    fn inner(&mut self) -> &mut [u8] {
        &mut self.buf[..]
    }
}

pub(crate) struct DirEntryReader<'dev, Dev> {
    device: &'dev mut Dev,
    inode: INode,
    block_cursor: usize, // index into inode.blocks[]
    buf: Buffer,
    buf_pos: usize,   // entry index within current buffer
    buf_len: usize,   // valid entries in current buffer (handles partial last block)
    total: usize,     // total number of entries in the directory
    remaining: usize, // total entries left to yield
}

pub(crate) struct PositionDirEntry {
    entry: DirEntry,
    block_index: BlockIndex,
    byte_offset: usize,
    logical_index: usize,
}

impl<'dev, Dev: BlockDevice> DirEntryReader<'dev, Dev> {
    pub(crate) fn new(device: &'dev mut Dev, inode: INode) -> Self {
        let total = unsafe { inode.current_dir_entries() };
        Self {
            device,
            inode,
            block_cursor: 0,
            buf: Buffer::new(),
            buf_pos: 0,
            buf_len: 0,
            total,
            remaining: total,
        }
    }

    fn load_next_block(&mut self) -> bool {
        while self.block_cursor < INODE_BLOCKS {
            let slot = self.inode.block(self.block_cursor);
            self.block_cursor += 1;
            if let Some(block) = slot.to_block() {
                self.device.read_block(block, self.buf.inner());
                self.buf_pos = 0;
                self.buf_len = self.remaining.min(DIR_ENTRY_PER_BLOCK);
                return true;
            }
        }
        false
    }
}

impl<'dev, Dev: BlockDevice> Iterator for DirEntryReader<'dev, Dev> {
    type Item = PositionDirEntry;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        if self.buf_pos >= self.buf_len && !self.load_next_block() {
            return None;
        }

        let entry = self
            .buf
            .read_dir_entry_from(self.buf_pos * mem::size_of::<DirEntry>());
        self.buf_pos += 1;
        self.remaining -= 1;

        self.inode
            .block(self.block_cursor - 1)
            .to_block()
            .map(|block_index| PositionDirEntry {
                entry,
                block_index,
                byte_offset: (self.buf_pos - 1) * mem::size_of::<DirEntry>(),
                logical_index: self.total - self.remaining - 1,
            })
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

struct ResolvedPath {
    parent: INodeIndex,
    basename: INodeIndex,
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
    /// Reads the superblock from block_index 0 if the filesystem has the right format.
    fn read_superblock(block_device: &mut Dev) -> Option<SuperBlock> {
        let mut buf = [0u8; BLOCK_SIZE];
        block_device.read_block(BlockIndex::from_raw(0), &mut buf);
        let sb = SuperBlock::from_bytes(&buf[0..mem::size_of::<SuperBlock>()]);

        (sb.magic == MAGIC).then_some(sb)
    }

    pub fn new(mut block_device: Dev) -> Result<Self, Error> {
        let sb = Self::read_superblock(&mut block_device).ok_or(Error::InvalidSuperblock)?;
        let layout = Layout::new(sb.total_blocks);

        log::info!("generated layout: {layout:?}");

        let mut read_bitmap = |blocks: usize, bitmap_start: usize| -> Vec<u8> {
            let mut raw = vec![0u8; BLOCK_SIZE * blocks];
            for i in 0..blocks {
                let start = i * BLOCK_SIZE;
                let end = start + BLOCK_SIZE;
                let block_index = BlockIndex::from_raw((bitmap_start + i) as u32);
                block_device.read_block(block_index, &mut raw[start..end]);
            }

            raw
        };

        let inode_bitmap_raw = read_bitmap(layout.inode_bitmap_blocks, layout.inode_bitmap_start);
        let data_bitmap_raw = read_bitmap(layout.data_bitmap_blocks, layout.data_bitmap_start);

        let inode_bitmap = Bitmap::from_bytes(&inode_bitmap_raw);
        let data_bitmap = Bitmap::from_bytes(&data_bitmap_raw);

        let mut fs = Self {
            inode_bitmap,
            data_bitmap,
            inode_cache: INodeCache::new(layout),
            block_device,
            layout,
        };

        fs.validate_root_inode()?;

        Ok(fs)
    }

    /// Formats a blank block device as a LemonShark filesystem.
    ///
    /// Writes the superblock, initialises empty bitmaps, creates the root
    /// directory inode with `.` and `..` entries, and flushes everything to
    /// disk. The returned `Filesystem` is ready for use (or can simply be
    /// dropped if the caller only needed the formatted image).
    pub fn format(mut block_device: Dev) {
        let total_blocks = block_device.total_blocks() as u32;
        let layout = Layout::new(total_blocks);

        // Each bitmap block stores: 4-byte word_count header + word_count*4
        // bytes of bit data.  To guarantee the serialised form fits within the
        // allocated blocks we size the bitmap so that header + data == exactly
        // N * BLOCK_SIZE bytes:
        //   usable_words = (N * BLOCK_SIZE - 4) / 4   →   usable_bits = usable_words * 32
        let inode_bitmap_bits = ((layout.inode_bitmap_blocks * BLOCK_SIZE - 4) / 4 * 32) as u32;
        let data_bitmap_bits = ((layout.data_bitmap_blocks * BLOCK_SIZE - 4) / 4 * 32) as u32;

        let mut fs = Self {
            block_device,
            inode_bitmap: Bitmap::new(inode_bitmap_bits),
            data_bitmap: Bitmap::new(data_bitmap_bits),
            inode_cache: INodeCache::new(layout),
            layout,
        };

        fs.create_empty_root();
        fs.flush();
    }

    /// Returns a mutable reference to the underlying block device.
    /// Useful for device-specific operations like debug dumps.
    pub fn block_device_mut(&mut self) -> &mut Dev {
        &mut self.block_device
    }

    fn validate_root_inode(&mut self) -> Result<(), Error> {
        let mut buf = [0u8; BLOCK_SIZE];

        let (index, _) = self.layout.inode_to_block(INodeIndex::new(0));
        self.block_device.read_block(index, &mut buf);

        let root_node = INode::from_bytes(&buf[0..mem::size_of::<INode>()]);

        if !root_node.is_directory() {
            return Err(Error::InvalidRootNode);
        }

        Ok(())
    }

    /// Writes an `INode` to disk.
    fn write_inode_to_disk(&mut self, inode_index: INodeIndex, inode: &INode) {
        let (block_index, byte_offset) = self.layout.inode_to_block(inode_index);

        modify_block(&mut self.block_device, block_index, |buf| {
            buf.inner()[byte_offset.range::<INode>()].copy_from_slice(inode.to_bytes().as_slice())
        });
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

    fn byte_compare(s: &str, entry: &str) -> bool {
        if entry.starts_with('/') {
            s == &entry[1..]
        } else {
            s == entry
        }
    }

    fn resolve_path(&mut self, path: &str) -> Result<ResolvedPath, Error> {
        let mut parent_dir = INodeIndex::new(0);

        let (path, basename) = path.rsplit_once('/').unwrap_or((path, path));

        for part in path.split('/').filter(|s| !s.is_empty()) {
            let dir_entries = self.read_dir_entry(parent_dir);

            let next = dir_entries
                .iter()
                .find(|e| Self::byte_compare(part, &e.name()))
                .ok_or(Error::NotFound)?;

            // TODO(mt): this check doens't allow files and directories to have the same name. That is fine for now!
            if !self.lookup_inode(next.inode()).is_directory() {
                return Err(Error::NotADirectory);
            }

            parent_dir = next.inode();
        }

        let dir_entries = self.read_dir_entry(parent_dir);

        let file = dir_entries
            .iter()
            .find(|e| Self::byte_compare(basename, &e.name()))
            .ok_or(Error::NotFound)?;

        Ok(ResolvedPath {
            parent: parent_dir,
            basename: file.inode(),
        })
    }

    fn lookup_inode(&mut self, inode_index: INodeIndex) -> &INode {
        self.inode_cache.get(inode_index, &mut self.block_device)
    }

    fn lookup_inode_mut(&mut self, inode_index: INodeIndex) -> &mut INode {
        self.inode_cache
            .get_mut(inode_index, &mut self.block_device)
    }

    pub fn remove_dir_entry(&mut self, path: &str) -> Result<(), Error> {
        let resolved = self.resolve_path(path)?;
        let parent_inode = *self.lookup_inode(resolved.parent);

        let num_parent_entries = unsafe { parent_inode.current_dir_entries() };

        let to_remove = path.rsplit_once('/').map(|(_, n)| n).unwrap_or(path);

        let found = DirEntryReader::new(self.block_device_mut(), parent_inode)
            .find(|entry| entry.entry.name() == to_remove)
            .ok_or(Error::NotFound)?;

        let last_block_slot = (num_parent_entries - 1) / DIR_ENTRY_PER_BLOCK;
        let last_block_offset =
            (num_parent_entries - 1) % DIR_ENTRY_PER_BLOCK * mem::size_of::<DirEntry>();

        let Some(last_block_index) = parent_inode.block(last_block_slot).to_block() else {
            log::error!("Could not find last_block_index. This should never happen.");
            return Err(Error::NotFound);
        };

        let last_entry = modify_block(&mut self.block_device, last_block_index, |buf| {
            buf.remove_dir_entry(last_block_offset)
        });

        if found.logical_index != num_parent_entries - 1 {
            // swap-remove the entry
            modify_block(&mut self.block_device, found.block_index, |buf| {
                buf.write_dir_entry(found.byte_offset, last_entry)
            });
        }

        let is_only_entry_in_last_block =
            (num_parent_entries - 1).is_multiple_of(DIR_ENTRY_PER_BLOCK);

        if is_only_entry_in_last_block {
            // removing the last entry in the directory, remove last block from the parent inode
            self.data_bitmap.unset(
                parent_inode
                    .block(last_block_slot)
                    .bitmap_index(&self.layout),
            );
            self.lookup_inode_mut(resolved.parent)
                .block_mut(last_block_slot)
                .unwrap()
                .clear();
        }

        self.lookup_inode_mut(resolved.parent)
            .shrink(mem::size_of::<DirEntry>());

        // Free the blocks of the deleted inode.
        let to_remove_inode = *self.lookup_inode(resolved.basename);

        // TODO(mt): right now we only support removing files, not directories as this would require removing all of it's files etc.
        if to_remove_inode.is_directory() {
            return Err(Error::Unsupported);
        }

        // Free the blocks of the deleted inode.
        for block in to_remove_inode.used_blocks() {
            let block_index = block.to_block().expect("Checked in `used_blocks`");
            modify_block(&mut self.block_device, block_index, |buf| buf.clear());
            self.data_bitmap.unset(block.bitmap_index(&self.layout));
        }

        // TODO(mt): double check that this is correct.
        self.inode_bitmap.unset(resolved.basename.inner());
        self.inode_cache.remove(resolved.basename);

        Ok(())
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
                .find(|e| Self::byte_compare(part, &e.name()))
                .ok_or(Error::NotFound)?;

            // If the INode that matches the `part` name is not a directory
            // return an error as we can't go in there.
            if !self
                .inode_cache
                .get(next.inode(), &mut self.block_device)
                .is_directory()
            {
                return Err(Error::NotADirectory);
            }

            // Update the current index.
            current = next.inode();
        }

        let parent_inode = self.inode_cache.get(current, &mut self.block_device);

        if !parent_inode.has_space() {
            return Err(Error::INodeBlocksExhausted);
        }

        if self
            .read_dir_entry(current)
            .iter()
            .any(|e| Self::byte_compare(new_entry_name, &e.name()))
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
            .ok_or(Error::INodeBitmapExhausted)?;

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
        const DIR_ENTRY_SIZE: usize = mem::size_of::<DirEntry>();

        let inode = self
            .inode_cache
            .get_mut(inode_index, &mut self.block_device);

        if !inode.is_directory() {
            return Err(Error::NotADirectory);
        }

        let slot = inode.write_slot();

        let Some(block) = slot.block else {
            return Err(Error::INodeBlocksExhausted);
        };

        if block.is_empty() {
            let free = self.data_bitmap.find_free().unwrap();
            let free_block_index = self.layout.data_block(free);
            *block = free_block_index;
            self.data_bitmap.set(free);
        }

        let block_index = block.to_block().expect("Is set by block above");

        modify_block(&mut self.block_device, block_index, |buf| {
            buf.write_dir_entry(slot.byte_offset, entry);
        });

        inode.advance(DIR_ENTRY_SIZE);

        Ok(())
    }

    /// Reads all the `DirEntry`s for that INode and returns them in a Vec.
    fn read_dir_entry(&mut self, inode_index: INodeIndex) -> Vec<DirEntry> {
        // Get the `INode`
        let inode = self.inode_cache.get(inode_index, &mut self.block_device);

        // If the `INode` is empty there is nothing to do here.
        if inode.size() == 0 {
            return Vec::new();
        }

        let max_items = unsafe { inode.current_dir_entries() };

        let mut res = Vec::with_capacity(max_items);
        let mut buf = Buffer::new();

        // Loop and read all `DirEntry`s into `res`.
        for block_index in inode.used_blocks().flat_map(|b| b.to_block()) {
            self.block_device.read_block(block_index, buf.inner());

            let items_in_block = (max_items - res.len()).min(DIR_ENTRY_PER_BLOCK);

            for i in 0..items_in_block {
                let start = i * mem::size_of::<DirEntry>();
                res.push(buf.read_dir_entry_from(start));
            }

            if res.len() == max_items {
                break;
            }
        }

        res
    }

    pub fn create_empty_root(&mut self) {
        // Create the root INode.
        let root_inode = INode::new_empty_directory();

        // Write the node to disk to get the `INodeIndex`.
        let root_inode_index = self
            .new_inode(&root_inode)
            .expect("There is space when creating the root");

        log::error!("Root inode index: {root_inode_index:?}");

        // Create the default directories in the root directory.
        let this = DirEntry::new(String::from("."), root_inode_index);
        let this_too = DirEntry::new(String::from(".."), root_inode_index);

        self.write_dir_entry(this, root_inode_index).unwrap();
        self.write_dir_entry(this_too, root_inode_index).unwrap();

        log::info!("initialized with empty root directory");
    }

    fn append_to_file(&mut self, path: &str, bytes: &[u8]) -> Result<usize, Error> {
        let resolved = self.resolve_path(path)?;

        let inode = self
            .inode_cache
            .get_mut(resolved.basename, &mut self.block_device);

        if inode.is_directory() {
            return Err(Error::NotAFile);
        }

        if inode.size() as usize + bytes.len() > 16 * BLOCK_SIZE {
            return Err(Error::NoSpaceInFile);
        }

        let mut total_bytes = bytes.len();
        let mut bytes_written = 0;

        while total_bytes > 0 {
            let slot = inode.write_slot();

            let Some(block) = slot.block else {
                return Err(Error::NoSpaceInFile);
            };

            if block.is_empty() {
                let free = self.data_bitmap.find_free().ok_or(Error::OutOfMemory)?;
                let free_block_index = self.layout.data_block(free);
                *block = free_block_index;
                self.data_bitmap.set(free);
            }

            let bytes_to_write = total_bytes.min(slot.capacity);
            log::debug!("writing {}/{} bytes", bytes_to_write, total_bytes);

            let block_index = block
                .to_block()
                .expect("Is set by the block above in this function");

            modify_block(&mut self.block_device, block_index, |buf| {
                let write_start = slot.byte_offset;
                let write_end = write_start + bytes_to_write;

                let read_start = bytes_written;
                let read_end = read_start + bytes_to_write;
                buf.inner()[write_start..write_end].copy_from_slice(&bytes[read_start..read_end]);
            });

            inode.advance(bytes_to_write);

            total_bytes -= bytes_to_write;
            bytes_written += bytes_to_write;
        }

        Ok(bytes_written)
    }

    pub fn read_file(&mut self, path: &str) -> Result<String, Error> {
        let resolved = self.resolve_path(path)?;

        let inode = self
            .inode_cache
            .get_mut(resolved.basename, &mut self.block_device);

        if inode.is_directory() {
            return Err(Error::NotAFile);
        }

        let mut buf = Buffer::new();

        let mut total_bytes = inode.size() as usize;
        let mut string = String::with_capacity(total_bytes);

        for block in inode.used_blocks().map(|b| b.to_block().unwrap()) {
            self.block_device.read_block(block, buf.inner());

            let valid_bytes = total_bytes.min(BLOCK_SIZE);
            total_bytes -= valid_bytes;

            string.push_str(str::from_utf8(&buf.inner()[..valid_bytes]).unwrap());
        }

        Ok(string)
    }

    /// Writes the superblock to block_index 0
    fn write_superblock(&mut self, superblock: &SuperBlock) {
        modify_block(&mut self.block_device, BlockIndex::from_raw(0), |buf| {
            buf.clear();
            buf.inner()[0..mem::size_of::<SuperBlock>()].copy_from_slice(&superblock.to_bytes());
        });
    }

    pub fn dump_dir(&mut self, path: &str, out: &mut impl core::fmt::Write) -> Result<(), Error> {
        log::info!("`ls` for \"{path}\"");

        let inode_index = match path {
            "/" => INodeIndex::root(),
            _ => self.resolve_path(path)?.basename,
        };

        log::info!("Found inode_index={inode_index:?} for path=\"{path}\"");

        let mut buf = Buffer::new();

        let inode = *self.lookup_inode(inode_index);
        log::info!("Found inode={inode:?} for path=\"{path}\"");

        if !inode.is_directory() {
            return Err(Error::NotADirectory);
        }

        for block in inode.used_blocks().map(|b| b.to_block().unwrap()) {
            self.block_device.read_block(block, buf.inner());

            buf.dir_entries()
                .filter(|e| !e.name().is_empty())
                .for_each(|e| {
                    writeln!(out, "\t{e:?}").unwrap();
                });
        }

        Ok(())
    }

    pub fn tree(&mut self, out: &mut impl core::fmt::Write) {
        fn inner(
            fs: &mut Filesystem<impl BlockDevice>,
            entry: &DirEntry,
            indent: u8,
            out: &mut impl core::fmt::Write,
        ) {
            let entries = fs.read_dir_entry(entry.inode());

            if !entries.iter().any(|e| {
                fs.inode_cache
                    .get(e.inode(), &mut fs.block_device)
                    .is_directory()
            }) {
                return;
            }

            let inode = fs.inode_cache.get(entry.inode(), &mut fs.block_device);

            if inode.is_directory() {
                let _ = writeln!(out, "{}{}", " ".repeat(indent as usize), entry.name());
            }

            for entry in entries.iter().filter(|e| !e.name().starts_with('.')) {
                inner(fs, entry, indent + 2, out);
                if !fs
                    .inode_cache
                    .get(entry.inode(), &mut fs.block_device)
                    .is_directory()
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

    pub fn write_to_file(&mut self, path: &str, bytes: &[u8]) -> Result<usize, Error> {
        self.append_to_file(path, bytes)
    }
}

fn modify_block<Dev: BlockDevice, R, F: FnOnce(&mut Buffer) -> R>(
    block_device: &mut Dev,
    block_index: BlockIndex,
    f: F,
) -> R {
    let mut buf = Buffer::new();
    block_device.read_block(block_index, buf.inner());
    let result = f(&mut buf);
    block_device.write_block(block_index, buf.inner());
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::DataBlockIndex;
    use std::cell::RefCell;
    use std::rc::Rc;

    const RAMDISK_SIZE: usize = 1024 * 1024;
    const MAX_FILE_SIZE: usize = 16 * BLOCK_SIZE;

    struct Ramdisk {
        data: Rc<RefCell<Vec<u8>>>,
    }

    impl Ramdisk {
        fn new() -> Self {
            Self {
                data: Rc::new(RefCell::new(vec![0; RAMDISK_SIZE])),
            }
        }

        fn share(&self) -> Self {
            Self {
                data: Rc::clone(&self.data),
            }
        }
    }

    impl BlockDevice for Ramdisk {
        fn read_block(&mut self, block_idx: BlockIndex, buf: &mut [u8]) {
            assert_eq!(buf.len(), BLOCK_SIZE);
            let data = self.data.borrow();
            let start = block_idx.inner() as usize * BLOCK_SIZE;
            let end = start + BLOCK_SIZE;
            assert!(end <= data.len());
            buf.copy_from_slice(&data[start..end]);
        }

        fn write_block(&mut self, block_idx: BlockIndex, data: &[u8]) {
            assert_eq!(data.len(), BLOCK_SIZE);
            let mut d = self.data.borrow_mut();
            let start = block_idx.inner() as usize * BLOCK_SIZE;
            let end = start + BLOCK_SIZE;
            assert!(end <= d.len());
            d[start..end].copy_from_slice(data);
        }

        fn total_blocks(&mut self) -> usize {
            self.data.borrow().len() / BLOCK_SIZE
        }
    }

    fn make_fs() -> Filesystem<Ramdisk> {
        let ramdisk = Ramdisk::new();
        let shared = ramdisk.share();
        Filesystem::format(ramdisk);
        Filesystem::new(shared).expect("failed to mount freshly formatted filesystem")
    }

    fn inode_copy(fs: &mut Filesystem<Ramdisk>, idx: INodeIndex) -> INode {
        *fs.inode_cache.get(idx, &mut fs.block_device)
    }

    fn remount(fs: Filesystem<Ramdisk>) -> Filesystem<Ramdisk> {
        let Filesystem { block_device, .. } = fs;
        Filesystem::new(block_device).expect("remount failed")
    }

    fn find_entry_inode(
        fs: &mut Filesystem<Ramdisk>,
        dir_inode: INodeIndex,
        name: &str,
    ) -> Option<INodeIndex> {
        fs.read_dir_entry(dir_inode)
            .into_iter()
            .find(|entry| entry.name() == name)
            .map(|entry| entry.inode())
    }

    fn first_data_block(inode: &INode) -> Option<u32> {
        inode
            .used_blocks()
            .find_map(|block| block.to_block())
            .map(|b| b.inner())
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
        assert!(root.is_directory());

        let entries = fs.read_dir_entry(INodeIndex::new(0));
        assert_eq!(entries.len(), 2);

        let dot = entries.iter().find(|e| e.name() == ".").unwrap();
        let dotdot = entries.iter().find(|e| e.name() == "..").unwrap();

        assert_eq!(dot.inode().inner(), 0);
        assert_eq!(dotdot.inode().inner(), 0);
    }

    #[test]
    fn create_file_and_read_back_small_content() {
        let mut fs = make_fs();

        fs.create_file("/hello.txt").unwrap();
        let written = fs.write_to_file("/hello.txt", b"hello filesystem").unwrap();

        assert_eq!(written, 16);
        assert_eq!(fs.read_file("/hello.txt").unwrap(), "hello filesystem");
    }

    #[test]
    fn append_file_within_single_block() {
        let mut fs = make_fs();

        let idx = fs.create_file("/append.txt").unwrap();
        fs.write_to_file("/append.txt", b"hello ").unwrap();
        fs.write_to_file("/append.txt", b"world").unwrap();

        let inode = inode_copy(&mut fs, idx);
        let used_blocks = inode.used_blocks().count();

        assert_eq!(fs.read_file("/append.txt").unwrap(), "hello world");
        assert_eq!(inode.size(), 11);
        assert_eq!(used_blocks, 1);
    }

    #[test]
    fn append_file_across_block_boundary() {
        let mut fs = make_fs();

        let idx = fs.create_file("/boundary.txt").unwrap();
        let first = vec![b'A'; BLOCK_SIZE - 1];
        let second = vec![b'B'; 10];

        fs.write_to_file("/boundary.txt", &first).unwrap();
        fs.write_to_file("/boundary.txt", &second).unwrap();

        let inode = inode_copy(&mut fs, idx);
        let used_blocks = inode.used_blocks().count();
        let content = fs.read_file("/boundary.txt").unwrap();

        assert_eq!(content.len(), BLOCK_SIZE - 1 + 10);
        assert!(content.starts_with(&"A".repeat(BLOCK_SIZE - 1)));
        assert!(content.ends_with(&"B".repeat(10)));
        assert_eq!(used_blocks, 2);
    }

    #[test]
    fn write_zero_bytes_is_noop() {
        let mut fs = make_fs();

        let idx = fs.create_file("/noop.txt").unwrap();
        fs.write_to_file("/noop.txt", b"abc").unwrap();
        let before = inode_copy(&mut fs, idx);

        let written = fs.write_to_file("/noop.txt", b"").unwrap();
        let after = inode_copy(&mut fs, idx);

        assert_eq!(written, 0);
        assert_eq!(before.size(), after.size());
        assert_eq!(fs.read_file("/noop.txt").unwrap(), "abc");
    }

    #[test]
    fn write_max_file_size_then_overflow() {
        let mut fs = make_fs();

        fs.create_file("/max.txt").unwrap();
        let content = vec![b'Z'; MAX_FILE_SIZE];

        let written = fs.write_to_file("/max.txt", &content).unwrap();
        let overflow = fs.write_to_file("/max.txt", b"!");
        let read_back = fs.read_file("/max.txt").unwrap();

        assert_eq!(written, MAX_FILE_SIZE);
        assert_eq!(overflow.err(), Some(Error::NoSpaceInFile));
        assert_eq!(read_back.len(), MAX_FILE_SIZE);
        assert!(read_back.as_bytes().iter().all(|b| *b == b'Z'));
    }

    #[test]
    fn writing_to_file() {
        let mut fs = make_fs();

        fs.create_file("/text.txt").expect("Unable to create file");
        let content = "A".repeat(511);
        let bytes_written = fs
            .write_to_file("/text.txt", content.as_bytes())
            .expect("Failed to write to file");
        assert_eq!(bytes_written, 511);

        let failed = fs.create_file("/text.txt");
        assert_eq!(failed.err(), Some(Error::DuplicatedEntry));

        fs.create_file("/other-text.txt")
            .expect("Unable to create file");
        let content = "B".repeat(513);
        let bytes_written = fs
            .write_to_file("/other-text.txt", content.as_bytes())
            .expect("Failed to write to file");
        assert_eq!(bytes_written, 513);

        let content = "C".repeat(5);
        let bytes_written = fs
            .write_to_file("/text.txt", content.as_bytes())
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

        assert_eq!(file_res.err(), Some(Error::NotFound));
        assert_eq!(dir_res.err(), Some(Error::NotFound));
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

        assert_eq!(a_dot.inode().inner(), a.inner());
        assert_eq!(b_dot.inode().inner(), b.inner());
    }

    #[test]
    fn dotdot_points_to_actual_parent_for_nested_dirs() {
        let mut fs = make_fs();

        let parent = fs.mkdir("/a").unwrap();
        let child = fs.mkdir("/a/b").unwrap();
        let child_entries = fs.read_dir_entry(child);

        let dotdot = child_entries.iter().find(|e| e.name() == "..").unwrap();

        assert_eq!(dotdot.inode().inner(), parent.inner());
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

        assert_eq!(after_one.size(), initial.size() + entry_size);
        assert_eq!(after_two.size(), initial.size() + 2 * entry_size);
    }

    #[test]
    fn directory_entry_capacity_limit() {
        let mut fs = make_fs();

        let cap = fs.mkdir("/cap").unwrap();
        let max_entries_for_inode = 16 * DIR_ENTRY_PER_BLOCK;
        let full_size = (max_entries_for_inode * core::mem::size_of::<DirEntry>()) as u32;

        let mut inode = fs.inode_cache.get_mut(cap, &mut fs.block_device);

        inode.set_size(full_size);
        for block in inode.blocks_mut().filter(|b| b.is_empty()) {
            *block = DataBlockIndex::from_raw_unchecked(1);
        }

        fs.inode_cache
            .get_mut(cap, &mut fs.block_device)
            .set_size(full_size);

        let overflow = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            fs.create_file("/cap/overflow")
        }));

        assert!(
            overflow.is_ok(),
            "create_file should return Err(INodeBlocksExhausted), not panic"
        );
        assert_eq!(overflow.unwrap().err(), Some(Error::INodeBlocksExhausted));
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
        fs.write_to_file("/dir/notes.txt", b"persisted").unwrap();
        fs.flush();

        let mut fs = remount(fs);
        let dir = find_entry_inode(&mut fs, INodeIndex::new(0), "dir").expect("dir missing");
        let note = find_entry_inode(&mut fs, dir, "notes.txt").expect("notes missing");

        assert_eq!(note.inner(), file.inner());
        assert_eq!(fs.read_file("/dir/notes.txt").unwrap(), "persisted");
    }

    #[test]
    fn flush_persists_bitmaps_for_future_allocations() {
        let mut fs = make_fs();

        let first = fs.create_file("/one").unwrap();
        fs.write_to_file("/one", b"hello").unwrap();
        let first_block = first_data_block(&inode_copy(&mut fs, first)).unwrap();
        fs.flush();

        let mut fs = remount(fs);
        let second = fs.create_file("/two").unwrap();
        fs.write_to_file("/two", b"world").unwrap();

        let second_block = first_data_block(&inode_copy(&mut fs, second)).unwrap();

        assert!(second.inner() > first.inner());
        assert_ne!(second_block, first_block);
    }

    #[test]
    fn flush_is_idempotent_without_mutations() {
        let mut fs = make_fs();

        fs.mkdir("/idempotent").unwrap();
        fs.create_file("/idempotent/file.txt").unwrap();
        fs.write_to_file("/idempotent/file.txt", b"hello").unwrap();

        fs.flush();
        let after_first_flush = fs.block_device.data.borrow().clone();

        fs.flush();
        let after_second_flush = fs.block_device.data.borrow().clone();

        assert_eq!(after_first_flush, after_second_flush);
    }

    #[test]
    fn remount_multiple_times_preserves_state() {
        let mut fs = make_fs();

        fs.create_file("/loop.txt").unwrap();
        fs.write_to_file("/loop.txt", b"stable").unwrap();
        fs.flush();

        for _ in 0..3 {
            fs = remount(fs);
            assert_eq!(fs.read_file("/loop.txt").unwrap(), "stable");
            fs.flush();
        }
    }

    #[test]
    fn append_after_remount_preserves_and_extends() {
        let mut fs = make_fs();

        fs.create_file("/append-remount.txt").unwrap();
        fs.write_to_file("/append-remount.txt", b"abc").unwrap();
        fs.flush();

        let mut fs = remount(fs);
        fs.write_to_file("/append-remount.txt", b"def").unwrap();
        assert_eq!(fs.read_file("/append-remount.txt").unwrap(), "abcdef");
        fs.flush();

        let mut fs = remount(fs);
        assert_eq!(fs.read_file("/append-remount.txt").unwrap(), "abcdef");
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

        fs.create_file("/data-oom.txt").unwrap();

        let bits = bitmap_capacity_bits(&fs.data_bitmap);
        for idx in 0..bits {
            fs.data_bitmap.set(idx);
        }

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            fs.write_to_file("/data-oom.txt", b"x")
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

        inode.set_size(full_size);
        for block in inode.blocks_mut().filter(|b| b.is_empty()) {
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
        assert_eq!(result.unwrap().err(), Some(Error::INodeBlocksExhausted));
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

        fs.write_to_file("/first.txt", b"aaa").unwrap();
        fs.write_to_file("/second.txt", b"bbb").unwrap();

        let first_block = first_data_block(&inode_copy(&mut fs, first)).unwrap();
        let second_block = first_data_block(&inode_copy(&mut fs, second)).unwrap();

        assert_ne!(first_block, second_block);
    }

    #[test]
    fn writing_multiple_blocks() {
        let mut fs = make_fs();

        let content = "A".repeat(MAX_FILE_SIZE);
        fs.create_file("/test.txt").unwrap();
        fs.write_to_file("/test.txt", content.as_bytes()).unwrap();

        let res = fs.write_to_file("/test.txt", b"overflow");
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

        fs.mkdir("/test").unwrap();
        let res = fs.write_to_file("/test", b"xd");
        assert_eq!(res.err(), Some(Error::NotAFile));
    }

    #[test]
    fn mkdir_into_file_returns_error() {
        let mut fs = make_fs();

        fs.create_file("/test.txt").unwrap();
        let res = fs.mkdir("/test.txt/huh");
        assert_eq!(res.err(), Some(Error::NotADirectory));
    }

    // --- remove() tests ---

    #[test]
    fn remove_file_from_root() {
        let mut fs = make_fs();

        fs.create_file("/file.txt").unwrap();
        fs.remove_dir_entry("/file.txt").unwrap();

        let names: Vec<_> = fs
            .read_dir_entry(INodeIndex::new(0))
            .into_iter()
            .map(|e| e.name())
            .collect();
        assert!(!names.contains(&"file.txt".to_string()));
    }

    #[test]
    fn remove_file_from_nested_dir() {
        let mut fs = make_fs();

        fs.mkdir("/a").unwrap();
        fs.mkdir("/a/b").unwrap();
        fs.create_file("/a/b/file.txt").unwrap();

        fs.remove_dir_entry("/a/b/file.txt").unwrap();

        let b_inode = find_entry_inode(&mut fs, INodeIndex::new(0), "a")
            .and_then(|a| find_entry_inode(&mut fs, a, "b"))
            .unwrap();

        let names: Vec<_> = fs
            .read_dir_entry(b_inode)
            .into_iter()
            .map(|e| e.name())
            .collect();
        assert!(!names.contains(&"file.txt".to_string()));
    }

    #[test]
    fn remove_dir_from_root() {
        let mut fs = make_fs();

        fs.mkdir("/emptydir").unwrap();
        let res = fs.remove_dir_entry("/emptydir");
        assert_eq!(res.err(), Some(Error::Unsupported));
    }

    #[test]
    fn remove_frees_inode_bitmap_bit() {
        let mut fs = make_fs();

        let idx = fs.create_file("/tracked.txt").unwrap();
        assert!(fs.inode_bitmap.is_set(idx.inner()));

        fs.remove_dir_entry("/tracked.txt").unwrap();

        assert!(!fs.inode_bitmap.is_set(idx.inner()));
    }

    #[test]
    fn remove_frees_data_blocks() {
        let mut fs = make_fs();

        fs.create_file("/data.txt").unwrap();
        fs.write_to_file("/data.txt", b"some content that occupies a block")
            .unwrap();

        let before = bitmap_set_count(&fs.data_bitmap);
        fs.remove_dir_entry("/data.txt").unwrap();
        let after = bitmap_set_count(&fs.data_bitmap);

        assert!(
            after < before,
            "data bitmap should have fewer bits set after remove (before={before}, after={after})"
        );
    }

    #[test]
    fn remove_allows_name_reuse() {
        let mut fs = make_fs();

        fs.create_file("/reuse.txt").unwrap();
        fs.remove_dir_entry("/reuse.txt").unwrap();

        let res = fs.create_file("/reuse.txt");
        assert!(res.is_ok(), "name reuse after remove should succeed");
    }

    #[test]
    fn remove_and_remount_does_not_see_entry() {
        let mut fs = make_fs();

        fs.create_file("/gone.txt").unwrap();
        fs.remove_dir_entry("/gone.txt").unwrap();
        fs.flush();

        let mut fs = remount(fs);
        let names: Vec<_> = fs
            .read_dir_entry(INodeIndex::new(0))
            .into_iter()
            .map(|e| e.name())
            .collect();
        assert!(!names.contains(&"gone.txt".to_string()));
    }

    #[test]
    fn remove_nonexistent_returns_not_found() {
        let mut fs = make_fs();

        let res = fs.remove_dir_entry("/nope.txt");
        assert_eq!(res.err(), Some(Error::NotFound));
    }

    #[test]
    fn remove_in_missing_parent_returns_directory_does_not_exist() {
        let mut fs = make_fs();

        let res = fs.remove_dir_entry("/nope/file.txt");
        assert_eq!(res.err(), Some(Error::NotFound));
    }

    #[test]
    fn remove_with_empty_path_returns_error() {
        let mut fs = make_fs();

        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| fs.remove_dir_entry("")));

        assert!(result.is_ok(), "empty path should not panic");
        let inner = result.unwrap();
        assert!(inner.is_err(), "empty path should return an error");
        assert_eq!(inner.err(), Some(Error::NotFound));
    }

    #[test]
    fn remove_root_path_returns_error() {
        let mut fs = make_fs();

        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| fs.remove_dir_entry("/")));

        assert!(result.is_ok(), "removing '/' should not panic");
        assert!(
            result.unwrap().is_err(),
            "removing '/' should return an error"
        );
    }

    #[test]
    fn remove_through_file_as_component_returns_not_a_directory() {
        let mut fs = make_fs();

        fs.create_file("/file.txt").unwrap();
        let res = fs.remove_dir_entry("/file.txt/thing");

        assert_eq!(res.err(), Some(Error::NotADirectory));
    }

    #[test]
    fn remove_nonempty_directory_returns_not_empty() {
        let mut fs = make_fs();

        fs.mkdir("/a").unwrap();
        fs.create_file("/a/child.txt").unwrap();

        let res = fs.remove_dir_entry("/a");
        assert_eq!(res.err(), Some(Error::Unsupported));
    }

    #[test]
    fn remove_does_not_affect_siblings() {
        let mut fs = make_fs();

        fs.create_file("/a").unwrap();
        fs.create_file("/b").unwrap();

        fs.remove_dir_entry("/a").unwrap();

        let names: Vec<_> = fs
            .read_dir_entry(INodeIndex::new(0))
            .into_iter()
            .map(|e| e.name())
            .collect();
        assert!(
            names.contains(&"b".to_string()),
            "/b should still be present"
        );
        assert!(!names.contains(&"a".to_string()), "/a should be gone");
    }

    #[test]
    fn remove_dot_entry_is_rejected() {
        let mut fs = make_fs();

        fs.mkdir("/dir").unwrap();

        let dot = fs.remove_dir_entry("/dir/.");
        let dotdot = fs.remove_dir_entry("/dir/..");

        assert!(dot.is_err(), "removing '.' should be rejected");
        assert!(dotdot.is_err(), "removing '..' should be rejected");
    }

    #[test]
    fn parent_dir_size_decrements_after_remove() {
        let mut fs = make_fs();

        fs.create_file("/sized.txt").unwrap();
        let root_before = inode_copy(&mut fs, INodeIndex::new(0));

        fs.remove_dir_entry("/sized.txt").unwrap();
        let root_after = inode_copy(&mut fs, INodeIndex::new(0));

        let entry_size = core::mem::size_of::<DirEntry>() as u32;
        assert_eq!(
            root_after.size(),
            root_before.size() - entry_size,
            "parent inode size should decrease by one DirEntry after remove"
        );
    }

    #[test]
    fn remove_then_readd_same_name_can_reuse_inode() {
        let mut fs = make_fs();

        let old_idx = fs.create_file("/fresh.txt").unwrap();
        fs.remove_dir_entry("/fresh.txt").unwrap();
        let new_idx = fs.create_file("/fresh.txt").unwrap();

        assert_eq!(
            old_idx.inner(),
            new_idx.inner(),
            "re-added entry should be able to reuse the freed inode index"
        );
    }
}
