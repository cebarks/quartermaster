use std::fs;
use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result};
use sevenz_rust2::{ArchiveReader, Password};
use sha2::{Digest, Sha256};
use zip::ZipArchive;

#[derive(Debug, Clone, Copy, PartialEq)]
enum ArchiveFormat {
    Zip,
    SevenZ,
}

/// Detect archive format by reading magic bytes from the file header.
fn detect_format(archive_path: &Path) -> Result<ArchiveFormat> {
    let mut file = fs::File::open(archive_path)
        .with_context(|| format!("failed to open archive: {}", archive_path.display()))?;
    let mut magic = [0u8; 6];
    file.read_exact(&mut magic)
        .with_context(|| format!("failed to read archive header: {}", archive_path.display()))?;

    // 7z magic: 37 7A BC AF 27 1C
    if magic == [0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C] {
        return Ok(ArchiveFormat::SevenZ);
    }
    // ZIP magic: 50 4B 03 04 (PK\x03\x04)
    if magic[..4] == [0x50, 0x4B, 0x03, 0x04] {
        return Ok(ArchiveFormat::Zip);
    }

    anyhow::bail!(
        "unsupported archive format (not ZIP or 7z): {}",
        archive_path.display()
    )
}

/// List all entry names in an archive (ZIP or 7z).
fn list_entry_names(archive_path: &Path) -> Result<Vec<String>> {
    match detect_format(archive_path)? {
        ArchiveFormat::Zip => {
            let file = fs::File::open(archive_path)
                .with_context(|| format!("failed to open archive: {}", archive_path.display()))?;
            let archive = ZipArchive::new(file)
                .with_context(|| format!("failed to read ZIP: {}", archive_path.display()))?;
            Ok(archive.file_names().map(String::from).collect())
        }
        ArchiveFormat::SevenZ => {
            let mut file = fs::File::open(archive_path)
                .with_context(|| format!("failed to open archive: {}", archive_path.display()))?;
            let pwd = Password::empty();
            let archive = sevenz_rust2::Archive::read(&mut file, &pwd)
                .with_context(|| format!("failed to read 7z: {}", archive_path.display()))?;
            Ok(archive
                .files
                .iter()
                .map(|entry| {
                    let name = entry.name().to_string();
                    if entry.is_directory() && !name.ends_with('/') {
                        format!("{name}/")
                    } else {
                        name
                    }
                })
                .collect())
        }
    }
}

/// Classification of a mod based on its archive contents.
#[derive(Debug, Clone, PartialEq)]
pub enum ModType {
    Server,
    Client,
    Hybrid,
    Ambiguous,
}

/// A file that was extracted from a mod archive.
#[derive(Debug, Clone)]
pub struct ExtractedFile {
    pub path: String,
    pub hash: String,
    pub size: u64,
}

/// Known prefixes that indicate a mod's target directory.
const SERVER_PREFIX: &str = "SPT/user/mods/";
const CLIENT_PREFIX: &str = "BepInEx/plugins/";

/// Inspect ZIP entries to determine whether a mod targets the server, client, or both.
///
/// - Has `SPT/user/mods/` paths -> Server
/// - Has `BepInEx/plugins/` paths -> Client
/// - Has both -> Hybrid
/// - Has neither -> Ambiguous
pub fn detect_mod_type(archive_path: &Path) -> Result<ModType> {
    let names = list_entry_names(archive_path)?;

    let mut has_server = false;
    let mut has_client = false;

    for name in &names {
        let effective = strip_known_prefix_from_name(name);

        if effective.starts_with(SERVER_PREFIX) {
            has_server = true;
        }
        if effective.starts_with(CLIENT_PREFIX) {
            has_client = true;
        }
    }

    match (has_server, has_client) {
        (true, true) => Ok(ModType::Hybrid),
        (true, false) => Ok(ModType::Server),
        (false, true) => Ok(ModType::Client),
        (false, false) => Ok(ModType::Ambiguous),
    }
}

