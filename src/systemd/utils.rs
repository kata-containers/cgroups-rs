// Copyright (c) 2025 Ant Group
//
// SPDX-License-Identifier: Apache-2.0 or MIT
//

use crate::systemd::error::{Error, Result};
use crate::systemd::{SCOPE_SUFFIX, SLICE_SUFFIX};

/// Check if a systemd unit name is a slice unit.
pub fn is_slice_unit(name: &str) -> bool {
    name.ends_with(SLICE_SUFFIX)
}

/// Check if a systemd unit name is a scope unit.
pub fn is_scope_unit(name: &str) -> bool {
    name.ends_with(SCOPE_SUFFIX)
}

/// Expand a slice name to a full path in the filesystem.
///
/// # Arguments
///
/// * `slice` - A string slice that holds the slice name in the format
///   "xxx-yyy-zzz.slice".
///
/// # Returns
///
/// A string that represents the full path of the slice in the filesystem.
/// In the above case, the value would be
/// "xxx.slice/xxx-yyy.slice/xxx-yyy-zzz.slice".
pub fn expand_slice(slice: &str) -> Result<String> {
    // Name has to end with ".slice", but can't be just ".slice".
    if !slice.ends_with(SLICE_SUFFIX) || slice.len() < SLICE_SUFFIX.len() {
        return Err(Error::InvalidArgument);
    }

    // Path-separators are not allowed.
    if slice.contains('/') {
        return Err(Error::InvalidArgument);
    }

    let name = slice.trim_end_matches(SLICE_SUFFIX);

    // If input was -.slice, we should just return root now
    if name == "-" {
        return Ok("".to_string());
    }

    let mut slice_path = String::new();
    let mut prefix = String::new();
    for sub_slice in name.split('-') {
        if sub_slice.is_empty() {
            return Err(Error::InvalidArgument);
        }

        slice_path = format!("{}/{}{}{}", slice_path, prefix, sub_slice, SLICE_SUFFIX);
        prefix = format!("{}{}-", prefix, sub_slice);
    }

    // We need a relative path, so remove the first slash.
    slice_path.remove(0);

    Ok(slice_path)
}

#[cfg(test)]
mod tests {
    use crate::systemd::utils::*;

    #[test]
    fn test_is_slice_unit() {
        assert!(is_slice_unit("test.slice"));
        assert!(!is_slice_unit("test.scope"));
    }

    #[test]
    fn test_is_scope_unit() {
        assert!(is_scope_unit("test.scope"));
        assert!(!is_scope_unit("test.slice"));
    }

    #[test]
    fn test_expand_slice() {
        assert_eq!(expand_slice("test.slice").unwrap(), "test.slice");
        assert_eq!(
            expand_slice("test-1.slice").unwrap(),
            "test.slice/test-1.slice"
        );
        assert_eq!(
            expand_slice("test-1-test-2.slice").unwrap(),
            "test.slice/test-1.slice/test-1-test.slice/test-1-test-2.slice"
        );
        assert_eq!(
            expand_slice("slice-slice.slice").unwrap(),
            "slice.slice/slice-slice.slice"
        );
        assert_eq!(expand_slice("-.slice").unwrap(), "");
        assert!(expand_slice("invalid/slice").is_err());
        assert!(expand_slice("invalid-slice").is_err());
    }
}
