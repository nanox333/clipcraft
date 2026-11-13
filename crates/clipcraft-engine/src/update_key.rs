//! The key that signs releases.
//!
//! The release workflow signs `SHA256SUMS.txt` with a private key kept as a repository secret, and
//! publishes the signature as `SHA256SUMS.txt.sig`. The matching public key is this constant: once
//! it is set, ClipCraft installs an update only when that signature checks out, so a tampered
//! release or a stolen account that cannot sign is not trusted.
//!
//! To turn signing on, run `cargo xtask sign-keygen` once. It prints the public key (put it here)
//! and the private key (store it as the `CLIPCRAFT_SIGNING_KEY` secret, and nowhere else). See
//! `docs/release-checklist.md`.

/// The release public key, as 64 hexadecimal characters, or `None` until signing is set up. While
/// it is `None` the checksum published beside the download is the only check.
pub const RELEASE_PUBLIC_KEY: Option<&str> = None;
