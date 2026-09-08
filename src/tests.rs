//! Round-trip tests: build an archive of each supported format in memory, then
//! unpack it and check the distribution came back intact.

use std::fs;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};

use tempfile::TempDir;

use crate::{Error, Format, checked_top, detect, extract, extract_bytes};

const MAKEFILE_PL: &str =
    "use ExtUtils::MakeMaker;\nWriteMakefile(NAME => 'Acme::UPT::Foo', VERSION => '1.00');\n";
const MODULE_PM: &str = "package Acme::UPT::Foo;\nour $VERSION = '1.00';\n1;\n";

/// Write a minimal `Acme-UPT-Foo-1.00` distribution tree under `parent`; return
/// the path to its top directory.
fn dist_tree(parent: &Path) -> PathBuf {
    let root = parent.join("Acme-UPT-Foo-1.00");
    fs::create_dir_all(root.join("lib/Acme/UPT")).unwrap();
    fs::write(root.join("Makefile.PL"), MAKEFILE_PL).unwrap();
    fs::write(root.join("lib/Acme/UPT/Foo.pm"), MODULE_PM).unwrap();
    root
}

/// A `tar` archive placing each `src` tree under the given archive path.
fn tar_bytes(entries: &[(&str, &Path)]) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    for (archive_path, src) in entries {
        builder.append_dir_all(archive_path, src).unwrap();
    }
    builder.into_inner().unwrap()
}

fn gz(bytes: &[u8]) -> Vec<u8> {
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    enc.write_all(bytes).unwrap();
    enc.finish().unwrap()
}

fn bz2(bytes: &[u8]) -> Vec<u8> {
    let mut enc = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::fast());
    enc.write_all(bytes).unwrap();
    enc.finish().unwrap()
}

#[cfg(feature = "xz")]
fn xz(bytes: &[u8]) -> Vec<u8> {
    let mut enc = xz2::write::XzEncoder::new(Vec::new(), 2);
    enc.write_all(bytes).unwrap();
    enc.finish().unwrap()
}