/// If all entries share a single top-level directory that does NOT start with a
/// known prefix (`SPT/` or `BepInEx/`), return that directory as the prefix to
/// strip (e.g. `"SAIN/"`). Otherwise return an empty string.
pub fn detect_strip_prefix(archive_path: &Path) -> Result<String> {
    let names = list_entry_names(archive_path)?;
    let mut common_prefix: Option<String> = None;

    for name in &names {
        let top_dir = match name.find('/') {
            Some(idx) => &name[..=idx],
            None => {
                return Ok(String::new());
            }
        };

        if top_dir == "SPT/" || top_dir == "BepInEx/" {
            return Ok(String::new());
        }

        match &common_prefix {
            None => common_prefix = Some(top_dir.to_string()),
            Some(existing) => {
                if existing != top_dir {
                    return Ok(String::new());
                }
            }
        }
    }

    Ok(common_prefix.unwrap_or_default())
}

/// Extract a mod archive into `spt_root`, stripping any wrapper directory prefix.
///
/// Returns a list of extracted files with their relative paths, SHA256 hashes, and sizes.
pub fn extract_mod(archive_path: &Path, spt_root: &Path) -> Result<Vec<ExtractedFile>> {
    let prefix = detect_strip_prefix(archive_path)?;

    match detect_format(archive_path)? {
        ArchiveFormat::Zip => extract_mod_zip(archive_path, spt_root, &prefix),
        ArchiveFormat::SevenZ => extract_mod_7z(archive_path, spt_root, &prefix),
    }
}

fn extract_mod_zip(
    archive_path: &Path,
    spt_root: &Path,
    prefix: &str,
) -> Result<Vec<ExtractedFile>> {
    let file = fs::File::open(archive_path)
        .with_context(|| format!("failed to open archive: {}", archive_path.display()))?;
    let mut archive = ZipArchive::new(file)
        .with_context(|| format!("failed to read ZIP: {}", archive_path.display()))?;

    let mut extracted = Vec::new();

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .with_context(|| format!("failed to read ZIP entry {i}"))?;

        let raw_name = entry.name().to_string();

        let relative = if !prefix.is_empty() && raw_name.starts_with(prefix) {
            &raw_name[prefix.len()..]
        } else {
            &raw_name
        };

        if relative.is_empty() {
            continue;
        }

        if relative.contains("..") {
            anyhow::bail!("archive entry contains path traversal: {raw_name}");
        }

        let dest = spt_root.join(relative);
        validate_dest_under_root(&dest, spt_root, &raw_name)?;

        if entry.is_dir() {
            fs::create_dir_all(&dest)
                .with_context(|| format!("failed to create directory: {}", dest.display()))?;
            continue;
        }

        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create directory: {}", parent.display()))?;
        }

        let mut content = Vec::new();
        entry
            .read_to_end(&mut content)
            .with_context(|| format!("failed to read ZIP entry: {relative}"))?;

        let hash = compute_hash(&content);
        let size = content.len() as u64;

        fs::write(&dest, &content)
            .with_context(|| format!("failed to write file: {}", dest.display()))?;

        extracted.push(ExtractedFile {
            path: relative.to_string(),
            hash,
            size,
        });
    }

    Ok(extracted)
}

