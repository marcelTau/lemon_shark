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
use crate::virtio2::LockedBlockDevice;
use crate::{logln, print, println, ramdisk};
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::cell::UnsafeCell;
use core::fmt::Debug;
use core::mem;
use core::sync::atomic::{AtomicBool, Ordering};
pub(crate) use layout::{BlockIndex, DataBlockIndex, INodeIndex, Layout};

const DIR_ENTRY_PER_BLOCK: usize = BLOCK_SIZE / core::mem::size_of::<DirEntry>();
// const INODE_BLOCKS: usize = 10;
// const INODE_START: usize = 1;
// const INODES_PER_BLOCK: usize = BLOCK_SIZE / core::mem::size_of::<INode>();
// const DATA_START: usize = INODE_START + INODE_BLOCKS + 1;
// const INODE_BITMAP_SIZE: usize = (INODE_BLOCKS * INODES_PER_BLOCK) / 32;

// ----------------------------------------------------------------------------
/// BlockSize of the Filesystem.
pub const BLOCK_SIZE: usize = 512;

/// Number of INodes per block.
const INODES_PER_BLOCK: usize = BLOCK_SIZE / core::mem::size_of::<INode>();

/// Max number of INodes supported by the Filesystem
const MAX_INODES: usize = 4096;

/// Magic value written to the start of the block device.
const MAGIC: u64 = 0x4e4f4d454c; // lemon (le)

pub(crate) mod layout {
    use core::mem;
    use core::num::NonZeroU32;

    use crate::filesystem::{INode, BLOCK_SIZE, MAX_INODES};

    use super::INODES_PER_BLOCK;

    /// An Index into the blocks used for the `ramdisk`.
    #[derive(Debug, Clone, Copy)]
    pub(crate) struct BlockIndex(u32);

    impl BlockIndex {
        /// This function should not be used in normal code.
        ///
        /// `BlockIndex` should only ever be created by the Layout other than in tests or
        /// debugging.
        pub(crate) fn from_raw(val: u32) -> Self {
            Self(val)
        }

        pub(crate) fn inner(&self) -> u32 {
            self.0
        }
    }

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
    pub struct INodeIndex(u32);

    impl INodeIndex {
        pub(crate) fn new(val: u32) -> Self {
            INodeIndex(val)
        }

        pub(crate) fn inner(&self) -> u32 {
            self.0
        }
    }

    /// `DataBlockIndex` is an index into the blocks of the ramdisk but is restricted to
    /// indexes into the data segment. This is used to enforce this invariant in the
    /// typesystem.
    #[derive(Clone, Copy, Debug, Default)]
    #[repr(transparent)]
    pub(crate) struct DataBlockIndex(Option<NonZeroU32>);
    impl DataBlockIndex {
        /// Creates a new `DataBlockIndex` from an index into the data segment.
        pub(crate) fn new(base: u32, val: u32) -> Self {
            Self(NonZeroU32::new(base + val))
        }

        pub(crate) fn from_raw_unchecked(val: u32) -> Self {
            Self(NonZeroU32::new(val))
        }

        pub(crate) fn value(&self) -> Option<u32> {
            self.0.map(|v| v.get())
        }

        pub(crate) fn is_none(&self) -> bool {
            self.0.is_none()
        }
    }

    #[derive(Debug, PartialEq, Clone, Copy)]
    pub(crate) struct Layout {
        pub(crate) inode_bitmap_start: usize,
        pub(crate) inode_bitmap_blocks: usize,
        pub(crate) data_bitmap_start: usize,
        pub(crate) data_bitmap_blocks: usize,
        pub(crate) inode_table_start: usize,
        pub(crate) inode_table_blocks: usize,
        pub(crate) data_start: usize,
        pub(crate) data_blocks: usize,
    }

