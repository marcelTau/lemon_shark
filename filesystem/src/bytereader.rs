/// Helper struct to read raw bytes.
pub(crate) struct ByteReader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> ByteReader<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    pub(crate) fn read_u32(&mut self) -> u32 {
        let value = u32::from_le_bytes(self.bytes[self.pos..self.pos + 4].try_into().unwrap());
        self.pos += 4;
        value
    }

    pub(crate) fn read_u64(&mut self) -> u64 {
        let value = u64::from_le_bytes(self.bytes[self.pos..self.pos + 8].try_into().unwrap());
        self.pos += 8;
        value
    }

    pub(crate) fn read_bytes(&mut self, len: usize) -> &'a [u8] {
        let slice = &self.bytes[self.pos..self.pos + len];
        self.pos += len;
        slice
    }
}
