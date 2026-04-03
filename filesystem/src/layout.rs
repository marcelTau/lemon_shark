use core::mem;
use core::num::NonZeroU32;

use crate::{BLOCK_SIZE, INODES_PER_BLOCK, INode, MAX_INODES};

/// An Index into the blocks used for the block device.
#[derive(Debug, Clone, Copy)]
pub struct BlockIndex(pub(crate) u32);

impl BlockIndex {
    /// This function should not be used in normal code.
    ///
    /// `BlockIndex` should only ever be created by the Layout other than in tests or
    /// debugging.
    pub fn from_raw(val: u32) -> Self {
        Self(val)
    }

    pub fn inner(&self) -> u32 {
        self.0
    }
}

/// A `ByteOffset` to something inside of a block.
#[derive(Debug)]
pub(crate) struct ByteOffset(pub(crate) u32);

/// This is the actual index of the INode.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct INodeIndex(pub(crate) u32);

impl INodeIndex {
    pub fn new(val: u32) -> Self {
        INodeIndex(val)
    }

    pub fn inner(&self) -> u32 {
        self.0
    }

    pub fn root() -> Self {
        INodeIndex(0)
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

    pub(crate) fn to_block(self) -> Option<BlockIndex> {
        self.0.map(|v| BlockIndex(v.get()))
    }

    pub(crate) fn bitmap_index(&self, layout: &Layout) -> u32 {
        self.0.unwrap().get() - layout.data_start as u32
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_none()
    }

    pub(crate) fn clear(&mut self) {
        self.0 = None;
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

    // pub(crate) fn data_to_block(&self, data: DataBlockIndex) -> BlockIndex {
    //     data.to_block().unwrap()
    // }
}