    impl Layout {
        /// Layout of the filesystem on disk:
        ///
        /// +--------------+
        /// | Superblock   |
        /// +--------------+
        /// | INodeBitmap  |
        /// | ...          |
        /// +--------------+
        /// | DataBitmap   |
        /// | ...          |
        /// +--------------+
        /// | INode blocks |
        /// | ...          |
        /// +--------------+
        /// | Data blocks  |
        /// | ...          |
        /// +--------------+
        pub(crate) fn new(total_blocks: u32) -> Self {
            const SUPERBLOCK_BLOCKS: usize = 1;
            const BITS_PER_BLOCK: usize = BLOCK_SIZE * 8;

            const INODES_PER_BLOCK: usize = BLOCK_SIZE / mem::size_of::<INode>();
            const INODE_BITMAP_BLOCKS: usize = MAX_INODES.div_ceil(BITS_PER_BLOCK);
            const INODE_BLOCKS: usize = MAX_INODES.div_ceil(INODES_PER_BLOCK);

            let fixed = SUPERBLOCK_BLOCKS + INODE_BITMAP_BLOCKS + INODE_BLOCKS;

            let mut data_bitmap_blocks = (total_blocks as usize - fixed).div_ceil(BITS_PER_BLOCK);

            let (data_start, data_blocks) = loop {
                let data_start = fixed + data_bitmap_blocks;
                let data_blocks = total_blocks as usize - data_start;

                let next = data_blocks.div_ceil(BITS_PER_BLOCK);

                if next == data_bitmap_blocks {
                    break (data_start, data_blocks);
                }

                data_bitmap_blocks = next;
            };

            Self {
                inode_bitmap_start: 1,
                inode_bitmap_blocks: INODE_BITMAP_BLOCKS,
                data_bitmap_start: 1 + INODE_BITMAP_BLOCKS,
                data_bitmap_blocks,
                inode_table_start: 1 + INODE_BITMAP_BLOCKS + data_bitmap_blocks,
                inode_table_blocks: INODE_BLOCKS,
                data_start,
                data_blocks,
            }
        }

        pub(crate) fn inode_to_block(&self, inode: INodeIndex) -> (BlockIndex, ByteOffset) {
            let block_index =
                BlockIndex(self.inode_table_start as u32 + (inode.0 / INODES_PER_BLOCK as u32));

            let offset =
                ByteOffset((inode.0 % INODES_PER_BLOCK as u32) * mem::size_of::<INode>() as u32);

            (block_index, offset)
        }

        pub(crate) fn data_block(&self, val: u32) -> DataBlockIndex {
            DataBlockIndex::new(self.data_start as u32, val)
        }

        pub(crate) fn data_to_block(&self, data: DataBlockIndex) -> BlockIndex {
            BlockIndex(data.value().unwrap())
        }
    }
}

/// Encapsulates the different `BlockDevice` types that we support.
#[allow(clippy::large_enum_variant)]
pub enum BlockDevice {
    /// Internal `ramdisk` used for testing mostly.
    Ramdisk,

    /// VirtIO block device for persistent storage.
    VirtIO(LockedBlockDevice<'static>),
}

impl BlockDevice {
    /// Reads a block from the disk.
    fn read_block(&mut self, block_idx: BlockIndex, buf: &mut [u8]) {
        match self {
            BlockDevice::Ramdisk => ramdisk::read_block(block_idx, buf),
            BlockDevice::VirtIO(blk) => blk.read_block(block_idx, buf),
        }
    }

    /// Writes a block to the disk.
    fn write_block(&mut self, block_idx: BlockIndex, data: &[u8]) {
        match self {
            BlockDevice::Ramdisk => ramdisk::write_block(block_idx, data),
            BlockDevice::VirtIO(blk) => blk.write_block(block_idx, data),
        }
    }

    /// Get's the total number of blocks on this device.
    fn total_blocks(&mut self) -> usize {
        match self {
            BlockDevice::Ramdisk => ramdisk::total_blocks(),
            BlockDevice::VirtIO(blk) => blk.total_blocks(),
        }
    }

