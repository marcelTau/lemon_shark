use crate::{dir_entry::DirEntry, layout::DataBlockIndex, BLOCK_SIZE};

use core::mem;

/// Hardcoded number of blocks that an INode can hold.
pub(crate) const INODE_BLOCKS: usize = 16;

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
    blocks: [DataBlockIndex; INODE_BLOCKS],

    /// Flag indicating if this is a directory.
    is_directory: bool,
}

pub(crate) struct WriteSlot<'a> {
    pub block: Option<&'a mut DataBlockIndex>,
    pub byte_offset: usize,
    pub capacity: usize,
}

impl INode {
    pub(crate) fn new_empty_directory() -> Self {
        INode {
            size: 0,
            is_directory: true,
            blocks: core::array::from_fn(|_| Default::default()),
        }
    }

    pub(crate) fn write_slot(&mut self) -> WriteSlot<'_> {
        let slot = self.size as usize / BLOCK_SIZE;
        let offset = self.size as usize % BLOCK_SIZE;

        WriteSlot {
            block: self.blocks.get_mut(slot),
            byte_offset: offset,
            capacity: BLOCK_SIZE - offset,
        }
    }

    pub(crate) fn new_empty_file() -> Self {
        INode {
            size: 0,
            is_directory: false,
            blocks: core::array::from_fn(|_| Default::default()),
        }
    }

    pub(crate) fn size(&self) -> u32 {
        self.size
    }

    #[allow(unused)]
    #[cfg(test)]
    pub(crate) fn set_size(&mut self, size: u32) {
        self.size = size;
    }

    #[allow(unused)]
    #[cfg(test)]
    pub(crate) fn blocks_mut(&mut self) -> impl Iterator<Item = &mut DataBlockIndex> {
        self.blocks.iter_mut()
    }

    pub(crate) fn advance(&mut self, by: usize) {
        self.size += by as u32;
    }

    pub(crate) fn shrink(&mut self, by: usize) {
        self.size -= by as u32;
    }

    /// SAFETY: This function assumes that the `INode` is a directory and that the
    /// `size` field is accurate.
    pub(crate) unsafe fn current_dir_entries(&self) -> usize {
        assert!(self.is_directory);
        self.size as usize / mem::size_of::<DirEntry>()
    }

    pub(crate) fn has_space(&self) -> bool {
        self.blocks.iter().any(DataBlockIndex::is_empty)
    }

    pub(crate) fn used_blocks(&self) -> impl Iterator<Item = DataBlockIndex> + '_ {
        self.blocks.iter().copied().filter(|b| !b.is_empty())
    }

    pub(crate) fn block(&self, block_index: usize) -> DataBlockIndex {
        self.blocks.get(block_index).copied().unwrap()
    }

    pub(crate) fn block_mut(&mut self, block_index: usize) -> Option<&mut DataBlockIndex> {
        self.blocks.get_mut(block_index)
    }

    pub(crate) fn from_bytes(bytes: &[u8]) -> Self {
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

    pub(crate) fn to_bytes(self) -> [u8; mem::size_of::<INode>()] {
        let mut bytes = [0u8; mem::size_of::<INode>()];

        bytes[0..4].copy_from_slice(&self.size.to_le_bytes());
        let current_offset = 4;
        for i in 0..16 {
            let start = current_offset + (i * 4);
            let value = self.blocks[i]
                .to_block()
                .map(|b| b.inner())
                .unwrap_or_default();
            bytes[start..start + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes[68] = if self.is_directory { 1 } else { 0 };

        bytes
    }

    pub(crate) fn is_directory(&self) -> bool {
        self.is_directory
    }
}
