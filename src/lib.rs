//! Unpack a CPAN distribution release archive.
//!
//! CPAN releases are shipped as a `tar` archive — most often gzip-compressed,
//! sometimes bzip2 or xz, occasionally uncompressed — or, rarely, as a ZIP.
//! Whichever it is, it unpacks to a single top-level directory named
//! `<Dist-Name>-<version>` containing `Makefile.PL` / `Build.PL` and the rest of
//! the distribution.
//!
//! [`extract`] takes a release archive and a destination directory, detects the
//! format (from the archive's magic bytes, falling back to its extension),
//! unpacks it, and returns the path to that top-level directory:
//!
//! ```no_run
//! let dir = cpan_distribution_extractor::extract(
//!     "JSON-PP-4.16.tar.gz",
//!     "/tmp/build",
//! )?;
//! assert!(dir.join("Makefile.PL").is_file());
//! # Ok::<_, cpan_distribution_extractor::Error>(())
//! ```
//!
//! # Formats and linking
//!
//! | format               | backend                     | C code |
//! |----------------------|-----------------------------|--------|
//! | `.tar`               | [`tar`]                     | no     |
//! | `.tar.gz` / `.tgz`   | [`flate2`] (`miniz_oxide`)  | no     |
//! | `.tar.bz2` / `.tbz`  | [`bzip2`] (`libbz2-rs-sys`) | no     |
//! | `.zip`               | [`zip`] (deflate / stored)  | no     |
//! | `.tar.xz` / `.txz`   | [`xz2`] (vendored `liblzma`)| static |
//!
//! Only `.tar.xz` pulls C — a vendored `liblzma` that links statically, so the
//! consuming binary still has no shared-library dependency. Build with
//! `--no-default-features` to drop it entirely; `.tar.xz` input then returns
//! [`Error::UnsupportedFormat`].
//!
//! [`tar`]: https://docs.rs/tar
//! [`flate2`]: https://docs.rs/flate2
//! [`bzip2`]: https://docs.rs/bzip2
//! [`zip`]: https://docs.rs/zip
//! [`xz2`]: https://docs.rs/xz2

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod error;
mod format;

pub use error::{Error, Result};
pub use format::Format;

use std::fs::{self, File};
use std::io::{Cursor, Read, Seek};
use std::path::{Component, Path, PathBuf};

/// Detect the [`Format`] of the archive at `archive`: its magic bytes first,
/// then its file-name extension.
///
/// # Errors
///
/// [`Error::Io`] if the file cannot be opened or read; [`Error::UnknownFormat`]
/// if neither the bytes nor the name identify a supported format.
pub fn detect(archive: impl AsRef<Path>) -> Result<Format> {
    let archive = archive.as_ref();
    let mut head = [0u8; 512];
    let read = {
        let mut file = File::open(archive).map_err(|e| Error::io(archive, e))?;
        fill(&mut file, &mut head).map_err(|e| Error::io(archive, e))?
    };
    Format::sniff(&head[..read])
        .or_else(|| Format::from_path(archive))
        .ok_or_else(|| Error::UnknownFormat {
            path: archive.to_path_buf(),
        })
}

/// Unpack the release archive at `archive` into the directory `into`, creating
/// `into` if needed, and return the path to the distribution's single top-level
/// directory (`into` joined with that directory's name).
///
/// The format is detected with [`detect`].
///
/// # Errors
///
/// * [`Error::Io`] — the archive cannot be read, or `into` cannot be written.
/// * [`Error::UnknownFormat`] / [`Error::UnsupportedFormat`] — see [`detect`]
///   and the crate-level docs.
/// * [`Error::Layout`] — the archive does not contain exactly one top-level
///   directory.
/// * [`Error::UnsafeMember`] — a member path is absolute or contains `..`.
/// * [`Error::Zip`] — the ZIP reader rejected the archive.
pub fn extract(archive: impl AsRef<Path>, into: impl AsRef<Path>) -> Result<PathBuf> {
    let archive = archive.as_ref();
    let into = into.as_ref();
    let format = detect(archive)?;
    fs::create_dir_all(into).map_err(|e| Error::io(into, e))?;

    if format.is_zip() {
        let bytes = fs::read(archive).map_err(|e| Error::io(archive, e))?;
        return extract_zip(Cursor::new(bytes), into);
    }

    // `tar` needs two passes (validate the layout, then unpack), so hand
    // `extract_tar` a way to open a fresh decoded stream each time.
    let open = || -> Result<Box<dyn Read>> {
        let file = File::open(archive).map_err(|e| Error::io(archive, e))?;
        tar_stream(format, file)
    };
    extract_tar(open, into)
}

