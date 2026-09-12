#![cfg_attr(not(test), no_std)]
#![allow(clippy::len_without_is_empty)]
extern crate alloc;
use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

const BITS_PER_WORD: usize = u32::BITS as usize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidWordCount {
    pub expected: usize,
    pub actual: usize,
}

/// A simple bitmap implemented ontop of u32's.
#[derive(Debug, PartialEq, Clone, Default)]
pub struct Bitmap {
    len: usize,
    words: Box<[u32]>,
}

impl Bitmap {
    /// Create a new `Bitmap` which holds up to `len` bits.
    pub fn new(len: usize) -> Self {
        let word_count = len.div_ceil(BITS_PER_WORD);
        let words = vec![0u32; word_count].into_boxed_slice();

        Self { len, words }
    }

    /// Create a bitmap from exactly enough raw words to represent `len` bits.
    pub fn from_words(len: usize, words: Vec<u32>) -> Result<Self, InvalidWordCount> {
        let expected = len.div_ceil(BITS_PER_WORD);
        if words.len() != expected {
            return Err(InvalidWordCount {
                expected,
                actual: words.len(),
            });
        }

        let mut bitmap = Self {
            len,
            words: words.into_boxed_slice(),
        };

        // Bits beyond the logical end are padding, never allocatable bits.
        if let (Some(last), remainder) = (bitmap.words.last_mut(), len % BITS_PER_WORD)
            && remainder != 0
        {
            *last &= (1u32 << remainder) - 1;
        }

        Ok(bitmap)
    }

    pub fn len(&self) -> usize {
        self.len
    }

    /// Set the bit at `index`.
    pub fn set(&mut self, index: usize) {
        assert!(index < self.len, "Bitmap index {index} out of bounds");

        let arr_index = index / BITS_PER_WORD;
        let bit_index = index % BITS_PER_WORD;

        self.words[arr_index] |= 1 << bit_index;
    }

    /// Unset the bit at `index`.
    pub fn unset(&mut self, index: usize) {
        assert!(index < self.len, "Bitmap index {index} out of bounds");

        let arr_index = index / BITS_PER_WORD;
        let bit_index = index % BITS_PER_WORD;

        self.words[arr_index] &= !(1 << bit_index);
    }

    /// Test wether the bit at `index` is set or not.
    pub fn is_set(&self, index: usize) -> bool {
        assert!(index < self.len, "Bitmap index {index} out of bounds");
        let arr_index = index / BITS_PER_WORD;
        let bit_index = index % BITS_PER_WORD;

        self.words[arr_index] & (1 << bit_index) > 0
    }

    /// Find the first free bit in the bitmap.
    pub fn find_free(&self) -> Option<usize> {
        for (arr_idx, bits) in self.words.iter().enumerate() {
            if bits != &u32::MAX {
                let res = (!bits).trailing_zeros();
                let index = arr_idx * BITS_PER_WORD + res as usize;
                return (index < self.len).then_some(index);
            }
        }

        None
    }

    /// Find the first run of `n` consecutive free bits without changing the bitmap.
    ///
    /// Returns the starting index, or `None` if `n` is zero or no run fits.
    /// Runs may cross word boundaries, but never include padding beyond `len`.
    pub fn find_free_range(&self, n: usize) -> Option<usize> {
        if n == 0 || n > self.len {
            return None;
        }

        let mut free_count = 0;
        for index in 0..self.len {
            if self.is_set(index) {
                free_count = 0;
            } else {
                free_count += 1;
                if free_count == n {
                    return Some(index + 1 - n);
                }
            }
        }

        None
    }

    /// Returns an iterator over all set bits and unsets them.
    pub fn drain_ones(&mut self) -> impl Iterator<Item = usize> {
        self.words.iter_mut().enumerate().flat_map(|(idx, n)| {
            core::iter::from_fn(move || {
                if *n == 0 {
                    return None;
                }

                let trailing = n.trailing_zeros();

                *n &= !(1u32 << trailing);

                Some(idx * BITS_PER_WORD + trailing as usize)
            })
        })
    }

