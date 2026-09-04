pub struct RingBuffer<const N: usize> {
    bytes: [u8; N],
    read: usize,
    write: usize,
    len: usize,
}

impl<const N: usize> RingBuffer<N> {
    pub const fn new() -> Self {
        Self {
            bytes: [0; N],
            read: 0,
            write: 0,
            len: 0,
        }
    }

    pub fn push(&mut self, byte: u8) -> Result<(), u8> {
        if self.len == N {
            return Err(byte);
        }

        self.bytes[self.write] = byte;
        self.write = (self.write + 1) % N;
        self.len += 1;
        Ok(())
    }

    pub fn pop(&mut self) -> Option<u8> {
        if self.len == 0 {
            return None;
        }

        let byte = self.bytes[self.read];
        self.read = (self.read + 1) % N;
        self.len -= 1;
        Some(byte)
    }
}
