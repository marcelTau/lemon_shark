use crate::{bytereader::ByteReader, logln, println};
extern crate alloc;
use alloc::vec::Vec;
use core::mem;

#[derive(Debug, PartialEq, Clone, Default)]
pub struct Bitmap {
    len: u32,
    arr: Vec<u32>,
}

impl Bitmap {
    pub fn new(bits: u32) -> Self {
        let len = bits / 32 + 1;
        Self {
            len,
            arr: alloc::vec![0; len as usize],
        }
    }

    pub fn from_bytes(bytes: &[u8]) -> Self {
        let mut reader = ByteReader::new(bytes);
        let len = reader.read_u32();

        println!("Allocating {} * 8", len);
        let mut arr = Vec::with_capacity(len as usize);

        for chunk in bytes.chunks(4).skip(1) {
            arr.push(u32::from_le_bytes(chunk.try_into().unwrap()));
        }

        Self { len, arr }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = alloc::vec![0u8;
            mem::size_of::<u32>() * (self.len as usize + 1)
        ];

        bytes[0..4].copy_from_slice(&self.len.to_le_bytes());
        let u8_arr_slice = unsafe {
            core::slice::from_raw_parts(self.arr.as_ptr() as *const u8, self.len as usize * 4)
        };

        bytes[4..].copy_from_slice(u8_arr_slice);

        bytes
    }

    pub fn set(&mut self, index: u32) {
        let arr_index = index / 32;
        let bit_index = index % 32;

        self.arr[arr_index as usize] |= 1 << bit_index;
    }

    pub fn unset(&mut self, index: u32) {
        let arr_index = index / 32;
        let bit_index = index % 32;

        self.arr[arr_index as usize] &= !(1 << bit_index);
    }

    pub fn is_set(&self, index: u32) -> bool {
        let arr_index = index / 32;
        let bit_index = index % 32;

        self.arr[arr_index as usize] & (1 << bit_index) > 0
    }

    /// Find the first free block in the bitmap.
    pub fn find_free(&self) -> Option<u32> {
        for (arr_idx, bits) in self.arr.iter().enumerate() {
            if bits != &u32::MAX {
                let res = (!bits).trailing_zeros();
                logln!("[FS] find free found at {arr_idx} {res}");
                return Some(arr_idx as u32 * 32 + res);
            }
        }

        None
    }
    /// Returns an iterator over all set bits and unsets them.
    pub fn fdrain_set(&self) -> impl Iterator<Item = u32> {
        self.arr
            .iter()
            .copied()
            .enumerate()
            .flat_map(|(idx, mut n)| {
                core::iter::from_fn(move || {
                    if n == 0 {
                        return None;
                    }

                    let trailing = n.trailing_zeros();

                    n &= !(1u32 << trailing);

                    let index = idx * 32 + trailing as usize;

                    Some(index as u32)
                })
            })
    }

    /// Returns an iterator over all set bits and unsets them.
    pub fn drain_set(&mut self) -> impl Iterator<Item = u32> {
        self.arr.iter_mut().enumerate().flat_map(|(idx, n)| {
            core::iter::from_fn(move || {
                if *n == 0 {
                    return None;
                }

                let trailing = n.trailing_zeros();

                *n &= !(1u32 << trailing);

                let index = idx * 32 + trailing as usize;

                Some(index as u32)
            })
        })
    }
}
