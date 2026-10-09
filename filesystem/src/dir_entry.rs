use crate::{
    INodeIndex,
    bytereader::{ByteReader, ByteWriter, DiskFormat},
};

/// The `DirEntry` contains metadata about an entry in a directory such as a
/// file or another directory which is pointed to by the `INodeIndex`.
/// NOTE: BLOCK_SIZE must always be a multiple of `DirEntry` to ensure tighly fitted entries.
#[derive(PartialEq)]
#[repr(C)]
pub(crate) struct DirEntry {
    /// Name of the directory
    name: [u8; 24],

    /// INode index of this directory
    inode: INodeIndex,
}

impl core::fmt::Debug for DirEntry {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> Result<(), core::fmt::Error> {
        write!(
            f,
            "DirEntry name=\"{}\" inode={:?}",
            self.name_str(),
            self.inode
        )
    }
}

impl DirEntry {
    pub(crate) fn new(name_string: &str, inode: INodeIndex) -> Self {
        let mut name = [0u8; 24];
        let bytes = name_string.as_bytes();
        let len = bytes.len().min(24);

        name[..len].copy_from_slice(&bytes[..len]);

        DirEntry { name, inode }
    }

    /// Borrow the unpadded name for display, using an empty name for invalid UTF-8.
    pub(crate) fn name_str(&self) -> &str {
        let len = self
            .name
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(self.name.len());
        core::str::from_utf8(&self.name[..len]).unwrap_or_default()
    }

    pub(crate) fn name_bytes(&self) -> &[u8] {
        self.name.as_slice()
    }

    pub(crate) fn inode(&self) -> INodeIndex {
        self.inode
    }
}

impl DiskFormat for DirEntry {
    fn write_to(&self, writer: &mut ByteWriter) {
        writer.write_bytes(&self.name);
        writer.write_u32(self.inode.inner());
    }

    fn read_from(reader: &mut ByteReader) -> Self {
        let name = reader.read_bytes(24).try_into().unwrap();
        let inode = INodeIndex::new(reader.read_u32());
        Self { name, inode }
    }
}
