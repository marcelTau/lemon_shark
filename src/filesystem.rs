#![allow(unused)]
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
use alloc::vec;

use core::{mem, num::NonZeroU32};

use crate::{print, println};

// Layout of blocks:
//
// 0. Superblock
// 1-10. inode
// 10-. data blocks

const BLOCK_SIZE: usize = 512;
const INODE_BLOCKS: usize = 10;
const INODE_START: usize = 1;
const INODES_PER_BLOCK: usize = BLOCK_SIZE / core::mem::size_of::<INode>();
const DATA_START: usize = INODE_BLOCKS * BLOCK_SIZE;

const MAGIC: u32 = 0x5348524b; // shrk

mod ramdisk {
    use crate::filesystem::{BLOCK_SIZE, BlockIndex};

    const RAMDISK_SIZE: usize = 1024 * 1024;
    static mut RAMDISK: [u8; RAMDISK_SIZE] = [0; RAMDISK_SIZE];

    pub(crate) fn total_blocks() -> usize {
        RAMDISK_SIZE / BLOCK_SIZE
    }

    /// Read block `block_num` into `buf`.
    pub(crate) fn read_block(block_idx: BlockIndex, buf: &mut [u8]) -> Result<(), &'static str> {
        if buf.len() != BLOCK_SIZE {
            return Err("Buffer must be BLOCK_SIZE bytes");
        }

        let start = block_idx.0 * BLOCK_SIZE;

        if start + BLOCK_SIZE >= RAMDISK_SIZE {
            return Err("Block number out of range");
        }

        unsafe {
            buf.copy_from_slice(&RAMDISK[start..start + BLOCK_SIZE]);
        }

        Ok(())
    }

    pub(crate) fn write_block(block_idx: BlockIndex, data: &[u8]) -> Result<(), &'static str> {
        if data.len() != BLOCK_SIZE {
            return Err("Data must be BLOCK_SIZE bytes");
        }

        let start = block_idx.0 * BLOCK_SIZE;

        if start + BLOCK_SIZE >= RAMDISK_SIZE {
            return Err("Block number out of range");
        }

        unsafe {
            RAMDISK[start..start + BLOCK_SIZE].copy_from_slice(data);
        }

        Ok(())
    }
}

/// The `INode` contains metadata about a file.
///
/// Memory layout:
/// `size`          4 bytes
/// `blocks`        64 bytes
/// `is_directory`  1 byte
/// `padding`       3 bytes
///
/// *Important* do not change the layout of this struct or reorder the fields.
#[repr(C)]
#[derive(Debug)]
struct INode {
    size: u32,
    /// Used blocks of this file.
    blocks: [u32; 16],
    is_directory: bool,
}

impl INode {
    fn from_bytes(bytes: &[u8]) -> Self {
        let size = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let mut blocks: [u32; 16] = [0; 16];
        let mut i = 4;

        (0..16).for_each(|idx| {
            blocks[idx] = u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
            i += 4;
        });

        let is_directory = bytes[68] != 0;

        Self {
            size,
            is_directory,
            blocks,
        }
    }

    fn to_bytes(&self) -> [u8; mem::size_of::<INode>()] {
        let mut bytes = [0u8; mem::size_of::<INode>()];
        println!("blocks={:?}", self.blocks);

        bytes[0..4].copy_from_slice(&self.size.to_le_bytes());
        let current_offset = 4;
        for i in 0..16 {
            let start = current_offset + (i * 4);
            bytes[start..start + 4].copy_from_slice(&self.blocks[i].to_le_bytes());
        }
        println!("blocks={:?}", self.blocks);
        bytes[68] = if self.is_directory { 1 } else { 0 };

        bytes
    }

    fn last_block_index(&self) -> usize {
        self.blocks.iter().rposition(|block| *block > 0).unwrap()
    }
}

/// The `DirEntry` contains metadata about a directory.
#[repr(C)]
#[derive(Debug)]
struct DirEntry {
    name: [u8; 24],
    inode: u32,
}

