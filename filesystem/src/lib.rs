#![cfg_attr(not(test), no_std)]
extern crate alloc;

mod bitmap;
mod bytereader;
mod dir_entry;
mod filesystem;
mod inode;
mod inode_cache;
mod layout;

pub use crate::layout::{BlockIndex, INodeIndex};
pub use filesystem::{BlockDevice, Error, Filesystem, BLOCK_SIZE};

pub(crate) use filesystem::{INODES_PER_BLOCK, MAX_INODES};
pub(crate) use inode::INode;
