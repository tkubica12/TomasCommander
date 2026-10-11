use crate::files::{self, FileResult, Scope};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::{Cursor, Read},
    path::Path,
};

pub const MAX_ARCHIVE_BYTES: u64 = 32 * 1_048_576;
const MAX_ENTRIES: usize = 10_000;
const MAX_NODES: usize = 20_000;
const MAX_EXPANDED_BYTES: u64 = 256 * 1_048_576;

#[derive(Clone, Debug)]
pub struct ArchiveEntry {
    pub path: String,
    pub directory: bool,
    pub size: u64,
}

pub struct Inventory {
    pub entries: Vec<ArchiveEntry>,
}

pub fn inventory(
    scope: &Scope,
    path: &Path,
    cancelled: impl Fn() -> bool,
) -> FileResult<Inventory> {
    if cancelled() {
        return Err("Archive read cancelled.".into());
    }
    let path = scope.resolve(path)?;
    let before = files::fingerprint(&path)?;
    if before.directory || before.size > MAX_ARCHIVE_BYTES {
        return Err("ZIP must be a regular file no larger than 32 MiB.".into());
    }
    let mut bytes = Vec::new();
    File::open(&path)
        .map_err(|error| format!("Open ZIP: {error}"))?
        .take(MAX_ARCHIVE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Read ZIP: {error}"))?;
    if bytes.len() as u64 > MAX_ARCHIVE_BYTES {
        return Err("ZIP grew beyond the input limit.".into());
    }
    let result = inventory_bytes(&bytes, &cancelled)?;
    if files::fingerprint(&path)? != before {
        return Err("ZIP changed while reading its directory. Reopen it.".into());
    }
    Ok(result)
}

pub(crate) fn inventory_bytes(bytes: &[u8], cancelled: impl Fn() -> bool) -> FileResult<Inventory> {
    preflight(bytes)?;
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|error| format!("Read ZIP directory: {error}"))?;
    if archive.len() > MAX_ENTRIES {
        return Err("ZIP exceeds the entry limit.".into());
    }
    let mut nodes = BTreeMap::<String, ArchiveEntry>::new();
    let mut declared = BTreeSet::new();
    let mut expanded = 0_u64;
    for index in 0..archive.len() {
        if cancelled() {
            return Err("Archive read cancelled.".into());
        }
        let entry = archive
            .by_index(index)
            .map_err(|error| format!("Read ZIP entry #{index}: {error}"))?;
        if !matches!(
            entry.compression(),
            zip::CompressionMethod::Stored | zip::CompressionMethod::Deflated
        ) {
            return Err(
                "ZIP contains unsupported compression; only Store/Deflate are enabled.".into(),
            );
        }
        if entry.unix_mode().is_some_and(|mode| {
            let kind = mode & 0o170000;
            kind != 0 && kind != 0o100000 && kind != 0o040000
        }) {
            return Err("ZIP links and special files are not supported.".into());
        }
        let path = validate_name(entry.name())?;
        if !declared.insert(path.to_lowercase()) {
            return Err(format!("Duplicate or case-ambiguous ZIP entry: {path}"));
        }
        expanded = expanded
            .checked_add(entry.size())
            .ok_or("ZIP declared expanded size overflow.")?;
        if expanded > MAX_EXPANDED_BYTES || entry.size() > 16 * 1_048_576 {
            return Err(
                "ZIP exceeds declared expanded-size limits (256 MiB total / 16 MiB per entry)."
                    .into(),
            );
        }
        let components: Vec<_> = path.split('/').collect();
        for length in 1..=components.len() {
            let name = components[..length].join("/");
            let directory = length < components.len() || entry.is_dir();
            let key = name.to_lowercase();
            if let Some(previous) = nodes.get(&key) {
                if previous.path != name || previous.directory != directory {
                    return Err(format!("Conflicting virtual ZIP path: {name}"));
                }
            } else {
                nodes.insert(
                    key,
                    ArchiveEntry {
                        path: name,
                        directory,
                        size: if directory { 0 } else { entry.size() },
                    },
                );
            }
            if nodes.len() > MAX_NODES {
                return Err("ZIP exceeds the virtual-directory limit.".into());
            }
        }
    }
    if cancelled() {
        return Err("Archive read cancelled.".into());
    }
    Ok(Inventory {
        entries: nodes.into_values().collect(),
    })
}

