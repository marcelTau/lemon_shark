/// Helper struct to read raw bytes.
pub(crate) struct ByteReader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> ByteReader<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    pub(crate) fn at(bytes: &'a [u8], pos: usize) -> Self {
        Self { bytes, pos }
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

    pub(crate) fn read_u8(&mut self) -> u8 {
        let value = self.bytes[self.pos];
        self.pos += 1;
        value
    }
}

pub(crate) struct ByteWriter<'a> {
    bytes: &'a mut [u8],
    pos: usize,
}

impl<'a> ByteWriter<'a> {
    pub(crate) fn new(bytes: &'a mut [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    pub(crate) fn at(bytes: &'a mut [u8], pos: usize) -> Self {
        Self { bytes, pos }
    }

    pub(crate) fn write_u32(&mut self, value: u32) {
        self.bytes[self.pos..self.pos + 4].copy_from_slice(&value.to_le_bytes());
        self.pos += 4;
    }

    pub(crate) fn write_u64(&mut self, value: u64) {
        self.bytes[self.pos..self.pos + 8].copy_from_slice(&value.to_le_bytes());
        self.pos += 8;
    }

    pub(crate) fn write_u8(&mut self, value: u8) {
        self.bytes[self.pos] = value;
        self.pos += 1;
    }

    pub(crate) fn write_bytes(&mut self, value: &[u8]) {
        self.bytes[self.pos..self.pos + value.len()].copy_from_slice(value);
        self.pos += value.len();
    }
}

pub(crate) trait DiskFormat: Sized {
    fn write_to(&self, writer: &mut ByteWriter);
    fn read_from(reader: &mut ByteReader) -> Self;

    fn from_bytes(bytes: &[u8]) -> Self {
        Self::read_from(&mut ByteReader::new(bytes))
    }
}