impl DirEntry {
    fn from_bytes(bytes: &[u8]) -> Self {
        let inode = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let mut name: [u8; 24] = [0; 24];

        let current_offset = 4;
        (0..16).for_each(|i| {
            name[i] = bytes[current_offset + i];
        });

        Self { name, inode }
    }

    fn to_bytes(&self) -> [u8; mem::size_of::<DirEntry>()] {
        let mut bytes = [0u8; mem::size_of::<DirEntry>()];

        bytes[0..24].copy_from_slice(self.name.as_slice());
        bytes[24..28].copy_from_slice(&self.inode.to_le_bytes());

        bytes
    }
}

/// The `Superblock` contains counts & pointers to strucutres used and metadata
/// about the state of the allocator.
#[repr(C)]
#[derive(Debug)]
struct SuperBlock {
    magic: u32,
    block_size: u32,
    total_blocks: u32,

    inode_table_start: u32,
    inode_table_blocks: u32,

    data_start: u32,

    /// simple for now, position of the next free block. will need to get
    /// updated in linear time and will soon be replaced by a bitmap.
    next_free_block: Option<NonZeroU32>,
}

const SUPERBLOCK_SIZE: usize = core::mem::size_of::<SuperBlock>();

impl SuperBlock {
    fn from_bytes(bytes: &[u8]) -> Self {
        let magic = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let block_size = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        let total_blocks = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
        let inode_table_start = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
        let inode_table_blocks = u32::from_le_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
        let data_start = u32::from_le_bytes([bytes[19], bytes[20], bytes[21], bytes[22]]);
        let next_free_block = u32::from_le_bytes([bytes[19], bytes[20], bytes[21], bytes[22]]);

        Self {
            magic,
            block_size,
            total_blocks,
            inode_table_start,
            inode_table_blocks,
            data_start,
            next_free_block: NonZeroU32::new(next_free_block),
        }
    }

    fn to_bytes(&self) -> [u8; SUPERBLOCK_SIZE] {
        let mut bytes = [0u8; SUPERBLOCK_SIZE];

        let next_free_block = self
            .next_free_block
            .map(NonZeroU32::get)
            .unwrap_or_default();

        bytes[0..4].copy_from_slice(&self.magic.to_le_bytes());
        bytes[4..8].copy_from_slice(&self.block_size.to_le_bytes());
        bytes[8..12].copy_from_slice(&self.total_blocks.to_le_bytes());
        bytes[12..16].copy_from_slice(&self.inode_table_start.to_le_bytes());
        bytes[16..20].copy_from_slice(&self.inode_table_blocks.to_le_bytes());
        bytes[20..24].copy_from_slice(&self.data_start.to_le_bytes());
        bytes[24..28].copy_from_slice(&next_free_block.to_le_bytes());

        bytes
    }
}

// fn read_superblock(buf: &mut [u8]) -> SuperBlock {
//     ramdisk::read_block(0, &mut buf[..]).unwrap();
//
//     let bytes = &buf[0..core::mem::size_of::<SuperBlock>()];
//     SuperBlock::from_bytes(bytes)
// }
//
// fn write_superblock(superblock: SuperBlock) {
//     let mut buf = vec![0; BLOCK_SIZE];
//
//     buf[0..core::mem::size_of::<SuperBlock>()].copy_from_slice(superblock.to_bytes().as_slice());
//
//     ramdisk::write_block(0, &buf).unwrap();
// }
//
// fn read_inode(n: usize, buf: &mut [u8]) -> INode {
//     let offset = INODE_START + n;
//     ramdisk::read_block(offset, buf).unwrap();
//     INode::from_bytes(buf)
// }
//
// fn write_inode(n: usize, inode: INode) {
//     let mut buf = vec![0; BLOCK_SIZE];
//     buf[0..core::mem::size_of::<INode>()].copy_from_slice(inode.to_bytes().as_slice());
//     let offset = INODE_START + n;
//     ramdisk::write_block(offset, &buf).unwrap();
// }

