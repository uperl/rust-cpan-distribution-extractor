# cpan-distribution-extractor

Unpack a CPAN distribution release archive — `.tar`, `.tar.gz` / `.tgz`,
`.tar.bz2` / `.tbz`, `.tar.xz` / `.txz`, or `.zip` — into a directory, and get
back the path to the distribution's single top-level directory.

```rust
use cpan_distribution_extractor::extract;

let dir = extract("JSON-PP-4.16.tar.gz", "/tmp/build")?;
assert!(dir.join("Makefile.PL").is_file());
```

The format is detected from the archive's magic bytes, falling back to its
file-name extension. `extract_bytes` does the same from an in-memory archive;
`detect` just reports the `Format`.

## No shelling out, no shared libraries

| format               | backend                     | C code |
|----------------------|-----------------------------|--------|
| `.tar`               | `tar`                       | no     |
| `.tar.gz` / `.tgz`   | `flate2` (`miniz_oxide`)    | no     |
| `.tar.bz2` / `.tbz`  | `bzip2` (`libbz2-rs-sys`)   | no     |
| `.zip`               | `zip` (deflate / stored)    | no     |
| `.tar.xz` / `.txz`   | `xz2` (vendored `liblzma`)  | static |

Only `.tar.xz` involves C: a `liblzma` compiled from vendored source and linked
statically, so a binary that depends on this crate still has no runtime
shared-library dependency. It is behind the default `xz` feature — build with
`--no-default-features` to drop it (and the C toolchain requirement), and
`.tar.xz` input then returns `Error::UnsupportedFormat`.

## Guarantees

* The archive must unpack to **exactly one top-level directory** (as every CPAN
  release does); otherwise `Error::Layout`.
* Member paths are checked for `..` and absolute components before extraction
  (`Error::UnsafeMember`), on top of the `tar` and `zip` crates' own traversal
  protection.

## License

MIT
