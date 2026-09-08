//! Archive format detection.

use std::fmt;
use std::path::Path;

/// A CPAN release archive format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Format {
    /// An uncompressed `tar` archive.
    Tar,
    /// A gzip-compressed `tar` archive (`.tar.gz`, `.tgz`).
    TarGz,
    /// A bzip2-compressed `tar` archive (`.tar.bz2`, `.tbz`, `.tbz2`).
    TarBz2,
    /// An xz-compressed `tar` archive (`.tar.xz`, `.txz`).
    TarXz,
    /// A ZIP archive.
    Zip,
}

impl Format {
    /// Recognise the format from an archive's leading bytes.
    ///
    /// Needs the first 262 bytes to recognise an uncompressed `tar`; a handful
    /// are enough for the compressed and ZIP formats. Returns `None` when the
    /// bytes match nothing known.
    pub fn sniff(bytes: &[u8]) -> Option<Format> {
        if bytes.starts_with(&[0x1f, 0x8b]) {
            Some(Format::TarGz)
        } else if bytes.starts_with(b"BZh") {
            Some(Format::TarBz2)
        } else if bytes.starts_with(&[0xfd, b'7', b'z', b'X', b'Z', 0x00]) {
            Some(Format::TarXz)
        } else if bytes.starts_with(b"PK\x03\x04")
            || bytes.starts_with(b"PK\x05\x06")
            || bytes.starts_with(b"PK\x07\x08")
        {
            Some(Format::Zip)
        } else if bytes.len() >= 262 && &bytes[257..262] == b"ustar" {
            Some(Format::Tar)
        } else {
            None
        }
    }

    /// Guess the format from a path's file-name extension. Case-insensitive.
    /// Returns `None` for an unrecognised extension.
    pub fn from_path(path: impl AsRef<Path>) -> Option<Format> {
        let name = path.as_ref().file_name()?.to_str()?.to_ascii_lowercase();
        let ends = |suffix: &str| name.ends_with(suffix);
        if ends(".tar.gz") || ends(".tgz") {
            Some(Format::TarGz)
        } else if ends(".tar.bz2") || ends(".tbz") || ends(".tbz2") {
            Some(Format::TarBz2)
        } else if ends(".tar.xz") || ends(".txz") {
            Some(Format::TarXz)
        } else if ends(".zip") {
            Some(Format::Zip)
        } else if ends(".tar") {
            Some(Format::Tar)
        } else {
            None
        }
    }

    /// Whether this format is a ZIP archive (the rest are `tar` based).
    pub(crate) fn is_zip(self) -> bool {
        matches!(self, Format::Zip)
    }
}

impl fmt::Display for Format {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Format::Tar => "tar",
            Format::TarGz => "tar.gz",
            Format::TarBz2 => "tar.bz2",
            Format::TarXz => "tar.xz",
            Format::Zip => "zip",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::Format;

    #[test]
    fn sniff_recognises_each_magic() {
        assert_eq!(Format::sniff(&[0x1f, 0x8b, 0x08, 0]), Some(Format::TarGz));
        assert_eq!(Format::sniff(b"BZh91AY&SY"), Some(Format::TarBz2));
        assert_eq!(
            Format::sniff(&[0xfd, b'7', b'z', b'X', b'Z', 0x00, 0x00]),
            Some(Format::TarXz)
        );
        assert_eq!(Format::sniff(b"PK\x03\x04rest"), Some(Format::Zip));

        let mut ustar = vec![0u8; 300];
        ustar[257..262].copy_from_slice(b"ustar");
        assert_eq!(Format::sniff(&ustar), Some(Format::Tar));

        assert_eq!(Format::sniff(b"not an archive"), None);
        // A `ustar` marker needs the full header to be present.
        assert_eq!(Format::sniff(b"ustar"), None);
    }

    #[test]
    fn from_path_reads_the_extension() {
        for (name, want) in [
            ("Foo-1.00.tar.gz", Format::TarGz),
            ("Foo-1.00.TGZ", Format::TarGz),
            ("Foo-1.00.tar.bz2", Format::TarBz2),
            ("Foo-1.00.tbz2", Format::TarBz2),
            ("Foo-1.00.tar.xz", Format::TarXz),
            ("Foo-1.00.txz", Format::TarXz),
            ("Foo-1.00.zip", Format::Zip),
            ("Foo-1.00.tar", Format::Tar),
        ] {
            assert_eq!(Format::from_path(name), Some(want), "{name}");
        }
        assert_eq!(Format::from_path("Foo-1.00.rar"), None);
        assert_eq!(Format::from_path("Foo-1.00"), None);
    }
}