// /// Write a `DirEntry` to a data block of a given INode.
// fn write_dir_entry(node: &INode, dir_entry: DirEntry) {
//     // Check if the currently used blocks have enough space for us to write
//     // the `DirEntry`
//     assert!(
//         node.is_directory,
//         "Node must be a directory to create `DirEntry`"
//     );
//
//     // this means that the current block is full.
//     // let block_index = if node.size.is_multiple_of(BLOCK_SIZE as u32) {
//     //     // allocate a new block
//
//     //     let mut buf = vec![0; BLOCK_SIZE];
//     //     let superblock = read_superblock(&mut buf[..]);
//     //     match superblock.next_free_block {
//     //         Some(free) => return free,
//     //         None => panic!("Filesystem exhausted"),
//     //     }
//     // } else {
//     //     node.last_block_index()
//     // };
//
//     let offset = DATA_START + offset;
//     let internal_offset = node.size % BLOCK_SIZE as u32;
// }

// /// The root node in the filesystem is the root directory.
// ///
// /// This function creates an empty root.
// ///
// /// The root block needs to be a directory and have 2 `DirEntry` blocks
// /// that both point back to the root node ("." & "..").
// fn make_empty_root_block() -> INode {
//     let mut blocks = [0u32; 16];
//
//     // one block can hold both of our default dir entries.
//     blocks[0] = 1;
//
//     let root_node = INode {
//         // size to hold the "." and ".." entries
//         size: mem::size_of::<DirEntry>() as u32 * 2,
//         is_directory: true,
//         blocks,
//     };
//
//     let mut name = [0u8; 24];
//     name[0] = '.' as u8;
//
//     let dir_entry = DirEntry { name, inode: 0 };
//
//     write_dir_entry(&root_node, dir_entry);
//
//     let mut name = [0u8; 24];
//     name[0] = '.' as u8;
//     name[1] = '.' as u8;
//
//     let dir_entry = DirEntry { name, inode: 0 };
//     write_dir_entry(&root_node, dir_entry);
//
//     root_node
// }

enum Offset {
    /// INode offset, *not block offset*.
    INode(usize),
    SuperBlock,
    Data(usize),
    DirEntry(usize),
    Block(usize),
}

/// Terminology:
/// * INode      - is a block of metadata about a file - written to INode
///                blocks
/// * DirEntry   - contains a name and the associated INode ID - is written to
///                Data block
/// * RawData    - contains raw file content - written to Data block
/// * Superblock - the first block in the filesystem containing metadata
///                about the state of the filesystem
struct Filesystem {
    buf: [u8; BLOCK_SIZE],
    next_free_block: Option<u32>,
}

#[derive(Copy, Clone, Debug)]
struct BlockIndex(usize);

impl BlockIndex {
    fn from_inode_index(inode_index: usize) -> Self {
        let block_index = INODE_START + (inode_index / INODES_PER_BLOCK);
        Self(block_index)
    }
}

impl Filesystem {
    fn new() -> Self {
        Self {
            buf: [0; BLOCK_SIZE],
            next_free_block: None,
        }
    }

    /// Writes the superblock to block_index 0
    fn write_superblock(&mut self, superblock: SuperBlock) {
        let offset = 0;
        self.buf.repeat(0);
    }

    /// Writes an `INode` into the block at the specific offset.
    fn write_inode(&mut self, offset: Offset, inode: &mut INode) {
        let Offset::INode(inode_offset) = offset else {
            panic!("Received wrong offset type");
        };

        println!("Writing INode {inode:?}");

        let block_index = BlockIndex::from_inode_index(inode_offset);

        // handle new block allocation
        if inode_offset.is_multiple_of(INODES_PER_BLOCK) {
            println!("Writing into new block {block_index:?}");
            if let Some(idx) = inode.blocks.iter().position(|v| v == &0) {
                inode.blocks[idx] = block_index.0 as u32;
            } else {
                panic!("No more blocks available for {inode:?}");
            }
            self.buf = [0; BLOCK_SIZE];
        } else {
            // write into existing block by reading into buf and appending to it.
            ramdisk::read_block(block_index, &mut self.buf).unwrap();
        }

        let byte_offset = (inode_offset % INODES_PER_BLOCK) * mem::size_of::<INode>();

        println!("Writing to block index {block_index:?} at byte_offset={byte_offset}");
        self.buf[byte_offset..byte_offset + mem::size_of::<INode>()]
            .copy_from_slice(inode.to_bytes().as_slice());

        ramdisk::write_block(block_index, &self.buf);
    }

