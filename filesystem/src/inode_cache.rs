use crate::{BLOCK_SIZE, BlockDevice, INode, INodeIndex, bitmap::Bitmap, layout::Layout};

extern crate alloc;
use alloc::vec::Vec;

#[derive(Default)]
pub(crate) struct INodeCache {
    /// A `vec` holding all used `INodes` mapped by their `INodeIndex`. If an
    /// `INode` has not been read yet, the value in it's spot is `None`.
    inodes: Vec<Option<INode>>,
    dirty: Bitmap,
    layout: Option<Layout>,
}

impl INodeCache {
    pub fn new(layout: Layout) -> Self {
        let size = layout.inode_bitmap_blocks * BLOCK_SIZE;
        log::debug!("INodeCache size={size}");
        Self {
            inodes: Vec::new(),
            layout: Some(layout),
            dirty: Bitmap::new(size as u32),
        }
    }

    /// Reads the `INode` from disk if it's not already in the cache.
    fn read_from_disk<D: BlockDevice>(&self, index: INodeIndex, device: &mut D) -> INode {
        let mut buf = [0u8; BLOCK_SIZE];
        let (block_index, byte_offset) = self.layout.as_ref().unwrap().inode_to_block(index);
        device.read_block(block_index, &mut buf);
        INode::from_bytes(&buf[byte_offset.range::<INode>()])
    }

    /// Get a `&INode` from the cache, fetching it from disk when not present.
    pub fn get<D: BlockDevice>(&mut self, index: INodeIndex, device: &mut D) -> &INode {
        if self.inodes.len() <= index.inner() as usize {
            self.inodes
                .resize_with(index.inner() as usize + 1, Default::default);
        }

        if self.inodes[index.inner() as usize].is_none() {
            self.inodes[index.inner() as usize] = Some(self.read_from_disk(index, device));
        }

        self.inodes
            .get(index.inner() as usize)
            .unwrap()
            .as_ref()
            .unwrap()
    }

    /// Get a `&mut INode` from the cache, fetching it from disk when not
    /// present.
    pub fn get_mut<D: BlockDevice>(&mut self, index: INodeIndex, device: &mut D) -> &mut INode {
        if self.inodes.len() <= index.inner() as usize {
            self.inodes
                .resize_with(index.inner() as usize + 1, Default::default);
        }

        if self.inodes[index.inner() as usize].is_none() {
            self.inodes[index.inner() as usize] = Some(self.read_from_disk(index, device));
        }

        // When handing out a mutable reference, consider it dirty.
        self.dirty.set(index.inner());

        self.inodes
            .get_mut(index.inner() as usize)
            .unwrap()
            .as_mut()
            .unwrap()
    }

    pub fn remove(&mut self, index: INodeIndex) {
        self.inodes[index.inner() as usize] = None;
    }

    pub fn register_new_inode(&mut self, index: INodeIndex, inode: INode) {
        if self.inodes.len() <= index.inner() as usize {
            self.inodes
                .resize_with(index.inner() as usize + 1, Default::default);
        }

        self.inodes[index.inner() as usize] = Some(inode);
    }

    pub fn drain(&mut self) -> impl Iterator<Item = (INodeIndex, INode)> {
        self.dirty
            .drain_set()
            .map(INodeIndex::new)
            .map(|idx| (idx, self.inodes[idx.inner() as usize].unwrap()))
    }
}
