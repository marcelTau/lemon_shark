#![cfg_attr(not(test), no_std)]
extern crate alloc;

mod bitmap;
mod bytereader;
mod filesystem;
mod layout;

pub use crate::layout::{BlockIndex, INodeIndex};
pub use filesystem::{BLOCK_SIZE, BlockDevice, Error, Filesystem};

pub(crate) use filesystem::{INODES_PER_BLOCK, INode, MAX_INODES};
