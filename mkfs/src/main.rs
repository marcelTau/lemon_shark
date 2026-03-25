use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};

use filesystem::{BLOCK_SIZE, BlockDevice, BlockIndex, Filesystem};

struct FileBlockDevice {
    file: std::fs::File,
    total_blocks: usize,
}

impl FileBlockDevice {
    fn create(path: &str, total_blocks: usize) -> std::io::Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;
        file.set_len((total_blocks * BLOCK_SIZE) as u64)?;
        Ok(Self { file, total_blocks })
    }
}

impl BlockDevice for FileBlockDevice {
    fn read_block(&mut self, block_idx: BlockIndex, buf: &mut [u8]) {
        self.file
            .seek(SeekFrom::Start(
                block_idx.inner() as u64 * BLOCK_SIZE as u64,
            ))
            .unwrap();
        self.file.read_exact(buf).unwrap();
    }

    fn write_block(&mut self, block_idx: BlockIndex, data: &[u8]) {
        self.file
            .seek(SeekFrom::Start(
                block_idx.inner() as u64 * BLOCK_SIZE as u64,
            ))
            .unwrap();
        self.file.write_all(data).unwrap();
    }

    fn total_blocks(&mut self) -> usize {
        self.total_blocks
    }
}

const DEFAULT_OUTPUT: &str = "lemonfs.img";
const DEFAULT_BLOCKS: usize = 16 * 1024 * 1024 / BLOCK_SIZE; // 16 MiB

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let path = args.get(1).map(String::as_str).unwrap_or(DEFAULT_OUTPUT);
    let total_blocks = args
        .get(2)
        .map(|s| s.parse().expect("total_blocks must be a number"))
        .unwrap_or(DEFAULT_BLOCKS);

    let dev = FileBlockDevice::create(path, total_blocks).expect("failed to create image");
    Filesystem::format(dev);

    println!(
        "Formatted {} ({} blocks, {} bytes)",
        path,
        total_blocks,
        total_blocks * BLOCK_SIZE
    );
}