    fn read_inode(&mut self, offset: Offset) -> INode {
        let Offset::INode(inode_offset) = offset else {
            panic!("Received wrong offset type");
        };

        let block_index = BlockIndex::from_inode_index(inode_offset);
        let byte_offset = (inode_offset % INODES_PER_BLOCK) * mem::size_of::<INode>();

        ramdisk::read_block(block_index, &mut self.buf).unwrap();

        INode::from_bytes(&self.buf[byte_offset..byte_offset + mem::size_of::<INode>()])
    }
}

pub fn dump() {
    let mut buf = [0u8; BLOCK_SIZE];
    let total = ramdisk::total_blocks();

    println!(
        "=== RAMDISK DUMP ({} blocks, {} bytes total) ===",
        total,
        total * BLOCK_SIZE
    );

    for block_idx in 0..total {
        if ramdisk::read_block(BlockIndex(block_idx), &mut buf).is_err() {
            continue;
        }

        // Skip empty blocks
        if buf.iter().all(|&b| b == 0) {
            continue;
        }

        println!(
            "\n--- Block {} (offset 0x{:06x}) ---",
            block_idx,
            block_idx * BLOCK_SIZE
        );

        for row in 0..(BLOCK_SIZE / 16) {
            let offset = row * 16;
            let addr = block_idx * BLOCK_SIZE + offset;

            // Print address
            print!("{:06x}  ", addr);

            // Print hex bytes
            for i in 0..16 {
                print!("{:02X} ", buf[offset + i]);
                if i == 7 {
                    print!(" ");
                }
            }

            // Print ASCII representation
            print!(" |");
            for i in 0..16 {
                let b = buf[offset + i];
                if (0x20..0x7f).contains(&b) {
                    print!("{}", b as char);
                } else {
                    print!(".");
                }
            }
            println!("|");
        }
    }

    println!("\n=== END DUMP ===");
}

pub fn init() {
    // let superblock = SuperBlock {
    //     magic: MAGIC,
    //     block_size: BLOCK_SIZE as u32,
    //     total_blocks: ramdisk::total_blocks() as u32,
    //     inode_table_start: 1 * BLOCK_SIZE as u32,
    //     inode_table_blocks: INODE_BLOCKS as u32,
    //     data_start: DATA_START as u32,
    //     next_free_block: Some(NonZeroU32::new(1).unwrap()),
    // };

    // write_superblock(superblock);

    // let mut buf = vec![0; BLOCK_SIZE];
    // let superblock = read_superblock(&mut buf[..]);
    // println!("Woo found SuperBlock = {superblock:?}");

    // write_inode(0, inode);
    // let mut buf = vec![0; BLOCK_SIZE];
    // let inode = read_inode(0, &mut buf[..]);

    let mut fs = Filesystem::new();

    let mut inode1 = INode {
        size: 0xAAAAAAAA,
        blocks: [
            0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ],
        is_directory: true,
    };

    let mut inode2 = INode {
        size: 0xBBBBBBBB,
        is_directory: true,
        blocks: [1, 2, 3, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    };

    let mut inode3 = INode {
        size: 0xCCCCCCCC,
        is_directory: true,
        blocks: [1, 2, 3, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    };

    fs.write_inode(Offset::INode(0), &mut inode1);
    // fs.write_inode(Offset::INode(6), &mut inode2);
    // fs.write_inode(Offset::INode(7), &mut inode3);

    // println!("Wrote INode {inode:?}");

    let read = fs.read_inode(Offset::INode(0));
    println!("Found INode {read:?}");

    let read = fs.read_inode(Offset::INode(6));
    println!("Found INode {read:?}");
    dump()
}