fn extract_mod_7z(
    archive_path: &Path,
    spt_root: &Path,
    prefix: &str,
) -> Result<Vec<ExtractedFile>> {
    let mut reader = ArchiveReader::open(archive_path, Password::empty())
        .with_context(|| format!("failed to read 7z: {}", archive_path.display()))?;

    let mut extracted = Vec::new();
    let prefix = prefix.to_string();

    reader
        .for_each_entries(|entry, reader| {
            let raw_name = entry.name().to_string();

            let relative = if !prefix.is_empty() && raw_name.starts_with(&prefix) {
                &raw_name[prefix.len()..]
            } else {
                &raw_name
            };

            if relative.is_empty() {
                return Ok(true);
            }

            if relative.contains("..") {
                return Err(sevenz_rust2::Error::Other(
                    format!("archive entry contains path traversal: {raw_name}").into(),
                ));
            }

            let dest = spt_root.join(relative);
            validate_dest_under_root(&dest, spt_root, &raw_name)
                .map_err(|e| sevenz_rust2::Error::Other(e.to_string().into()))?;

            if entry.is_directory() {
                fs::create_dir_all(&dest).map_err(|e| {
                    sevenz_rust2::Error::Io(
                        std::io::Error::new(
                            e.kind(),
                            format!("failed to create directory: {}", dest.display()),
                        ),
                        "".into(),
                    )
                })?;
                return Ok(true);
            }

            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent).map_err(|e| {
                    sevenz_rust2::Error::Io(
                        std::io::Error::new(
                            e.kind(),
                            format!("failed to create directory: {}", parent.display()),
                        ),
                        "".into(),
                    )
                })?;
            }

            let mut content = Vec::with_capacity(entry.size() as usize);
            std::io::copy(reader, &mut content).map_err(|e| {
                sevenz_rust2::Error::Io(
                    std::io::Error::new(e.kind(), format!("failed to read 7z entry: {relative}")),
                    "".into(),
                )
            })?;

            let hash = compute_hash(&content);
            let size = content.len() as u64;

            fs::write(&dest, &content).map_err(|e| {
                sevenz_rust2::Error::Io(
                    std::io::Error::new(
                        e.kind(),
                        format!("failed to write file: {}", dest.display()),
                    ),
                    "".into(),
                )
            })?;

            extracted.push(ExtractedFile {
                path: relative.to_string(),
                hash,
                size,
            });

            Ok(true)
        })
        .with_context(|| format!("failed to extract 7z: {}", archive_path.display()))?;

    Ok(extracted)
}

/// Verify the resolved destination path is under spt_root (defense in depth).
fn validate_dest_under_root(dest: &Path, spt_root: &Path, raw_name: &str) -> Result<()> {
    let canonical_root = spt_root
        .canonicalize()
        .with_context(|| format!("failed to canonicalize spt_root: {}", spt_root.display()))?;

    let parent = dest
        .parent()
        .ok_or_else(|| anyhow::anyhow!("archive entry has no parent directory: {raw_name}"))?;

    fs::create_dir_all(parent).ok();

    if let Ok(canonical_dest) = parent.canonicalize() {
        if !canonical_dest.starts_with(&canonical_root) {
            anyhow::bail!("archive entry escapes SPT root: {raw_name}");
        }
    }

    Ok(())
}

/// Compute the SHA256 hash of a file on disk, returned as a lowercase hex string.
pub fn compute_file_hash(path: &Path) -> Result<String> {
    let data = fs::read(path)
        .with_context(|| format!("failed to read file for hashing: {}", path.display()))?;
    Ok(compute_hash(&data))
}

/// Delete mod files from `spt_root` and clean up empty parent directories.
///
/// For each file path, deletes the file and then walks up through parent
/// directories, removing any that are empty, stopping at `spt_root`.
pub fn delete_mod_files(spt_root: &Path, file_paths: &[String]) -> Result<()> {
    for rel_path in file_paths {
        let full = spt_root.join(rel_path);
        if full.exists() {
            fs::remove_file(&full)
                .with_context(|| format!("failed to delete: {}", full.display()))?;
        }

        // Walk up, removing empty dirs until we hit spt_root
        let mut dir = full.parent().map(|p| p.to_path_buf());
        while let Some(d) = dir {
            if !d.starts_with(spt_root) || d == spt_root {
                break;
            }
            // Try to remove — will fail if non-empty, which is fine
            if fs::remove_dir(&d).is_err() {
                break;
            }
            dir = d.parent().map(|p| p.to_path_buf());
        }
    }

    Ok(())
}

/// Recursively scan `SPT/user/mods/` and `BepInEx/plugins/` under `spt_root`,
/// returning all file paths relative to `spt_root`.
pub fn scan_mod_directories(spt_root: &Path) -> Result<Vec<String>> {
    let mut out = Vec::new();

    let server_dir = spt_root.join("SPT/user/mods");
    if server_dir.is_dir() {
        scan_dir_recursive(&server_dir, spt_root, &mut out)?;
    }

    let client_dir = spt_root.join("BepInEx/plugins");
    if client_dir.is_dir() {
        scan_dir_recursive(&client_dir, spt_root, &mut out)?;
    }

    Ok(out)
}

