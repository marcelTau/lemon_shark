#![cfg_attr(not(test), no_std)]
extern crate alloc;

mod bytereader;
mod dir_entry;
mod filesystem;
mod inode;
mod inode_cache;
mod layout;

use crate::bytereader::DiskFormat;
pub use crate::layout::{BlockIndex, INodeIndex};
use bitmap::Bitmap;
pub use filesystem::{BLOCK_SIZE, BlockDevice, Error, Filesystem};

pub(crate) use filesystem::{INODES_PER_BLOCK, MAX_INODES};
pub(crate) use inode::INode;

impl DiskFormat for Bitmap {
    fn write_to<'a>(&self, writer: &'a mut bytereader::ByteWriter) {
        let words = self.words();
        writer.write_u32(words.len() as u32);
        for &word in words {
            writer.write_u32(word);
        }
    }

    fn read_from<'a>(reader: &'a mut bytereader::ByteReader) -> Self {
        let word_count = reader.read_u32();
        let arr = (0..word_count).map(|_| reader.read_u32()).collect();

        Bitmap::from_raw(arr)
    }
}
