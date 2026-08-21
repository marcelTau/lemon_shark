extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;

use crate::PAGE_SIZE;

/// A non-empty half-open range of physical addresses: `[start, end)`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct PhysRange {
    start: usize,
    end: usize,
}

impl core::fmt::Debug for PhysRange {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "PhysRange {{ start: {:#x}, end: {:#x} }}",
            self.start, self.end
        )
    }
}

impl PhysRange {
    pub fn from_start_size(start: usize, size: usize) -> Result<Self, PhysRangeError> {
        let end = start
            .checked_add(size)
            .ok_or(PhysRangeError::AddressOverflow)?;

        if start >= end {
            return Err(PhysRangeError::EmptyRange);
        }

        Ok(Self { start, end })
    }

    pub const fn start(self) -> usize {
        self.start
    }

    pub const fn end(self) -> usize {
        self.end
    }

    pub const fn size(self) -> usize {
        self.end - self.start
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhysRangeError {
    AddressOverflow,
    EmptyRange,
}

/// Sort ranges and merge ranges which overlap or directly touch.
pub fn normalize_ranges(mut ranges: Vec<PhysRange>) -> Vec<PhysRange> {
    ranges.sort_by_key(|range| range.start);

    let mut normalized: Vec<PhysRange> = Vec::new();

    for range in ranges {
        match normalized.last_mut() {
            Some(previous) if range.start <= previous.end => {
                previous.end = previous.end.max(range.end);
            }
            _ => normalized.push(range),
        }
    }

    normalized
}

fn align_up(addr: usize) -> Option<usize> {
    addr.checked_add(PAGE_SIZE - 1)
        .map(|addr| addr & !(PAGE_SIZE - 1))
}

fn align_down(addr: usize) -> usize {
    addr & !(PAGE_SIZE - 1)
}

/// Subtract reserved byte ranges from RAM and return only complete pages.
///
/// Overlapping and adjacent input ranges are normalized before subtraction.
pub fn usable_memory_ranges(memory: &[PhysRange], reserved: &[PhysRange]) -> Vec<PhysRange> {
    let mut usable = Vec::new();
    let memory = normalize_ranges(memory.to_vec());
    let reserved = normalize_ranges(reserved.to_vec());

    for memory_range in memory {
        let mut fragments = vec![memory_range];

        for reserved_range in reserved.iter().copied() {
            let mut next = Vec::new();

            for fragment in fragments {
                if reserved_range.end <= fragment.start || reserved_range.start >= fragment.end {
                    next.push(fragment);
                    continue;
                }

                if fragment.start < reserved_range.start {
                    next.push(PhysRange {
                        start: fragment.start,
                        end: reserved_range.start.min(fragment.end),
                    });
                }

                if reserved_range.end < fragment.end {
                    next.push(PhysRange {
                        start: reserved_range.end.max(fragment.start),
                        end: fragment.end,
                    });
                }
            }

            fragments = next;
        }

        for fragment in fragments {
            let Some(start) = align_up(fragment.start) else {
                continue;
            };
            let end = align_down(fragment.end);

            if start < end {
                usable.push(PhysRange { start, end });
            }
        }
    }

    usable.sort_by_key(|range| range.start);
    usable
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(start: usize, end: usize) -> PhysRange {
        PhysRange::from_start_size(start, end - start).unwrap()
    }

    fn ranges(values: &[(usize, usize)]) -> Vec<PhysRange> {
        values
            .iter()
            .map(|&(start, end)| range(start, end))
            .collect()
    }

    fn assert_usable(
        memory: &[(usize, usize)],
        reserved: &[(usize, usize)],
        expected: &[(usize, usize)],
    ) {
        assert_eq!(
            usable_memory_ranges(&ranges(memory), &ranges(reserved)),
            ranges(expected)
        );
    }

    #[test]
    fn no_memory_produces_no_usable_ranges() {
        assert_usable(&[], &[(0x1000, 0x2000)], &[]);
    }

    #[test]
    fn no_reservations_preserves_aligned_memory() {
        assert_usable(&[(0x1000, 0x5000)], &[], &[(0x1000, 0x5000)]);
    }

    #[test]
    fn partial_pages_at_memory_boundaries_are_discarded() {
        assert_usable(&[(0x1001, 0x4fff)], &[], &[(0x2000, 0x4000)]);
        assert_usable(&[(0x1001, 0x1fff)], &[], &[]);
    }

    #[test]
    fn reservations_outside_memory_do_not_change_it() {
        assert_usable(
            &[(0x3000, 0x7000)],
            &[(0x1000, 0x3000), (0x7000, 0x9000)],
            &[(0x3000, 0x7000)],
        );
    }

    #[test]
    fn exact_or_containing_reservation_removes_entire_region() {
        assert_usable(&[(0x3000, 0x7000)], &[(0x3000, 0x7000)], &[]);
        assert_usable(&[(0x3000, 0x7000)], &[(0x1000, 0x9000)], &[]);
    }

    #[test]
    fn reservation_overlapping_start_keeps_suffix() {
        assert_usable(
            &[(0x3000, 0x8000)],
            &[(0x1000, 0x5000)],
            &[(0x5000, 0x8000)],
        );
    }

    #[test]
    fn reservation_overlapping_end_keeps_prefix() {
        assert_usable(
            &[(0x3000, 0x8000)],
            &[(0x6000, 0xa000)],
            &[(0x3000, 0x6000)],
        );
    }

    #[test]
    fn reservation_inside_memory_splits_region() {
        assert_usable(
            &[(0x1000, 0x9000)],
            &[(0x4000, 0x6000)],
            &[(0x1000, 0x4000), (0x6000, 0x9000)],
        );
    }

    #[test]
    fn unaligned_reservation_discards_every_page_it_touches() {
        assert_usable(
            &[(0x1000, 0x6000)],
            &[(0x2800, 0x3800)],
            &[(0x1000, 0x2000), (0x4000, 0x6000)],
        );
    }

    #[test]
    fn overlapping_and_nested_reservations_are_merged() {
        assert_usable(
            &[(0x1000, 0xb000)],
            &[
                (0x7000, 0x9000),
                (0x3000, 0x6000),
                (0x5000, 0x8000),
                (0x4000, 0x5000),
            ],
            &[(0x1000, 0x3000), (0x9000, 0xb000)],
        );
    }

    #[test]
    fn adjacent_reservations_are_merged() {
        assert_usable(
            &[(0x1000, 0xa000)],
            &[(0x3000, 0x5000), (0x5000, 0x7000)],
            &[(0x1000, 0x3000), (0x7000, 0xa000)],
        );
    }

    #[test]
    fn disjoint_unordered_reservations_create_multiple_fragments() {
        assert_usable(
            &[(0x1000, 0xc000)],
            &[(0x8000, 0x9000), (0x3000, 0x4000)],
            &[(0x1000, 0x3000), (0x4000, 0x8000), (0x9000, 0xc000)],
        );
    }

    #[test]
    fn reservation_can_span_multiple_memory_regions_and_holes() {
        assert_usable(
            &[(0x1000, 0x5000), (0x8000, 0xc000)],
            &[(0x3000, 0xa000)],
            &[(0x1000, 0x3000), (0xa000, 0xc000)],
        );
    }

    #[test]
    fn overlapping_and_adjacent_memory_descriptors_are_normalized() {
        assert_usable(
            &[(0x1001, 0x2800), (0x2800, 0x5000), (0x4000, 0x7000)],
            &[],
            &[(0x2000, 0x7000)],
        );
    }

    #[test]
    fn unordered_memory_descriptors_produce_sorted_ranges() {
        assert_usable(
            &[(0x9000, 0xb000), (0x1000, 0x3000)],
            &[],
            &[(0x1000, 0x3000), (0x9000, 0xb000)],
        );
    }

    #[test]
    fn address_space_end_is_handled_without_overflow() {
        let last_page = usize::MAX & !(PAGE_SIZE - 1);

        assert_usable(
            &[(last_page - PAGE_SIZE, usize::MAX)],
            &[],
            &[(last_page - PAGE_SIZE, last_page)],
        );
        assert_usable(&[(last_page + 1, usize::MAX)], &[], &[]);
    }
}