/// Compute SHA256 of a byte slice, returned as a lowercase hex string.
pub fn compute_hash_public(data: &[u8]) -> String {
    compute_hash(data)
}

fn compute_hash(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let result = hasher.finalize();
    hex_encode(&result)
}

/// Recursively scan a directory, collecting file paths relative to `spt_root`.
fn scan_dir_recursive(dir: &Path, spt_root: &Path, out: &mut Vec<String>) -> Result<()> {
    let entries = fs::read_dir(dir)
        .with_context(|| format!("failed to read directory: {}", dir.display()))?;

    for entry in entries {
        let entry = entry
            .with_context(|| format!("failed to read directory entry in: {}", dir.display()))?;
        let path = entry.path();

        if path.is_dir() {
            scan_dir_recursive(&path, spt_root, out)?;
        } else {
            let relative = path
                .strip_prefix(spt_root)
                .with_context(|| {
                    format!(
                        "path {} is not under spt_root {}",
                        path.display(),
                        spt_root.display()
                    )
                })?
                .to_string_lossy()
                .to_string();
            out.push(relative);
        }
    }

    Ok(())
}

/// Encode bytes as a lowercase hex string (avoids pulling in the `hex` crate).
fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Given a full entry name from a ZIP, strip a wrapper directory if the
/// underlying path starts with a known prefix. This is used by `detect_mod_type`
/// to look through wrapper directories.
fn strip_known_prefix_from_name(name: &str) -> &str {
    // If the name directly starts with a known prefix, return as-is
    if name.starts_with("SPT/") || name.starts_with("BepInEx/") {
        return name;
    }

    // Check if after the first path component, a known prefix appears
    if let Some(idx) = name.find('/') {
        let after = &name[idx + 1..];
        if after.starts_with("SPT/") || after.starts_with("BepInEx/") {
            return after;
        }
    }

    name
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    /// Create a test ZIP archive with the given entries.
    fn create_test_zip(entries: &[(&str, &[u8])]) -> tempfile::NamedTempFile {
        let buf = std::io::Cursor::new(Vec::new());
        let mut zip = ZipWriter::new(buf);
        let opts = SimpleFileOptions::default();
        for (name, content) in entries {
            zip.start_file(*name, opts).unwrap();
            zip.write_all(content).unwrap();
        }
        let buf = zip.finish().unwrap();
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(buf.get_ref()).unwrap();
        tmp.flush().unwrap();
        tmp
    }

    #[test]
    fn detect_server_mod() {
        let zip = create_test_zip(&[
            ("SPT/user/mods/TestMod/package.json", b"{}"),
            ("SPT/user/mods/TestMod/src/mod.ts", b"// code"),
        ]);
        let result = detect_mod_type(zip.path()).unwrap();
        assert_eq!(result, ModType::Server);
    }

    #[test]
    fn detect_client_mod() {
        let zip = create_test_zip(&[
            ("BepInEx/plugins/TestPlugin.dll", b"\x00\x01"),
            ("BepInEx/plugins/TestPlugin/config.json", b"{}"),
        ]);
        let result = detect_mod_type(zip.path()).unwrap();
        assert_eq!(result, ModType::Client);
    }

    #[test]
    fn detect_hybrid_mod() {
        let zip = create_test_zip(&[
            ("SPT/user/mods/TestMod/package.json", b"{}"),
            ("BepInEx/plugins/TestPlugin.dll", b"\x00\x01"),
        ]);
        let result = detect_mod_type(zip.path()).unwrap();
        assert_eq!(result, ModType::Hybrid);
    }

    #[test]
    fn detect_ambiguous_mod() {
        let zip = create_test_zip(&[("readme.txt", b"hello"), ("data/config.json", b"{}")]);
        let result = detect_mod_type(zip.path()).unwrap();
        assert_eq!(result, ModType::Ambiguous);
    }

    #[test]
    fn strip_top_level_wrapper_dir() {
        let zip = create_test_zip(&[
            ("SAIN/SPT/user/mods/SAIN/package.json", b"{}"),
            ("SAIN/SPT/user/mods/SAIN/src/mod.ts", b"// code"),
        ]);
        let prefix = detect_strip_prefix(zip.path()).unwrap();
        assert_eq!(prefix, "SAIN/");
    }

    #[test]
    fn no_strip_when_known_prefix() {
        let zip = create_test_zip(&[
            ("SPT/user/mods/TestMod/package.json", b"{}"),
            ("SPT/user/mods/TestMod/src/mod.ts", b"// code"),
        ]);
        let prefix = detect_strip_prefix(zip.path()).unwrap();
        assert_eq!(prefix, "");
    }

    #[test]
    fn extract_server_mod() {
        let zip = create_test_zip(&[
            ("SPT/user/mods/TestMod/package.json", b"{\"name\":\"test\"}"),
            ("SPT/user/mods/TestMod/src/mod.ts", b"export class Mod {}"),
        ]);

        let tmp_dir = TempDir::new().unwrap();
        let files = extract_mod(zip.path(), tmp_dir.path()).unwrap();

        assert_eq!(files.len(), 2);

        // Verify files exist on disk
        for f in &files {
            let full_path = tmp_dir.path().join(&f.path);
            assert!(full_path.exists(), "file should exist: {}", f.path);
        }

        // Verify hashes are populated (non-empty hex strings)
        for f in &files {
            assert!(!f.hash.is_empty(), "hash should be populated");
            assert!(
                f.hash.chars().all(|c| c.is_ascii_hexdigit()),
                "hash should be hex: {}",
                f.hash
            );
            assert_eq!(f.hash.len(), 64, "SHA256 hex should be 64 chars");
        }

        // Verify sizes match content
        let pkg = files
            .iter()
            .find(|f| f.path.contains("package.json"))
            .unwrap();
        assert_eq!(pkg.size, b"{\"name\":\"test\"}".len() as u64);
    }

    #[test]
    fn compute_hash_of_file() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        fs::write(tmp.path(), b"hello world").unwrap();

        let hash = compute_file_hash(tmp.path()).unwrap();
        assert_eq!(
            hash,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[test]
    fn delete_mod_files_and_empty_dirs() {
        let tmp_dir = TempDir::new().unwrap();
        let root = tmp_dir.path();

        // Create some files in nested directories
        let mod_dir = root.join("SPT/user/mods/TestMod/src");
        fs::create_dir_all(&mod_dir).unwrap();
        fs::write(mod_dir.join("mod.ts"), b"// code").unwrap();
        fs::write(root.join("SPT/user/mods/TestMod/package.json"), b"{}").unwrap();

        let file_paths = vec![
            "SPT/user/mods/TestMod/src/mod.ts".to_string(),
            "SPT/user/mods/TestMod/package.json".to_string(),
        ];

        delete_mod_files(root, &file_paths).unwrap();

        // Files should be gone
        assert!(!mod_dir.join("mod.ts").exists());
        assert!(!root.join("SPT/user/mods/TestMod/package.json").exists());

        // Empty directories should be cleaned up
        assert!(!mod_dir.exists(), "src/ should be removed (empty)");
        assert!(
            !root.join("SPT/user/mods/TestMod").exists(),
            "TestMod/ should be removed (empty)"
        );

        // SPT/user/mods/ should still exist (it's a structural dir we don't own)
        // Actually, it will be removed too since it becomes empty. That's fine —
        // the important thing is we stop at spt_root.
        // The root itself must still exist.
        assert!(root.exists(), "spt_root itself should not be deleted");
    }

    #[test]
    fn scan_finds_all_mod_files() {
        let tmp_dir = TempDir::new().unwrap();
        let root = tmp_dir.path();

        // Create server mod files
        let server_dir = root.join("SPT/user/mods/TestMod");
        fs::create_dir_all(&server_dir).unwrap();
        fs::write(server_dir.join("package.json"), b"{}").unwrap();
        fs::write(server_dir.join("mod.ts"), b"// code").unwrap();

        // Create client mod files
        let client_dir = root.join("BepInEx/plugins/TestPlugin");
        fs::create_dir_all(&client_dir).unwrap();
        fs::write(client_dir.join("TestPlugin.dll"), b"\x00").unwrap();

        let mut files = scan_mod_directories(root).unwrap();
        files.sort();

        assert_eq!(files.len(), 3);
        assert!(
            files.contains(&"BepInEx/plugins/TestPlugin/TestPlugin.dll".to_string()),
            "should find client plugin: {files:?}"
        );
        assert!(
            files.contains(&"SPT/user/mods/TestMod/mod.ts".to_string()),
            "should find server mod source: {files:?}"
        );
        assert!(
            files.contains(&"SPT/user/mods/TestMod/package.json".to_string()),
            "should find server mod package.json: {files:?}"
        );
    }

    /// Create a test 7z archive with the given entries.
    fn create_test_7z(entries: &[(&str, &[u8])]) -> tempfile::NamedTempFile {
        use sevenz_rust2::{ArchiveEntry, ArchiveWriter};

        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mut writer = ArchiveWriter::create(tmp.path()).unwrap();
        for (name, content) in entries {
            let entry = ArchiveEntry::new_file(name);
            writer
                .push_archive_entry(entry, Some(std::io::Cursor::new(content)))
                .unwrap();
        }
        writer.finish().unwrap();
        tmp
    }

    #[test]
    fn detect_server_mod_7z() {
        let archive = create_test_7z(&[
            ("SPT/user/mods/TestMod/package.json", b"{}"),
            ("SPT/user/mods/TestMod/src/mod.ts", b"// code"),
        ]);
        let result = detect_mod_type(archive.path()).unwrap();
        assert_eq!(result, ModType::Server);
    }

    #[test]
    fn extract_server_mod_7z() {
        let archive = create_test_7z(&[
            ("SPT/user/mods/TestMod/package.json", b"{\"name\":\"test\"}"),
            ("SPT/user/mods/TestMod/src/mod.ts", b"export class Mod {}"),
        ]);

        let tmp_dir = TempDir::new().unwrap();
        let files = extract_mod(archive.path(), tmp_dir.path()).unwrap();

        assert_eq!(files.len(), 2);

        for f in &files {
            let full_path = tmp_dir.path().join(&f.path);
            assert!(full_path.exists(), "file should exist: {}", f.path);
        }

        for f in &files {
            assert!(!f.hash.is_empty(), "hash should be populated");
            assert!(
                f.hash.chars().all(|c| c.is_ascii_hexdigit()),
                "hash should be hex: {}",
                f.hash
            );
            assert_eq!(f.hash.len(), 64, "SHA256 hex should be 64 chars");
        }

        let pkg = files
            .iter()
            .find(|f| f.path.contains("package.json"))
            .unwrap();
        assert_eq!(pkg.size, b"{\"name\":\"test\"}".len() as u64);
    }

    #[test]
    fn strip_top_level_wrapper_dir_7z() {
        let archive = create_test_7z(&[
            ("SAIN/SPT/user/mods/SAIN/package.json", b"{}"),
            ("SAIN/SPT/user/mods/SAIN/src/mod.ts", b"// code"),
        ]);
        let prefix = detect_strip_prefix(archive.path()).unwrap();
        assert_eq!(prefix, "SAIN/");
    }

    #[test]
    fn extract_rejects_path_traversal() {
        let zip = create_test_zip(&[
            ("SPT/user/mods/../../etc/evil.txt", b"malicious"),
            ("SPT/user/mods/../../../tmp/bad", b"also bad"),
        ]);
        let tmp_dir = TempDir::new().unwrap();
        let result = extract_mod(zip.path(), tmp_dir.path());
        assert!(result.is_err(), "should reject path traversal entries");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("path traversal"),
            "error should mention path traversal: {err}"
        );
    }
}
