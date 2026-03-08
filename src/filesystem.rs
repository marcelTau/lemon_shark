extern crate alloc;
use crate::println::UartWriter;
use crate::virtio2::LockedBlockDevice;
use crate::{print, println, ramdisk};
use alloc::string::String;
use core::sync::atomic::{AtomicBool, Ordering};
use filesystem::{BlockDevice, Filesystem};

pub use filesystem::{BLOCK_SIZE, BlockIndex, Error, INodeIndex};

/// The concrete block device used by the kernel, wrapping either the in-memory
/// ramdisk or the VirtIO persistent storage.
#[allow(clippy::large_enum_variant)]
pub enum KernelBlockDevice {
    /// Internal `ramdisk` used for testing mostly.
    Ramdisk,

    /// VirtIO block device for persistent storage.
    VirtIO(LockedBlockDevice<'static>),
}

impl BlockDevice for KernelBlockDevice {
    fn read_block(&mut self, block_idx: BlockIndex, buf: &mut [u8]) {
        match self {
            KernelBlockDevice::Ramdisk => ramdisk::read_block(block_idx, buf),
            KernelBlockDevice::VirtIO(blk) => blk.read_block(block_idx, buf),
        }
    }

    fn write_block(&mut self, block_idx: BlockIndex, data: &[u8]) {
        match self {
            KernelBlockDevice::Ramdisk => ramdisk::write_block(block_idx, data),
            KernelBlockDevice::VirtIO(blk) => blk.write_block(block_idx, data),
        }
    }

    fn total_blocks(&mut self) -> usize {
        match self {
            KernelBlockDevice::Ramdisk => ramdisk::total_blocks(),
            KernelBlockDevice::VirtIO(blk) => blk.total_blocks(),
        }
    }
}

impl KernelBlockDevice {
    /// Complete memory dump of all used pages. Useful for debugging.
    pub fn dump_non_empty_pages(&mut self) {
        const PAGE_BYTES: usize = 4096;
        const BYTES_PER_ROW: usize = 16;
        const BLOCKS_PER_PAGE: usize = PAGE_BYTES / BLOCK_SIZE;

        let mut page = [0u8; PAGE_BYTES];
        let device_name = match self {
            KernelBlockDevice::Ramdisk => "RAMDISK",
            KernelBlockDevice::VirtIO(_) => "VIRTIO",
        };
        let total_blocks = self.total_blocks();
        let total_pages = total_blocks.div_ceil(BLOCKS_PER_PAGE);

        println!(
            "=== {} DUMP ({} blocks, {} pages, {} bytes total) ===",
            device_name,
            total_blocks,
            total_pages,
            total_blocks * BLOCK_SIZE
        );

        for page_idx in 0..total_pages {
            let first_block = page_idx * BLOCKS_PER_PAGE;
            let blocks_in_page = (total_blocks - first_block).min(BLOCKS_PER_PAGE);
            let bytes_in_page = blocks_in_page * BLOCK_SIZE;

            for block_offset in 0..blocks_in_page {
                let offset = block_offset * BLOCK_SIZE;
                self.read_block(
                    BlockIndex::from_raw((first_block + block_offset) as u32),
                    &mut page[offset..offset + BLOCK_SIZE],
                );
            }

            if page[..bytes_in_page].iter().all(|&byte| byte == 0) {
                continue;
            }

            let page_offset = first_block * BLOCK_SIZE;
            println!("\n--- Page {page_idx} (block {first_block}, offset 0x{page_offset:08x}) ---");

            for row in (0..bytes_in_page).step_by(BYTES_PER_ROW) {
                let addr = page_offset + row;
                print!("{addr:08x}  ");

                for i in 0..BYTES_PER_ROW {
                    if row + i < bytes_in_page {
                        print!("{:02X} ", page[row + i]);
                    } else {
                        print!("   ");
                    }

                    if i == 7 {
                        print!(" ");
                    }
                }

                print!(" |");
                for i in 0..BYTES_PER_ROW {
                    if row + i >= bytes_in_page {
                        print!(" ");
                        continue;
                    }

                    let byte = page[row + i];
                    if (0x20..0x7f).contains(&byte) {
                        print!("{}", byte as char);
                    } else {
                        print!(".");
                    }
                }
                println!("|");
            }
        }

        println!("\n=== END DUMP ===");
    }
}

static FS: spin::Mutex<LockedFilesystem> = spin::Mutex::new(LockedFilesystem::new());
static FS_INITIALIZED: AtomicBool = AtomicBool::new(false);

struct LockedFilesystem {
    inner: Option<Filesystem<KernelBlockDevice>>,
}

impl LockedFilesystem {
    pub const fn new() -> Self {
        Self { inner: None }
    }

    fn get(&mut self) -> &mut Filesystem<KernelBlockDevice> {
        self.inner.as_mut().unwrap()
    }

    pub fn init(&mut self, filesystem: Filesystem<KernelBlockDevice>) {
        self.inner = Some(filesystem);
    }

    pub fn mkdir(&mut self, path: &str) -> Result<INodeIndex, Error> {
        self.get().mkdir(path)
    }

    pub fn create_file(&mut self, path: &str) -> Result<INodeIndex, Error> {
        self.get().create_file(path)
    }

    fn dump_dir(&mut self, index: u32) {
        self.get().dump_dir(index, &mut UartWriter)
    }

    fn dump(&mut self) {
        self.get().block_device_mut().dump_non_empty_pages()
    }

    fn write_to_file(&mut self, inode_index: INodeIndex, bytes: &[u8]) -> Result<usize, Error> {
        self.get().write_to_file(inode_index, bytes)
    }

    pub(crate) fn read_file(&mut self, inode_index: INodeIndex) -> String {
        self.get().read_file(inode_index)
    }

    fn reset(&mut self) {
        ramdisk::reset();
    }

    fn tree(&mut self) {
        self.get().tree(&mut UartWriter)
    }

    fn flush(&mut self) {
        self.get().flush()
    }
}

/// Those functions are wrappers around the `LockedFilesystem` for the shell
/// to do some filesystem operations.
///
/// This is the only place where the `.lock()` should be called to avoid
/// deadlocks.
pub mod api {
    use super::*;

    pub fn dump() {
        (*FS.lock()).dump();
    }

    pub fn dump_dir(index: u32) {
        (*FS.lock()).dump_dir(index);
    }

    pub fn mkdir(name: &str) -> Result<INodeIndex, Error> {
        (*FS.lock()).mkdir(name)
    }

    pub fn create_file(name: &str) -> Result<INodeIndex, Error> {
        (*FS.lock()).create_file(name)
    }

    pub fn write_to_file(inode_index: usize, text: String) -> Result<usize, Error> {
        (*FS.lock()).write_to_file(INodeIndex::new(inode_index as u32), text.as_bytes())
    }

    pub fn read_file(inode_index: usize) -> String {
        (*FS.lock()).read_file(INodeIndex::new(inode_index as u32))
    }

    pub fn reset() {
        (*FS.lock()).reset();
    }

    pub fn tree() {
        (*FS.lock()).tree();
    }

    pub fn flush() {
        (*FS.lock()).flush();
    }
}

/// Initializes the Filesystem by reading the superblock or defaulting it
/// if it doesn't exist.
pub fn init_with_device(dev: KernelBlockDevice) {
    // Guard against re-initializing the Filesystem by setting the atomic
    // flag.
    if FS_INITIALIZED
        .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
        .is_err()
    {
        log::info!("not initializing the filesystem again");
        return;
    }

    let fs = Filesystem::new(dev);

    (*FS.lock()).init(fs);

    log::info!("initialized");
}