/// Like [`extract`], but from an already in-memory archive.
///
/// The format is detected from the bytes alone (there is no file name to fall
/// back to). The whole archive is held in memory, and cloned once for the
/// `tar` layout-validation pass.
///
/// # Errors
///
/// As [`extract`], except [`Error::UnknownFormat`] is returned whenever the
/// leading bytes match nothing known.
pub fn extract_bytes(archive: &[u8], into: impl AsRef<Path>) -> Result<PathBuf> {
    let into = into.as_ref();
    let format = Format::sniff(archive).ok_or_else(|| Error::UnknownFormat {
        path: PathBuf::from("<memory>"),
    })?;
    fs::create_dir_all(into).map_err(|e| Error::io(into, e))?;

    if format.is_zip() {
        return extract_zip(Cursor::new(archive.to_vec()), into);
    }

    let owned = archive.to_vec();
    let open = || tar_stream(format, Cursor::new(owned.clone()));
    extract_tar(open, into)
}

/// Wrap `reader` in the decoder for `format` (a `tar`-family format only).
fn tar_stream<R: Read + 'static>(format: Format, reader: R) -> Result<Box<dyn Read>> {
    Ok(match format {
        Format::Tar => Box::new(reader),
        Format::TarGz => Box::new(flate2::read::MultiGzDecoder::new(reader)),
        Format::TarBz2 => Box::new(bzip2::read::MultiBzDecoder::new(reader)),
        Format::TarXz => {
            #[cfg(feature = "xz")]
            {
                Box::new(xz2::read::XzDecoder::new(reader))
            }
            #[cfg(not(feature = "xz"))]
            {
                return Err(Error::UnsupportedFormat {
                    format,
                    feature: "xz",
                });
            }
        }
        Format::Zip => unreachable!("zip archives are handled before tar_stream"),
    })
}

/// Validate the archive's layout, then unpack it. `open` yields a fresh decoded
/// `tar` byte stream on each call.
fn extract_tar(open: impl Fn() -> Result<Box<dyn Read>>, into: &Path) -> Result<PathBuf> {
    let root = {
        let mut archive = tar::Archive::new(open()?);
        let mut root: Option<String> = None;
        for entry in archive.entries()? {
            let entry = entry?;
            let kind = entry.header().entry_type();
            // pax / GNU extension records carry metadata for the *next* entry,
            // not a path of their own.
            if kind.is_pax_global_extensions()
                || kind.is_pax_local_extensions()
                || kind.is_gnu_longname()
                || kind.is_gnu_longlink()
            {
                continue;
            }
            let path = entry.path()?;
            if let Some(name) = checked_top(&path)? {
                merge_root(&mut root, name)?;
            }
        }
        root.ok_or_else(|| Error::Layout {
            saw: "an empty archive".to_string(),
        })?
    };

    let mut archive = tar::Archive::new(open()?);
    archive.set_overwrite(true);
    archive.set_preserve_permissions(true);
    archive.set_preserve_mtime(true);
    // The `tar` crate refuses to write outside `into` (it resolves `..` and
    // checks the prefix), so traversal is covered here too.
    archive.unpack(into)?;

    Ok(into.join(root))
}

/// Read a ZIP archive: validate every member path, confirm a single top-level
/// directory, then extract.
fn extract_zip<R: Read + Seek>(reader: R, into: &Path) -> Result<PathBuf> {
    let mut zip = zip::ZipArchive::new(reader)?;

    let mut root: Option<String> = None;
    for i in 0..zip.len() {
        let entry = zip.by_index(i)?;
        let name = PathBuf::from(entry.name());
        if let Some(top) = checked_top(&name)? {
            merge_root(&mut root, top)?;
        }
    }
    let root = root.ok_or_else(|| Error::Layout {
        saw: "an empty archive".to_string(),
    })?;

    zip.extract(into)?;
    Ok(into.join(root))
}

/// Check that every component of `path` is an ordinary name (no `..`, no root,
/// no drive prefix), returning the first such component — the member's
/// top-level directory. `None` when the path is empty or only `.`.
fn checked_top(path: &Path) -> Result<Option<String>> {
    let mut top = None;
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(segment) => {
                if top.is_none() {
                    top = Some(segment.to_string_lossy().into_owned());
                }
            }
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(Error::UnsafeMember {
                    member: path.to_path_buf(),
                });
            }
        }
    }
    Ok(top)
}

/// Fold a member's top-level component into the archive-wide root, erroring if a
/// second, different one turns up.
fn merge_root(root: &mut Option<String>, name: String) -> Result<()> {
    match root {
        None => {
            *root = Some(name);
            Ok(())
        }
        Some(existing) if *existing == name => Ok(()),
        Some(existing) => Err(Error::Layout {
            saw: format!("top-level {existing:?} and {name:?}"),
        }),
    }
}

/// Read into `buf` until it is full or the reader is exhausted; returns how many
/// bytes were read.
fn fill(mut reader: impl Read, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..])? {
            0 => break,
            n => filled += n,
        }
    }
    Ok(filled)
}

#[cfg(test)]
mod tests;