    /// Complete memory dump of all used pages. Useful for debugging.
    fn dump_non_empty_pages(&mut self) {
        const PAGE_BYTES: usize = 4096;
        const BYTES_PER_ROW: usize = 16;
        const BLOCKS_PER_PAGE: usize = PAGE_BYTES / BLOCK_SIZE;

        let mut page = [0u8; PAGE_BYTES];
        let device_name = match self {
            BlockDevice::Ramdisk => "RAMDISK",
            BlockDevice::VirtIO(_) => "VIRTIO",
        };
        let total_blocks = self.total_blocks();
        let total_pages = total_blocks.div_ceil(BLOCKS_PER_PAGE);

        println!(
            "=== {} DUMP ({} blocks, {} pages, {} bytes total) ===",
            device_name,
            total_blocks,
            total_pages,
            total_blocks * BLOCK_SIZE
        );

        for page_idx in 0..total_pages {
            let first_block = page_idx * BLOCKS_PER_PAGE;
            let blocks_in_page = (total_blocks - first_block).min(BLOCKS_PER_PAGE);
            let bytes_in_page = blocks_in_page * BLOCK_SIZE;

            for block_offset in 0..blocks_in_page {
                let offset = block_offset * BLOCK_SIZE;
                self.read_block(
                    BlockIndex::from_raw((first_block + block_offset) as u32),
                    &mut page[offset..offset + BLOCK_SIZE],
                );
            }

            if page[..bytes_in_page].iter().all(|&byte| byte == 0) {
                continue;
            }

            let page_offset = first_block * BLOCK_SIZE;
            println!("\n--- Page {page_idx} (block {first_block}, offset 0x{page_offset:08x}) ---");

            for row in (0..bytes_in_page).step_by(BYTES_PER_ROW) {
                let addr = page_offset + row;
                print!("{addr:08x}  ");

                for i in 0..BYTES_PER_ROW {
                    if row + i < bytes_in_page {
                        print!("{:02X} ", page[row + i]);
                    } else {
                        print!("   ");
                    }

                    if i == 7 {
                        print!(" ");
                    }
                }

                print!(" |");
                for i in 0..BYTES_PER_ROW {
                    if row + i >= bytes_in_page {
                        print!(" ");
                        continue;
                    }

                    let byte = page[row + i];
                    if (0x20..0x7f).contains(&byte) {
                        print!("{}", byte as char);
                    } else {
                        print!(".");
                    }
                }
                println!("|");
            }
        }

        println!("\n=== END DUMP ===");
    }
}

/// Filesystem Errors
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

/// The `INode` contains metadata about a file.
///
/// Memory layout:
/// `size`          4 bytes
/// `blocks`        64 bytes
/// `is_directory`  1 byte
/// `padding`       3 bytes
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
        String::from_utf8(self.name.to_vec()).unwrap()
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
pub struct SuperBlock {
    magic: u64,
    block_size: u32,
    total_blocks: u32,
}

impl SuperBlock {
    pub fn default_superblock(block_device: &mut BlockDevice) -> Self {
        let total_blocks = block_device.total_blocks() as u32;

        SuperBlock {
            magic: MAGIC,
            block_size: BLOCK_SIZE as u32,
            total_blocks,
        }
    }

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

