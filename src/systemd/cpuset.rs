// Copyright (c) 2025 Ant Group
//
// SPDX-License-Identifier: Apache-2.0 or MIT
//

use bit_vec::BitVec;

use crate::systemd::error::{Error, Result};
use crate::systemd::{ALLOWED_CPUS, ALLOWED_MEMORY_NODES};

const BYTE_IN_BITS: usize = 8;

/// Returns the property for cpuset CPUs.
pub fn cpus(cpus: &str) -> Result<(&'static str, Vec<u8>)> {
    let mask = convert_list_to_mask(cpus)?;
    Ok((ALLOWED_CPUS, mask))
}

/// Returns the property for cpuset memory nodes.
pub fn mems(mems: &str) -> Result<(&'static str, Vec<u8>)> {
    let mask = convert_list_to_mask(mems)?;
    Ok((ALLOWED_MEMORY_NODES, mask))
}

/// Convert cpuset cpus/mems from the string in comma-separated list format
/// to bitmask restored in `Vec<u8>`, see [1].
///
/// 1: https://man7.org/linux/man-pages/man7/cpuset.7.html
///
/// # Arguments
///
/// * `list` - A string slice that holds the list of CPUs in the format
///   "0-3,5,7".
fn convert_list_to_mask(list: &str) -> Result<Vec<u8>> {
    let mut bit_vec = BitVec::from_elem(8, false);

    let local_idx =
        |index: usize| -> usize { index / BYTE_IN_BITS * BYTE_IN_BITS + 7 - index % BYTE_IN_BITS };

    for part1 in list.split(',') {
        let range: Vec<&str> = part1.split('-').collect();
        match range.len() {
            // x-
            1 => {
                let left: usize = range[0].parse().map_err(|_| Error::InvalidArgument)?;

                while left >= bit_vec.len() {
                    bit_vec.grow(BYTE_IN_BITS, false);
                }
                bit_vec.set(local_idx(left), true);
            }
            // x-y
            2 => {
                let left: usize = range[0].parse().map_err(|_| Error::InvalidArgument)?;
                let right: usize = range[1].parse().map_err(|_| Error::InvalidArgument)?;

                while right >= bit_vec.len() {
                    bit_vec.grow(BYTE_IN_BITS, false);
                }

                for index in left..=right {
                    bit_vec.set(local_idx(index), true);
                }
            }
            _ => {
                return Err(Error::InvalidArgument);
            }
        }
    }

    let mut mask = bit_vec.to_bytes();
    mask.reverse();

    Ok(mask)
}

#[cfg(test)]
mod tests {
    use crate::systemd::cpuset::convert_list_to_mask;

    #[test]
    fn test_convert_list_to_mask() {
        let mask = convert_list_to_mask("2-4").unwrap();
        assert_eq!(vec![0b00011100_u8], mask);

        let mask = convert_list_to_mask("1,7").unwrap();
        assert_eq!(vec![0b10000010_u8], mask);

        let mask = convert_list_to_mask("0-4,9").unwrap();
        assert_eq!(vec![0b00000010_u8, 0b00011111_u8], mask);

        assert!(convert_list_to_mask("1-3-4").is_err());

        assert!(convert_list_to_mask("1-3,,").is_err());
    }
}
