use crate::filesystem::{BLOCK_SIZE, BlockIndex};

pub(crate) const RAMDISK_SIZE: usize = 1024 * 1024;

static mut RAMDISK: [u8; RAMDISK_SIZE] = [0; RAMDISK_SIZE];

pub(crate) const fn total_blocks() -> usize {
    RAMDISK_SIZE / BLOCK_SIZE
}

/// Read block `block_num` into `buf`.
pub(crate) fn read_block(block_idx: BlockIndex, buf: &mut [u8]) {
    if buf.len() != BLOCK_SIZE {
        panic!("Buffer must be BLOCK_SIZE bytes");
    }

    let start = block_idx.0 as usize * BLOCK_SIZE;

    if start + BLOCK_SIZE > RAMDISK_SIZE {
        panic!(
            "Block number out of range {} >= {RAMDISK_SIZE}",
            start + BLOCK_SIZE
        );
    }

    unsafe {
        buf.copy_from_slice(&RAMDISK[start..start + BLOCK_SIZE]);
    }
}

pub(crate) fn write_block(block_idx: BlockIndex, data: &[u8]) {
    if data.len() != BLOCK_SIZE {
        panic!("Data must be BLOCK_SIZE bytes");
    }

    let start = block_idx.0 as usize * BLOCK_SIZE;

    if start + BLOCK_SIZE > RAMDISK_SIZE {
        panic!(
            "Block number out of range {} >= {RAMDISK_SIZE}",
            start + BLOCK_SIZE
        );
    }

    unsafe {
        RAMDISK[start..start + BLOCK_SIZE].copy_from_slice(data);
    }
}