pub(crate) fn validate_name(name: &str) -> FileResult<String> {
    let name = name.strip_suffix('/').unwrap_or(name);
    if name.is_empty() || name.encode_utf16().count() > 1024 {
        return Err("ZIP has an empty or oversized path.".into());
    }
    let components: Vec<_> = name.split('/').collect();
    if components.len() > 32 {
        return Err("ZIP nesting exceeds 32 components.".into());
    }
    for component in components {
        let base = component
            .split('.')
            .next()
            .unwrap_or("")
            .to_ascii_uppercase();
        let reserved = matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || ["COM", "LPT"].iter().any(|prefix| {
                base.strip_prefix(prefix).is_some_and(|suffix| {
                    matches!(
                        suffix,
                        "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                    )
                })
            });
        if component.is_empty()
            || matches!(component, "." | "..")
            || component.ends_with(['.', ' '])
            || component.encode_utf16().count() > 255
            || reserved
            || component.chars().any(|character| {
                character.is_control()
                    || matches!(character, '<' | '>' | ':' | '"' | '\\' | '|' | '?' | '*')
            })
        {
            return Err(format!(
                "ZIP has an unsafe/unsupported Windows path: {name}"
            ));
        }
    }
    Ok(name.to_owned())
}

fn preflight(bytes: &[u8]) -> FileResult<()> {
    let start = bytes.len().saturating_sub(65_557);
    let end = bytes.len().saturating_sub(22);
    let position = (start..=end)
        .rev()
        .find(|&offset| {
            bytes.get(offset..offset + 4) == Some(b"PK\x05\x06")
                && bytes.get(offset + 20..offset + 22).is_some_and(|comment| {
                    offset + 22 + u16::from_le_bytes([comment[0], comment[1]]) as usize
                        == bytes.len()
                })
        })
        .ok_or("ZIP has no complete end-of-directory record.")?;
    let word =
        |offset| u16::from_le_bytes([bytes[position + offset], bytes[position + offset + 1]]);
    let dword = |offset| {
        u32::from_le_bytes(
            bytes[position + offset..position + offset + 4]
                .try_into()
                .expect("bounded ZIP record"),
        )
    };
    if word(4) != 0
        || word(6) != 0
        || word(8) != word(10)
        || word(10) as usize > MAX_ENTRIES
        || (position >= 20 && &bytes[position - 20..position - 16] == b"PK\x06\x07")
    {
        return Err("ZIP64, split archives or excessive entries are not supported.".into());
    }
    let directory_size = dword(12) as u64;
    let directory_offset = dword(16) as u64;
    if directory_size > 8 * 1_048_576 || directory_offset + directory_size > position as u64 {
        return Err("ZIP central directory exceeds bounds.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn zip_bytes(names: &[&str]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for name in names {
            writer
                .start_file(
                    *name,
                    zip::write::SimpleFileOptions::default()
                        .compression_method(zip::CompressionMethod::Stored),
                )
                .unwrap();
            writer.write_all(b"fixture").unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    #[test]
    fn paths_reject_windows_aliases_traversal_and_virtual_limits() {
        for name in [
            "../x", "/x", "a//b", "a/./b", "a\\b", "C:/x", "x:stream", "CON.txt", "a/NUL", "x. ",
            "x.", "a\0b", "",
        ] {
            assert!(validate_name(name).is_err(), "{name}");
        }
        assert_eq!(
            validate_name("Příliš žluťoučký/hello.txt").unwrap(),
            "Příliš žluťoučký/hello.txt"
        );
        assert!(validate_name(&"a/".repeat(33)).is_err());
        for bytes in [Vec::new(), b"PK\x05\x06".to_vec(), vec![0; 22]] {
            assert!(preflight(&bytes).is_err());
        }
    }

    #[test]
    fn real_inventory_is_virtual_bounded_cancelled_and_rejects_unsafe_archives() {
        use std::{cell::Cell, fs};
        let root = std::env::temp_dir().join(format!("tomas-zip-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        let scope = Scope::fixture(&root).unwrap();
        let path = root.join("fixture.zip");
        let bytes = zip_bytes(&["Příliš žluťoučký/nested/file.txt", "root.txt"]);
        fs::write(&path, &bytes).unwrap();
        let listing = inventory(&scope, &path, || false).unwrap();
        assert_eq!(listing.entries.len(), 4);
        assert!(
            listing
                .entries
                .iter()
                .any(|entry| entry.path == "Příliš žluťoučký/nested" && entry.directory)
        );
        assert!(
            listing
                .entries
                .iter()
                .any(|entry| entry.path == "root.txt" && entry.size == 7)
        );
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert!(inventory(&scope, &path, || true).is_err());
        let calls = Cell::new(0);
        assert!(
            inventory(&scope, &path, || {
                let count = calls.get();
                calls.set(count + 1);
                count >= 2
            })
            .is_err()
        );
        for names in [
            vec!["../escape.txt"],
            vec!["CON.txt"],
            vec!["A.txt", "a.txt"],
            vec!["a", "a/b"],
            vec!["a/file", "A/other"],
        ] {
            fs::write(&path, zip_bytes(&names)).unwrap();
            assert!(inventory(&scope, &path, || false).is_err(), "{names:?}");
            assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        }
        let mut excessive = bytes.clone();
        let position = excessive.len() - 22;
        excessive[position + 8..position + 10].copy_from_slice(&10_001_u16.to_le_bytes());
        excessive[position + 10..position + 12].copy_from_slice(&10_001_u16.to_le_bytes());
        assert!(preflight(&excessive).is_err());
        let mut oversized = bytes.clone();
        let central = oversized
            .windows(4)
            .position(|bytes| bytes == b"PK\x01\x02")
            .unwrap();
        oversized[central + 24..central + 28]
            .copy_from_slice(&(16_u32 * 1_048_576 + 1).to_le_bytes());
        fs::write(&path, oversized).unwrap();
        assert!(inventory(&scope, &path, || false).is_err());
        fs::write(&path, &bytes).unwrap();
        calls.set(0);
        assert!(
            inventory(&scope, &path, || {
                let count = calls.get();
                calls.set(count + 1);
                if count == 3 {
                    fs::write(&path, b"changed").unwrap();
                }
                false
            })
            .err()
            .unwrap()
            .contains("changed")
        );
        fs::remove_file(path).unwrap();
        fs::remove_dir(root).unwrap();
    }
}