    pub fn as_words(&self) -> &[u32] {
        &self.words
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
    fn new_minimum_size() {
        let bitmap = Bitmap::new(32);
        assert_eq!(bitmap.as_words().len(), 1);
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
    fn find_free_range_rejects_empty_and_oversized_requests() {
        for len in [0, 1, 31, 32, 33, 64] {
            let bitmap = Bitmap::new(len);
            assert_eq!(bitmap.find_free_range(0), None);
            assert_eq!(bitmap.find_free_range(len + 1), None);
            assert_eq!(bitmap.find_free_range(usize::MAX), None);
            if len != 0 {
                assert_eq!(bitmap.find_free_range(len), Some(0));
            }
        }
    }

    #[test]
    fn find_free_range_returns_first_fit_without_reserving_it() {
        let mut bitmap = Bitmap::new(16);
        bitmap.set(0);
        bitmap.set(3);
        bitmap.set(8);
        let before = bitmap.clone();

        assert_eq!(bitmap.find_free_range(1), bitmap.find_free());
        assert_eq!(bitmap.find_free_range(2), Some(1));
        assert_eq!(bitmap.find_free_range(4), Some(4));
        assert_eq!(bitmap.find_free_range(7), Some(9));
        assert_eq!(bitmap, before);
    }

    #[test]
    fn find_free_range_crosses_word_boundaries() {
        let mut bitmap = Bitmap::from_words(128, vec![u32::MAX; 4]).unwrap();
        for index in 30..100 {
            bitmap.unset(index);
        }

        for n in [4, 32, 33, 64, 70] {
            assert_eq!(bitmap.find_free_range(n), Some(30));
        }
        assert_eq!(bitmap.find_free_range(71), None);
    }

    #[test]
    fn find_free_range_does_not_combine_separate_runs() {
        let mut bitmap = Bitmap::new(64);
        for index in (1..64).step_by(2) {
            bitmap.set(index);
        }
        assert_eq!(bitmap.find_free_range(2), None);

        let full = Bitmap::from_words(64, vec![u32::MAX; 2]).unwrap();
        assert_eq!(full.find_free_range(1), None);
    }

    #[test]
    fn find_free_range_excludes_padding_in_last_word() {
        let mut bitmap = Bitmap::from_words(35, vec![u32::MAX; 2]).unwrap();
        assert_eq!(bitmap.find_free_range(1), None);
        for index in 31..35 {
            bitmap.unset(index);
        }
        assert_eq!(bitmap.find_free_range(4), Some(31));
        assert_eq!(bitmap.find_free_range(5), None);
    }

    #[test]
    fn drain_set_empty_yields_nothing() {
        let mut bitmap = Bitmap::new(64);
        let result: Vec<usize> = bitmap.drain_ones().collect();
        assert!(result.is_empty());
    }

    #[test]
    fn drain_set_yields_all_set_indices() {
        let mut bitmap = Bitmap::new(64);
        let indices = [0usize, 5, 31, 32, 63];
        for &i in &indices {
            bitmap.set(i);
        }
        let mut result: Vec<usize> = bitmap.drain_ones().collect();
        result.sort();
        assert_eq!(result, indices);
    }

    #[test]
    fn drain_set_clears_bits() {
        let mut bitmap = Bitmap::new(64);
        bitmap.set(1);
        bitmap.set(33);
        let _: Vec<_> = bitmap.drain_ones().collect();
        assert!(!bitmap.is_set(1));
        assert!(!bitmap.is_set(33));
    }

    #[test]
    fn drain_set_yields_in_ascending_order() {
        let mut bitmap = Bitmap::new(128);
        for i in [63usize, 0, 100, 32, 7] {
            bitmap.set(i);
        }
        let result: Vec<usize> = bitmap.drain_ones().collect();
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
        let result: Vec<usize> = bitmap.drain_ones().collect();
        assert_eq!(result, (0usize..64).collect::<Vec<_>>());
        for i in 0..64 {
            assert!(!bitmap.is_set(i));
        }
    }

    #[test]
    fn supports_all_boundary_lengths() {
        for len in [0, 1, 31, 32, 33, 4096] {
            let bitmap = Bitmap::new(len);
            assert_eq!(bitmap.len(), len);
            assert_eq!(bitmap.as_words().len(), len.div_ceil(32));
            assert_eq!(bitmap.find_free(), (len != 0).then_some(0));
        }
    }

    #[test]
    fn from_words_rejects_invalid_word_counts() {
        assert_eq!(
            Bitmap::from_words(33, vec![0]).unwrap_err(),
            InvalidWordCount {
                expected: 2,
                actual: 1
            }
        );
        assert!(Bitmap::from_words(0, vec![0]).is_err());
    }

    #[test]
    fn from_words_clears_final_word_padding() {
        for len in [1, 31, 33] {
            let bitmap = Bitmap::from_words(len, vec![u32::MAX; len.div_ceil(32)]).unwrap();
            assert_eq!(bitmap.find_free(), None);
            let remainder = len % 32;
            assert_eq!(
                bitmap.as_words().last().copied().unwrap(),
                if remainder == 0 {
                    u32::MAX
                } else {
                    (1u32 << remainder) - 1
                }
            );
        }
    }
}
