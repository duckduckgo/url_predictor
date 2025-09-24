// tests/psl_ffi.rs
#![cfg(feature = "real-psl")]

use std::{slice, str};
use std::ffi::c_char;

// Import the FFI fns from the library crate by name
use url_predictor::{ddg_up_get_psl_len, ddg_up_get_psl_ptr};

#[test]
fn psl_pointer_and_length_are_valid() {
    let ptr: *const c_char = ddg_up_get_psl_ptr();
    let len: usize = ddg_up_get_psl_len();

    assert!(!ptr.is_null(), "ddg_up_get_psl_ptr returned NULL");
    assert!(len > 0, "PSL length should be > 0");

    let bytes: &[u8] = unsafe { slice::from_raw_parts(ptr as *const u8, len) };
    let s = str::from_utf8(bytes).expect("PSL should be valid UTF-8");
    assert!(s.starts_with("//"), "PSL should start with comment lines; got: {:?}", &s[..s.len().min(80)]);
}

#[test]
fn psl_is_nul_terminated() {
    let ptr: *const c_char = ddg_up_get_psl_ptr();
    let len: usize = ddg_up_get_psl_len();
    assert_eq!(unsafe { *ptr.add(len) }, 0, "buffer must be NUL-terminated");

    let ptr2: *const c_char = unsafe { ddg_up_get_psl_ptr() };
    assert_eq!(ptr, ptr2, "PSL pointer should be stable across calls");
}

