#![cfg_attr(not(test), no_std)]
extern crate alloc;
use alloc::vec;
use alloc::vec::Vec;

/// A simple bitmap implemented ontop of u32's.
#[derive(Debug, PartialEq, Clone, Default)]
pub struct Bitmap {
    arr: Vec<u32>,
}

impl Bitmap {
    pub fn new(bits: u32) -> Self {
        assert!(
            bits.is_multiple_of(32),
            "`find_free` breaks if it's not a multiple of 32"
        );
        let word_count = bits.div_ceil(32);
        Self {
            arr: vec![0; word_count as usize],
        }
    }

    pub fn from_raw(arr: Vec<u32>) -> Self {
        Self { arr }
    }

    pub fn words(&self) -> &[u32] {
        &self.arr
    }

    pub fn set(&mut self, index: u32) {
        debug_assert!(
            index < self.arr.len() as u32 * 32,
            "Bitmap index {index} out of bounds"
        );
        let arr_index = index / 32;
        let bit_index = index % 32;

        self.arr[arr_index as usize] |= 1 << bit_index;
    }

    pub fn unset(&mut self, index: u32) {
        debug_assert!(
            index < self.arr.len() as u32 * 32,
            "Bitmap index {index} out of bounds"
        );
        let arr_index = index / 32;
        let bit_index = index % 32;

        self.arr[arr_index as usize] &= !(1 << bit_index);
    }

    pub fn is_set(&self, index: u32) -> bool {
        debug_assert!(
            index < self.arr.len() as u32 * 32,
            "Bitmap index {index} out of bounds"
        );
        let arr_index = index / 32;
        let bit_index = index % 32;

        self.arr[arr_index as usize] & (1 << bit_index) > 0
    }

