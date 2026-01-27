use crate::logln;

#[derive(Debug, PartialEq)]
pub struct Bitmap<const WORDS: usize> {
    arr: [u32; WORDS],
}

impl<const WORDS: usize> Bitmap<WORDS> {
    pub const fn new() -> Self {
        Self { arr: [0u32; WORDS] }
    }

    pub fn from_bytes(bytes: &[u8]) -> Self {
        assert_eq!(bytes.len(), WORDS * 4);

        let mut arr = [0u32; WORDS];
        for (i, chunk) in bytes.chunks(4).enumerate() {
            arr[i] = u32::from_le_bytes(chunk.try_into().unwrap());
        }
        Self { arr }
    }

    pub fn to_bytes(&self) -> &[u8] {
        unsafe { core::slice::from_raw_parts(self.arr.as_ptr() as *const u8, WORDS * 4) }
    }

    pub fn set(&mut self, index: u32) {
        assert!(index < WORDS as u32 * 32);

        let arr_index = index / 32;
        let bit_index = index % 32;

        self.arr[arr_index as usize] |= 1 << bit_index;
    }

    pub fn unset(&mut self, index: u32) {
        assert!(index < WORDS as u32 * 32);

        let arr_index = index / 32;
        let bit_index = index % 32;

        self.arr[arr_index as usize] &= !(1 << bit_index);
    }

    pub fn is_set(&self, index: u32) -> bool {
        assert!(index < WORDS as u32 * 32);

        let arr_index = index / 32;
        let bit_index = index % 32;

        self.arr[arr_index as usize] & (1 << bit_index) > 0
    }

    /// Find the first free block in the bitmap.
    pub fn find_free(&self) -> Option<u32> {
        for (arr_idx, bits) in self.arr.iter().enumerate().skip(1) {
            if bits != &u32::MAX {
                let res = (!bits).trailing_zeros();
                logln!("[FS] find free found at {arr_idx} {res}");
                return Some(arr_idx as u32 * 32 + res);
            }
        }

        None
    }
}