    fn dump(&mut self) {
        self.inner.get_mut().as_mut().unwrap().dump();
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

    fn tree(&mut self) {
        self.inner.get_mut().as_mut().unwrap().tree();
    }

    fn flush(&mut self) {
        self.inner.get_mut().as_mut().unwrap().flush();
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
        println!("INodeCache size={size}");
        Self {
            inodes: Vec::new(),
            layout: Some(layout),
            dirty: Bitmap::new(size as u32),
        }
    }

    /// Reads the `INode` from disk if it's not already in the cache.
    fn read_from_disk(&self, index: INodeIndex, device: &mut BlockDevice) -> INode {
        let mut buf = [0u8; BLOCK_SIZE];
        let (block_index, byte_offset) = self.layout.as_ref().unwrap().inode_to_block(index);
        device.read_block(block_index, &mut buf);
        INode::from_bytes(&buf[byte_offset.range::<INode>()])
    }

    /// Get a `&INode` from the cache, fetching it from disk when not present.
    pub fn get(&mut self, index: INodeIndex, device: &mut BlockDevice) -> &INode {
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
    pub fn get_mut(&mut self, index: INodeIndex, device: &mut BlockDevice) -> &mut INode {
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
pub struct Filesystem {
    block_device: BlockDevice,
    inode_bitmap: Bitmap,
    data_bitmap: Bitmap,
    inode_cache: INodeCache,
    layout: Layout,
}

impl Filesystem {
    /// Reads the superblock from block_index 0
    fn read_superblock(block_device: &mut BlockDevice) -> (SuperBlock, bool) {
        let mut buf = [0u8; BLOCK_SIZE];
        block_device.read_block(BlockIndex::from_raw(0), &mut buf);
        let sb = SuperBlock::from_bytes(&buf[0..mem::size_of::<SuperBlock>()]);

        match sb.magic {
            0 => {
                println!("[FS] Empty disk. Creating new superblock");
                (SuperBlock::default_superblock(block_device), false)
            }
            MAGIC => {
                println!("[FS] Found superblock on disk: {sb:?}");
                (sb, true)
            }
            _ => panic!("Disk has wrong format"),
        }
    }

    fn new_with_device(mut block_device: BlockDevice) -> Self {
        let (sb, is_initialized) = Self::read_superblock(&mut block_device);

        let layout = Layout::new(sb.total_blocks);

        println!("[FS] Generated layout: {layout:?}");

        println!(
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

        println!(
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
            println!("Reading inode_bitmap");
            let inode_bitmap = Bitmap::from_bytes(&inode_bitmap_raw);
            println!("Reading data_bitmap");
            let data_bitmap = Bitmap::from_bytes(&data_bitmap_raw);

            (inode_bitmap, data_bitmap)
        } else {
            (
                Bitmap::new((BLOCK_SIZE * layout.inode_bitmap_blocks) as u32),
                Bitmap::new((BLOCK_SIZE * layout.data_bitmap_blocks) as u32),
            )
        };

        Self {
            inode_bitmap,
            data_bitmap,
            inode_cache: INodeCache::new(layout.clone()),
            block_device,
            layout,
        }
    }

    // TODO(mt): how does this work? For tests just use ramdisk? Do we enforce that?
    pub fn reset(&mut self) {
        todo!();
        // *self = Filesystem::new();
        // ramdisk::reset();

        // self.create_empty_root();
    }

    /// Writes an `INode` to `ramdisk`.
    fn write_inode_to_disk(&mut self, inode_index: INodeIndex, inode: &INode) {
        // Creating the buffer to write the INode to.
        let mut buf = [0u8; BLOCK_SIZE];

        // Calculate the `BlockIndex` and the `ByteOffset` for the `INode`
        // to be written to.
        let (block_index, byte_offset) = self.layout.inode_to_block(inode_index);

        // Reading the block into `buf` to append the `INode` to it.
        self.block_device.read_block(block_index, &mut buf);

        logln!("[FS] Writing to block index {block_index:?} at byte_offset={byte_offset:?}");

        buf[byte_offset.range::<INode>()].copy_from_slice(inode.to_bytes().as_slice());

        // Writing the block to memory.
        self.block_device.write_block(block_index, &buf);
    }

    /// Writes a new `INode` to disk.
    fn new_inode(&mut self, inode: &INode) -> INodeIndex {
        // Finds the next free block in the `INodeBitmap`.
        let free = INodeIndex::new(self.inode_bitmap.find_free().unwrap());

        logln!("[FS] Writing INode to {free:?} in {:?}", self.inode_bitmap);

        // Set this block to be used.
        self.inode_bitmap.set(free.inner());

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
        let mut current_index = INodeIndex::new(0);
        let mut prev_index = INodeIndex::new(0);

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
    fn write_dir_entry(&mut self, entry: DirEntry, inode_index: INodeIndex) -> Result<(), Error> {
        let mut buf = [0u8; BLOCK_SIZE];

        let inode = self
            .inode_cache
            .get_mut(inode_index, &mut self.block_device);

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
                let free = self.data_bitmap.find_free().unwrap();
                let free_block_index = self.layout.data_block(free);
                inode.blocks[last_used_block_index] = free_block_index;
                self.data_bitmap.set(free);
            }

            let data_block_index = inode.blocks[last_used_block_index];
            let byte_offset = inode.size % BLOCK_SIZE as u32;

            let bytes_to_write = total_bytes.min(BLOCK_SIZE - byte_offset as usize);
            logln!("[FS] Writing {}/{} bytes", bytes_to_write, total_bytes);

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

    pub(crate) fn read_file(&mut self, inode_index: INodeIndex) -> String {
        let inode = self
            .inode_cache
            .get_mut(inode_index, &mut self.block_device);

        if inode.is_directory {
            panic!("Can't write to directory");
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
    pub fn write_superblock(&mut self, superblock: &SuperBlock) {
        let mut buf = [0u8; BLOCK_SIZE];
        buf[0..mem::size_of::<SuperBlock>()].copy_from_slice(&superblock.to_bytes());
        self.block_device.write_block(BlockIndex::from_raw(0), &buf);
    }

    fn dump_dir(&mut self, index: u32) {
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
                    println!("\t{entry:?}");
                }
            }
        }
    }

    fn tree(&mut self) {
        fn inner(fs: &mut Filesystem, entry: &DirEntry, indent: u8) {
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
                println!("{}{}", " ".repeat(indent as usize), entry.name());
            }

            for entry in entries.iter().filter(|e| !e.name().starts_with('.')) {
                inner(fs, entry, indent + 2);
                if !fs
                    .inode_cache
                    .get(entry.inode, &mut fs.block_device)
                    .is_directory
                {
                    println!("{}{}", " ".repeat(indent as usize + 2), entry.name());
                }
            }
        }

        let root_entries = self.read_dir_entry(INodeIndex::new(0));

        println!("root_entries={root_entries:?}");

        for root_entry in root_entries.iter().filter(|e| !e.name().starts_with('.')) {
            inner(self, root_entry, 0);
        }
    }

    fn dump(&mut self) {
        self.block_device.dump_non_empty_pages();
    }

    fn flush(&mut self) {
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

        logln!("[FS] Flushed");
    }
}

/// Those functions are wrappers around the `LockedFilesystem` for the shell
/// to do some filesystem operations.
///
/// This is the only place where the `.lock()` should be called to avoid
/// deadlocks.
pub mod api {
    use super::*;

    pub fn dump() {
        (*FS.lock()).dump();
    }

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
        (*FS.lock()).write_to_file(INodeIndex::new(inode_index as u32), text.as_bytes())
    }

    pub fn read_file(inode_index: usize) -> String {
        (*FS.lock()).read_file(INodeIndex::new(inode_index as u32))
    }

    pub fn reset() {
        (*FS.lock()).reset();
    }

    pub fn tree() {
        (*FS.lock()).tree();
    }

    pub fn flush() {
        (*FS.lock()).flush();
    }
}

/// Initializes the Filesystem by reading the superblock or defaulting it
/// if it doesn't exist.
pub fn init_with_device(dev: BlockDevice) {
    // Guard against re-initializing the Filesystem by setting the atomic
    // flag.
    if FS_INITIALIZED
        .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
        .is_err()
    {
        logln!("[FS] Not initializing the Filesystem again!");
        return;
    }

    let fs = Filesystem::new_with_device(dev);

    (*FS.lock()).init(fs);
    (*FS.lock()).create_empty_root();

    // api::mkdir("/foo").unwrap();
    // api::mkdir("/foo/test").unwrap();
    // api::create_file("/foo/file.txt").unwrap();
    // api::mkdir("/foo/test/deep").unwrap();
    // api::mkdir("/foo/test/deep/deep2").unwrap();
    // api::create_file("/foo/test/deep/deep2/xxfile.txt").unwrap();
    // api::create_file("/foo/test/deep/deep-file.txt").unwrap();
    // api::create_file("/foo/test1.txt").unwrap();
    // api::create_file("/foo/test2.txt").unwrap();
    // api::create_file("/foo/test3.txt").unwrap();

    // api::dump_dir(1);

    logln!("[FS] Initialized");

    // (*FS.lock()).tree();
}