    /// Find the first free block in the bitmap.
    pub fn find_free(&self) -> Option<u32> {
        for (arr_idx, bits) in self.arr.iter().enumerate() {
            if bits != &u32::MAX {
                let res = (!bits).trailing_zeros();
                log::debug!("find free found at {arr_idx} {res}");
                return Some(arr_idx as u32 * 32 + res);
            }
        }

        None
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_all_bits_clear() {
        let bitmap = Bitmap::new(64);
        for i in 0..64 {
            assert!(!bitmap.is_set(i));
        }
    }

    #[test]
    #[should_panic]
    fn new_rejects_non_multiple_of_32() {
        Bitmap::new(33);
    }

    #[test]
    fn new_minimum_size() {
        let bitmap = Bitmap::new(32);
        assert_eq!(bitmap.arr.len(), 1);
    }

    #[test]
    fn set_and_is_set_first_bit() {
        let mut bitmap = Bitmap::new(32);
        bitmap.set(0);
        assert!(bitmap.is_set(0));
    }

    #[test]
    fn set_and_is_set_last_bit_in_word() {
        let mut bitmap = Bitmap::new(32);
        bitmap.set(31);
        assert!(bitmap.is_set(31));
    }

    #[test]
    fn set_and_is_set_first_bit_in_second_word() {
        let mut bitmap = Bitmap::new(64);
        bitmap.set(32);
        assert!(bitmap.is_set(32));
        assert!(!bitmap.is_set(31));
        assert!(!bitmap.is_set(33));
    }

    #[test]
    fn set_and_is_set_last_bit() {
        let mut bitmap = Bitmap::new(64);
        bitmap.set(63);
        assert!(bitmap.is_set(63));
    }

    #[test]
    fn set_is_idempotent() {
        let mut bitmap = Bitmap::new(32);
        bitmap.set(5);
        bitmap.set(5);
        assert!(bitmap.is_set(5));
    }

    #[test]
    fn set_does_not_affect_neighbours() {
        let mut bitmap = Bitmap::new(64);
        bitmap.set(10);
        assert!(!bitmap.is_set(9));
        assert!(!bitmap.is_set(11));
    }

    #[test]
    fn unset_clears_bit() {
        let mut bitmap = Bitmap::new(32);
        bitmap.set(7);
        bitmap.unset(7);
        assert!(!bitmap.is_set(7));
    }

    #[test]
    fn unset_already_clear_bit_is_noop() {
        let mut bitmap = Bitmap::new(32);
        bitmap.unset(7);
        assert!(!bitmap.is_set(7));
    }

    #[test]
    fn unset_does_not_affect_neighbours() {
        let mut bitmap = Bitmap::new(64);
        bitmap.set(9);
        bitmap.set(10);
        bitmap.set(11);
        bitmap.unset(10);
        assert!(bitmap.is_set(9));
        assert!(!bitmap.is_set(10));
        assert!(bitmap.is_set(11));
    }

    #[test]
    #[should_panic]
    fn set_out_of_bounds_panics_in_debug() {
        let mut bitmap = Bitmap::new(32);
        bitmap.set(32);
    }

    #[test]
    #[should_panic]
    fn unset_out_of_bounds_panics_in_debug() {
        let mut bitmap = Bitmap::new(32);
        bitmap.unset(32);
    }

    #[test]
    #[should_panic]
    fn is_set_out_of_bounds_panics_in_debug() {
        let bitmap = Bitmap::new(32);
        bitmap.is_set(32);
    }

    #[test]
    fn find_free_empty_bitmap_returns_zero() {
        let bitmap = Bitmap::new(64);
        assert_eq!(bitmap.find_free(), Some(0));
    }

    #[test]
    fn find_free_returns_lowest_free() {
        let mut bitmap = Bitmap::new(64);
        bitmap.set(0);
        bitmap.set(1);
        bitmap.set(2);
        assert_eq!(bitmap.find_free(), Some(3));
    }

    #[test]
    fn find_free_after_first_word_full() {
        let mut bitmap = Bitmap::new(64);
        for i in 0..32 {
            bitmap.set(i);
        }
        assert_eq!(bitmap.find_free(), Some(32));
    }

    #[test]
    fn find_free_full_bitmap_returns_none() {
        let mut bitmap = Bitmap::new(64);
        for i in 0..64 {
            bitmap.set(i);
        }
        assert_eq!(bitmap.find_free(), None);
    }

    #[test]
    fn find_free_after_unset() {
        let mut bitmap = Bitmap::new(64);
        for i in 0..64 {
            bitmap.set(i);
        }
        bitmap.unset(37);
        assert_eq!(bitmap.find_free(), Some(37));
    }

    #[test]
    fn drain_set_empty_yields_nothing() {
        let mut bitmap = Bitmap::new(64);
        let result: Vec<u32> = bitmap.drain_set().collect();
        assert!(result.is_empty());
    }

    #[test]
    fn drain_set_yields_all_set_indices() {
        let mut bitmap = Bitmap::new(64);
        let indices = [0u32, 5, 31, 32, 63];
        for &i in &indices {
            bitmap.set(i);
        }
        let mut result: Vec<u32> = bitmap.drain_set().collect();
        result.sort();
        assert_eq!(result, indices);
    }

    #[test]
    fn drain_set_clears_bits() {
        let mut bitmap = Bitmap::new(64);
        bitmap.set(1);
        bitmap.set(33);
        let _: Vec<_> = bitmap.drain_set().collect();
        assert!(!bitmap.is_set(1));
        assert!(!bitmap.is_set(33));
    }

    #[test]
    fn drain_set_yields_in_ascending_order() {
        let mut bitmap = Bitmap::new(128);
        for i in [63u32, 0, 100, 32, 7] {
            bitmap.set(i);
        }
        let result: Vec<u32> = bitmap.drain_set().collect();
        let mut sorted = result.clone();
        sorted.sort();
        assert_eq!(result, sorted);
    }

    #[test]
    fn drain_set_full_bitmap() {
        let mut bitmap = Bitmap::new(64);
        for i in 0..64 {
            bitmap.set(i);
        }
        let result: Vec<u32> = bitmap.drain_set().collect();
        assert_eq!(result, (0u32..64).collect::<Vec<_>>());
        for i in 0..64 {
            assert!(!bitmap.is_set(i));
        }
    }
}
