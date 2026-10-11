//! The OpenH264 build Rustshot downloads on Linux (when ffmpeg is absent
//! and the user asks for it): Cisco's own binary, served over plain HTTP
//! by Cisco's design from [`BASE_URL`], so each file is pinned here by its
//! SHA-256, both as downloaded (`.bz2`) and unpacked (`.so`). Nothing else
//! is ever fetched or loaded.
//!
//! How the hashes were checked (v2.6.0, 2026-10-10): each `.bz2` was
//! fetched from Cisco and hashed; its unpacked `.so` matches the MD5 in
//! Cisco's signed `<file>.signed.md5.txt` next to it; CI downloads the
//! pinned file again and refuses it unless both SHA-256s match.
//!
//! A new release changes the API structures (`SEncParamExt` grew in 2.6.0,
//! the soname went from 7 to 8): the encoder's struct layouts follow
//! [`VERSION`] and the loader checks the library reports it.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))] // unit-tested everywhere, used on Linux

/// The pinned OpenH264 release.
pub const VERSION: (u32, u32, u32) = (2, 6, 0);

/// Where Cisco serves the binaries (HTTP by Cisco's design).
pub const BASE_URL: &str = "http://ciscobinary.openh264.org/";

/// One pinned binary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pin {
    /// `std::env::consts::ARCH` of the machines it runs on.
    pub arch: &'static str,
    /// The file Cisco serves (under [`BASE_URL`]).
    pub file: &'static str,
    /// SHA-256 of that file (hex).
    pub bz2_sha256: &'static str,
    /// Its size in bytes (the download is cut off past it).
    pub bz2_size: u64,
    /// The unpacked library's name, SHA-256 (hex) and size.
    pub so: &'static str,
    pub so_sha256: &'static str,
    pub so_size: u64,
}

impl Pin {
    /// The download URL: [`BASE_URL`] and the file name, nothing else.
    pub fn url(&self) -> String {
        format!("{BASE_URL}{}", self.file)
    }
}

/// Every pinned binary (Linux x86_64 and aarch64).
pub const PINS: [Pin; 2] = [
    Pin {
        arch: "x86_64",
        file: "libopenh264-2.6.0-linux64.8.so.bz2",
        bz2_sha256: "27ab53323c110b76214c1c72222f459d17febbcd1e252136cadc292b0308d75b",
        bz2_size: 634_264,
        so: "libopenh264-2.6.0-linux64.8.so",
        so_sha256: "2f0cde7c6a6abcf5cae76942894ea42897fa677bce4ed6c91a24dd1b041d5f04",
        so_size: 1_731_128,
    },
    Pin {
        arch: "aarch64",
        file: "libopenh264-2.6.0-linux-arm64.8.so.bz2",
        bz2_sha256: "a78aea7970150f46bcd3bb7994c9e6dd90bd7a9ea785920f5a73f6964e3fcda7",
        bz2_size: 619_371,
        so: "libopenh264-2.6.0-linux-arm64.8.so",
        so_sha256: "12e7b33623667cdab0e575170c147b1b36eadb77d0d2aa7ceb5afd3e58902140",
        so_size: 1_492_120,
    },
];

/// The pin for `arch` (`std::env::consts::ARCH`), if there is one.
pub fn pin_for(arch: &str) -> Option<&'static Pin> {
    PINS.iter().find(|p| p.arch == arch)
}

/// The pin for this machine.
pub fn this_machine() -> Option<&'static Pin> {
    pin_for(std::env::consts::ARCH)
}

/// Whether `url` is exactly one of the pinned downloads.
pub fn is_pinned_url(url: &str) -> bool {
    PINS.iter().any(|p| p.url() == url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pins_are_looked_up_per_arch() {
        let x = pin_for("x86_64").unwrap();
        assert_eq!(x.url(), "http://ciscobinary.openh264.org/libopenh264-2.6.0-linux64.8.so.bz2");
        assert_eq!(x.so, "libopenh264-2.6.0-linux64.8.so");
        let a = pin_for("aarch64").unwrap();
        assert_eq!(a.url(), "http://ciscobinary.openh264.org/libopenh264-2.6.0-linux-arm64.8.so.bz2");
        assert!(pin_for("riscv64").is_none());
        assert!(pin_for("").is_none());
        for p in PINS {
            for h in [p.bz2_sha256, p.so_sha256] {
                assert!(crate::sha256::parse_hex(h).is_some(), "{h}");
            }
            assert!(p.file.starts_with("libopenh264-2.6.0-") && p.file.ends_with(".so.bz2"));
            assert_eq!(format!("{}.bz2", p.so), p.file);
            assert!(p.bz2_size > 0 && p.so_size > p.bz2_size);
        }
    }

    #[test]
    fn only_the_pinned_urls_are_allowed() {
        assert!(is_pinned_url("http://ciscobinary.openh264.org/libopenh264-2.6.0-linux64.8.so.bz2"));
        for bad in [
            "https://ciscobinary.openh264.org/libopenh264-2.6.0-linux64.8.so.bz2",
            "http://ciscobinary.openh264.org/libopenh264-2.5.0-linux64.7.so.bz2",
            "http://ciscobinary.openh264.org.evil/libopenh264-2.6.0-linux64.8.so.bz2",
            "http://ciscobinary.openh264.org/../libopenh264-2.6.0-linux64.8.so.bz2",
            "http://ciscobinary.openh264.org/libopenh264-2.6.0-linux64.8.so.bz2?x",
            "",
        ] {
            assert!(!is_pinned_url(bad), "{bad}");
        }
    }
}
