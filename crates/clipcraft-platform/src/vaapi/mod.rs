//! H.264 hardware encoding through VA-API, the interface Mesa (AMD) and Intel's media drivers
//! offer on Linux. libva is loaded at run time; a computer without it simply has no VA-API
//! encoder. See ADR 0008.
//!
//! This is the only module on Linux that uses `unsafe`: the calls into libva. Each block says why
//! it is sound.
#![allow(unsafe_code)]
#![warn(clippy::undocumented_unsafe_blocks)]

pub mod encoder;
pub mod ffi;
