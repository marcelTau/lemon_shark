use core::mem;
use core::num::NonZeroU32;

use crate::{BLOCK_SIZE, INODES_PER_BLOCK, INode};

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
    pub(crate) fn new(base: usize, val: usize) -> Self {
        Self(NonZeroU32::new(
            (base + val)
                .try_into()
                .expect("validated filesystem blocks fit in u32"),
        ))
    }

    pub(crate) fn from_raw_unchecked(val: u32) -> Self {
        Self(NonZeroU32::new(val))
    }

    pub(crate) fn to_block(self) -> Option<BlockIndex> {
        self.0.map(|v| BlockIndex(v.get()))
    }

    pub(crate) fn bitmap_index(&self, layout: &Layout) -> usize {
        self.0.unwrap().get() as usize - layout.data_start
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_none()
    }

    pub(crate) fn clear(&mut self) {
        self.0 = None;
    }
}

/// Describes a `LemonShark` filesystems representation on disk.
///
/// Terminology:
/// Block: 512 bytes on disk
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
    pub(crate) fn new(total_blocks: usize, inode_count: usize) -> Option<Self> {
        const SUPERBLOCK_BLOCKS: usize = 1;
        const BITS_PER_BLOCK: usize = BLOCK_SIZE * 8;

        let inode_bitmap_blocks = inode_count.div_ceil(BITS_PER_BLOCK);
        let inode_table_blocks = inode_count.div_ceil(INODES_PER_BLOCK);

        let fixed = SUPERBLOCK_BLOCKS
            .checked_add(inode_bitmap_blocks)?
            .checked_add(inode_table_blocks)?;
        let remaining = total_blocks.checked_sub(fixed)?;

        let mut data_bitmap_blocks = remaining.div_ceil(BITS_PER_BLOCK);

        let (data_start, data_blocks) = loop {
            let data_start = fixed.checked_add(data_bitmap_blocks)?;
            let data_blocks = total_blocks.checked_sub(data_start)?;

            if data_blocks == 0 {
                return None;
            }

            let next = data_blocks.div_ceil(BITS_PER_BLOCK);

            if next == data_bitmap_blocks {
                break (data_start, data_blocks);
            }

            data_bitmap_blocks = next;
        };

        Some(Self {
            inode_bitmap_start: 1,
            inode_bitmap_blocks,
            data_bitmap_start: 1 + inode_bitmap_blocks,
            data_bitmap_blocks,
            inode_table_start: 1 + inode_bitmap_blocks + data_bitmap_blocks,
            inode_table_blocks,
            data_start,
            data_blocks,
        })
    }

    pub(crate) fn inode_to_block(&self, inode: INodeIndex) -> (BlockIndex, ByteOffset) {
        let block_index =
            BlockIndex(self.inode_table_start as u32 + (inode.0 / INODES_PER_BLOCK as u32));

        let offset =
            ByteOffset((inode.0 % INODES_PER_BLOCK as u32) * mem::size_of::<INode>() as u32);

        (block_index, offset)
    }

    pub(crate) fn data_block(&self, val: usize) -> DataBlockIndex {
        DataBlockIndex::new(self.data_start, val)
    }
}
