#![cfg_attr(not(test), no_std)]
extern crate alloc;

mod bitmap;
mod bytereader;
mod filesystem;
mod layout;

pub use crate::layout::{BlockIndex, INodeIndex};
pub use filesystem::{BLOCK_SIZE, BlockDevice, Error, Filesystem};

pub(crate) use filesystem::{INODES_PER_BLOCK, INode, MAX_INODES};

/// Logging macro: no-op when compiled as a `no_std` dependency, forwards to
/// `println!` when running tests so output is visible.
#[cfg(not(test))]
#[macro_export]
macro_rules! fs_log {
    ($($arg:tt)*) => {};
}

#[cfg(test)]
#[macro_export]
macro_rules! fs_log {
    ($($arg:tt)*) => {
        println!($($arg)*)
    };
}