/// A ZIP archive with each `(name, body)` under `root/`.
fn zip_bytes(root: &str, files: &[(&str, &str)]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    for (name, body) in files {
        writer
            .start_file(format!("{root}/{name}"), options)
            .unwrap();
        writer.write_all(body.as_bytes()).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

fn write_file(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, bytes).unwrap();
    path
}

fn assert_unpacked(dir: &Path) {
    assert!(
        dir.ends_with("Acme-UPT-Foo-1.00"),
        "unexpected return path {dir:?}"
    );
    assert!(dir.join("Makefile.PL").is_file(), "Makefile.PL missing");
    assert_eq!(
        fs::read_to_string(dir.join("lib/Acme/UPT/Foo.pm")).unwrap(),
        MODULE_PM
    );
}

#[test]
fn extract_every_tar_family_format_from_a_file() {
    let src = TempDir::new().unwrap();
    let tar = tar_bytes(&[("Acme-UPT-Foo-1.00", dist_tree(src.path()).as_path())]);

    #[cfg_attr(not(feature = "xz"), allow(unused_mut))]
    let mut cases: Vec<(&str, Vec<u8>)> = vec![
        ("dist.tar", tar.clone()),
        ("dist.tar.gz", gz(&tar)),
        ("dist.tgz", gz(&tar)),
        ("dist.tar.bz2", bz2(&tar)),
    ];
    #[cfg(feature = "xz")]
    cases.push(("dist.tar.xz", xz(&tar)));

    for (name, bytes) in cases {
        let out = TempDir::new().unwrap();
        let archive = write_file(out.path(), name, &bytes);
        let dir = extract(&archive, out.path().join("unpacked")).unwrap();
        assert_unpacked(&dir);
    }
}

#[test]
fn extract_zip_from_a_file() {
    let out = TempDir::new().unwrap();
    let bytes = zip_bytes(
        "Acme-UPT-Foo-1.00",
        &[
            ("Makefile.PL", MAKEFILE_PL),
            ("lib/Acme/UPT/Foo.pm", MODULE_PM),
        ],
    );
    let archive = write_file(out.path(), "dist.zip", &bytes);
    let dir = extract(&archive, out.path().join("unpacked")).unwrap();
    assert_unpacked(&dir);
}

#[test]
fn extract_bytes_detects_the_format_from_magic_alone() {
    let src = TempDir::new().unwrap();
    let tar_gz = gz(&tar_bytes(&[(
        "Acme-UPT-Foo-1.00",
        dist_tree(src.path()).as_path(),
    )]));
    let out = TempDir::new().unwrap();
    let dir = extract_bytes(&tar_gz, out.path()).unwrap();
    assert_unpacked(&dir);
}

#[test]
fn detect_prefers_magic_then_falls_back_to_the_extension() {
    let out = TempDir::new().unwrap();

    // Real gzip magic behind a misleading name.
    let mystery = write_file(out.path(), "mystery.bin", &gz(b"hello"));
    assert_eq!(detect(&mystery).unwrap(), Format::TarGz);

    // No usable magic: the `.tar` extension decides.
    let named = write_file(out.path(), "thing.tar", b"\x00\x00\x00\x00");
    assert_eq!(detect(&named).unwrap(), Format::Tar);
}

#[test]
fn multiple_top_level_directories_are_rejected() {
    let src = TempDir::new().unwrap();
    let a = src.path().join("a");
    let b = src.path().join("b");
    fs::create_dir_all(&a).unwrap();
    fs::create_dir_all(&b).unwrap();
    fs::write(a.join("Makefile.PL"), MAKEFILE_PL).unwrap();
    fs::write(b.join("Makefile.PL"), MAKEFILE_PL).unwrap();

    let tar_gz = gz(&tar_bytes(&[
        ("A-1.0", a.as_path()),
        ("B-1.0", b.as_path()),
    ]));
    let out = TempDir::new().unwrap();
    match extract_bytes(&tar_gz, out.path()) {
        Err(Error::Layout { .. }) => {}
        other => panic!("expected Error::Layout, got {other:?}"),
    }
}

#[test]
fn unrecognised_bytes_are_an_unknown_format() {
    let out = TempDir::new().unwrap();
    match extract_bytes(b"this is not any archive we know about", out.path()) {
        Err(Error::UnknownFormat { .. }) => {}
        other => panic!("expected Error::UnknownFormat, got {other:?}"),
    }
}

#[test]
fn checked_top_guards_against_traversal() {
    assert_eq!(
        checked_top(Path::new("Acme-UPT-Foo-1.00/lib/Foo.pm")).unwrap(),
        Some("Acme-UPT-Foo-1.00".to_string())
    );
    assert_eq!(
        checked_top(Path::new("./Acme-UPT-Foo-1.00/x")).unwrap(),
        Some("Acme-UPT-Foo-1.00".to_string())
    );
    assert_eq!(checked_top(Path::new(".")).unwrap(), None);

    for bad in ["../evil", "ok/../../evil", "/abs/evil"] {
        assert!(
            matches!(checked_top(Path::new(bad)), Err(Error::UnsafeMember { .. })),
            "{bad} should be rejected"
        );
    }
}

#[cfg(feature = "xz")]
#[test]
fn xz_round_trips_when_the_feature_is_on() {
    let src = TempDir::new().unwrap();
    let bytes = xz(&tar_bytes(&[(
        "Acme-UPT-Foo-1.00",
        dist_tree(src.path()).as_path(),
    )]));
    let out = TempDir::new().unwrap();
    assert_unpacked(&extract_bytes(&bytes, out.path()).unwrap());
}

#[cfg(not(feature = "xz"))]
#[test]
fn xz_without_the_feature_is_unsupported_not_a_crash() {
    let mut bytes = vec![0xfd, b'7', b'z', b'X', b'Z', 0x00];
    bytes.extend_from_slice(&[0u8; 64]);
    let out = TempDir::new().unwrap();
    match extract_bytes(&bytes, out.path()) {
        Err(Error::UnsupportedFormat {
            feature: "xz",
            format: Format::TarXz,
        }) => {}
        other => panic!("expected Error::UnsupportedFormat, got {other:?}"),
    }
}
