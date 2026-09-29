//! Password-encrypted library backups (roadmap `0305`).
//!
//! A backup is a portable export encrypted with a key derived from the user's password:
//!
//! ```text
//! magic "LOOMBAK1" | header length (u32 LE) | header JSON | chunk*
//! chunk = ciphertext length (u32 LE) | XChaCha20-Poly1305 ciphertext and tag
//! ```
//!
//! The key is Argon2id(password, random 16-byte salt). Chunks use the STREAM construction: the
//! 24-byte nonce is a random 19-byte prefix, a 32-bit big-endian chunk counter, and a final-chunk
//! flag, and every chunk authenticates the magic, header length, and header bytes as associated
//! data. Reordering, truncation, extension, header edits, and wrong passwords all fail
//! authentication before any plaintext is used. Passwords and keys are never written or logged.
//! Zeroization is best effort: the derived key, the cipher state, and the decrypted buffer are
//! wiped when dropped, but copies made while serializing or parsing the export are not.

use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    XChaCha20Poly1305, XNonce,
};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::{
    error::{io_error, LoomError, Result},
    portable::{PortableExport, PortableImportReport},
    store::Library,
};

pub const BACKUP_MAGIC: &[u8; 8] = b"LOOMBAK1";
pub const BACKUP_FORMAT: &str = "loom.encrypted-backup";
pub const BACKUP_FORMAT_VERSION: u32 = 1;
const CHUNK_BYTES: usize = 64 * 1024;
const TAG_BYTES: usize = 16;
const SALT_BYTES: usize = 16;
const NONCE_PREFIX_BYTES: usize = 19;
const MAX_HEADER_BYTES: usize = 4096;
/// Upper bounds accepted when reading a header, so a crafted backup cannot demand unbounded work.
/// Header bounds are checked before any key derivation, so a crafted backup cannot demand more
/// than 256 MiB, 6 passes, or 4 lanes.
const MAX_KDF_MEMORY_KIB: u32 = 256 * 1024;
const MAX_KDF_ITERATIONS: u32 = 6;
const MAX_KDF_PARALLELISM: u32 = 4;
/// Minimum cost accepted when writing: the OWASP Argon2id floor (19 MiB, 2 passes).
pub const MIN_KDF_MEMORY_KIB: u32 = 19 * 1024;
pub const MIN_KDF_ITERATIONS: u32 = 2;
/// Largest backup or export file read into memory.
pub const MAX_BACKUP_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MIN_PASSWORD_BYTES: usize = 12;

/// Key-derivation cost. The default follows the OWASP Argon2id recommendation with extra memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackupOptions {
    pub kdf_memory_kib: u32,
    pub kdf_iterations: u32,
    pub kdf_parallelism: u32,
}

impl Default for BackupOptions {
    fn default() -> Self {
        Self {
            kdf_memory_kib: 64 * 1024,
            kdf_iterations: 3,
            kdf_parallelism: 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct KdfHeader {
    algorithm: String,
    version: u32,
    memory_kib: u32,
    iterations: u32,
    parallelism: u32,
    salt: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AeadHeader {
    algorithm: String,
    nonce_prefix: String,
    chunk_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BackupHeader {
    format: String,
    version: u32,
    kdf: KdfHeader,
    aead: AeadHeader,
    payload: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupReport {
    pub path: PathBuf,
    pub bytes: u64,
    pub chunks: u64,
    pub rows: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestoreReport {
    pub database: PathBuf,
    pub import: PortableImportReport,
}

fn backup_error(message: impl Into<String>) -> LoomError {
    LoomError::Backup(message.into())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn unhex(text: &str, length: usize) -> Result<Vec<u8>> {
    let bytes = text.as_bytes();
    if bytes.len() != length * 2 {
        return Err(backup_error("malformed header"));
    }
    let nibble = |byte: u8| match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => Err(backup_error("malformed header")),
    };
    let (pairs, _) = bytes.as_chunks::<2>();
    pairs
        .iter()
        .map(|[high, low]| Ok(nibble(*high)? << 4 | nibble(*low)?))
        .collect()
}

/// Reads a regular file of at most `MAX_BACKUP_BYTES`, refusing FIFOs, devices, and directories.
pub fn read_bounded_file(path: &Path) -> Result<Vec<u8>> {
    let metadata = fs::metadata(path).map_err(|source| io_error(path, source))?;
    if !metadata.is_file() {
        return Err(backup_error(format!(
            "not a regular file: {}",
            path.display()
        )));
    }
    if metadata.len() > MAX_BACKUP_BYTES {
        return Err(backup_error(format!(
            "file exceeds the {MAX_BACKUP_BYTES}-byte limit: {}",
            path.display()
        )));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    File::open(path)
        .and_then(|file| file.take(MAX_BACKUP_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|source| io_error(path, source))?;
    if bytes.len() as u64 > MAX_BACKUP_BYTES {
        return Err(backup_error("file grew past the size limit while reading"));
    }
    Ok(bytes)
}

fn random_bytes<const N: usize>() -> Result<[u8; N]> {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).map_err(|error| backup_error(format!("no randomness: {error}")))?;
    Ok(bytes)
}

fn derive_key(password: &[u8], kdf: &KdfHeader) -> Result<Zeroizing<[u8; 32]>> {
    if kdf.algorithm != "argon2id"
        || kdf.version != 0x13
        || kdf.memory_kib > MAX_KDF_MEMORY_KIB
        || kdf.iterations == 0
        || kdf.iterations > MAX_KDF_ITERATIONS
        || kdf.parallelism == 0
        || kdf.parallelism > MAX_KDF_PARALLELISM
    {
        return Err(backup_error("unsupported key derivation parameters"));
    }
    let salt = unhex(&kdf.salt, SALT_BYTES)?;
    let params = Params::new(kdf.memory_kib, kdf.iterations, kdf.parallelism, Some(32))
        .map_err(|_| backup_error("unsupported key derivation parameters"))?;
    let mut key = Zeroizing::new([0u8; 32]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(password, &salt, key.as_mut())
        .map_err(|_| backup_error("key derivation failed"))?;
    Ok(key)
}

fn chunk_nonce(prefix: &[u8], counter: u32, last: bool) -> XNonce {
    let mut nonce = [0u8; 24];
    nonce[..NONCE_PREFIX_BYTES].copy_from_slice(prefix);
    nonce[NONCE_PREFIX_BYTES..NONCE_PREFIX_BYTES + 4].copy_from_slice(&counter.to_be_bytes());
    nonce[23] = u8::from(last);
    XNonce::from(nonce)
}

/// Encrypts `plaintext` into the backup byte format.
pub(crate) fn encrypt_backup(
    plaintext: &[u8],
    password: &[u8],
    options: BackupOptions,
) -> Result<(Vec<u8>, u64)> {
    if password.len() < MIN_PASSWORD_BYTES {
        return Err(backup_error(format!(
            "backup passwords must be at least {MIN_PASSWORD_BYTES} bytes"
        )));
    }
    let salt = random_bytes::<SALT_BYTES>()?;
    let prefix = random_bytes::<NONCE_PREFIX_BYTES>()?;
    let header = BackupHeader {
        format: BACKUP_FORMAT.into(),
        version: BACKUP_FORMAT_VERSION,
        kdf: KdfHeader {
            algorithm: "argon2id".into(),
            version: 0x13,
            memory_kib: options.kdf_memory_kib,
            iterations: options.kdf_iterations,
            parallelism: options.kdf_parallelism,
            salt: hex(&salt),
        },
        aead: AeadHeader {
            algorithm: "xchacha20poly1305-stream".into(),
            nonce_prefix: hex(&prefix),
            chunk_bytes: CHUNK_BYTES,
        },
        payload: "loom.portable-export+json".into(),
    };
    let header_bytes = serde_json::to_vec(&header)?;
    let key = derive_key(password, &header.kdf)?;
    let cipher = XChaCha20Poly1305::new_from_slice(key.as_ref())
        .map_err(|_| backup_error("invalid key length"))?;

    let mut output = Vec::with_capacity(plaintext.len() + plaintext.len() / 32 + 256);
    output.extend_from_slice(BACKUP_MAGIC);
    output.extend_from_slice(&(header_bytes.len() as u32).to_le_bytes());
    output.extend_from_slice(&header_bytes);
    let aad = output.clone();

    let chunks: Vec<&[u8]> = if plaintext.is_empty() {
        vec![&[]]
    } else {
        plaintext.chunks(CHUNK_BYTES).collect()
    };
    let count = u32::try_from(chunks.len()).map_err(|_| backup_error("backup is too large"))?;
    for (index, chunk) in chunks.iter().enumerate() {
        let counter = index as u32;
        let nonce = chunk_nonce(&prefix, counter, counter + 1 == count);
        let ciphertext = cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: chunk,
                    aad: &aad,
                },
            )
            .map_err(|_| backup_error("encryption failed"))?;
        output.extend_from_slice(&(ciphertext.len() as u32).to_le_bytes());
        output.extend_from_slice(&ciphertext);
    }
    Ok((output, u64::from(count)))
}

fn take<'a>(bytes: &'a [u8], cursor: &mut usize, length: usize) -> Result<&'a [u8]> {
    let end = cursor
        .checked_add(length)
        .filter(|end| *end <= bytes.len())
        .ok_or_else(|| backup_error("backup is truncated"))?;
    let slice = &bytes[*cursor..end];
    *cursor = end;
    Ok(slice)
}

fn read_u32(bytes: &[u8], cursor: &mut usize) -> Result<usize> {
    let raw = take(bytes, cursor, 4)?;
    Ok(u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]) as usize)
}

/// Authenticates and decrypts backup bytes. No plaintext is returned unless every chunk verifies
/// and the final chunk is present with nothing after it.
pub(crate) fn decrypt_backup(bytes: &[u8], password: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
    let mut cursor = 0usize;
    if take(bytes, &mut cursor, BACKUP_MAGIC.len())? != BACKUP_MAGIC {
        return Err(backup_error("not a LOOM backup"));
    }
    let header_length = read_u32(bytes, &mut cursor)?;
    if header_length == 0 || header_length > MAX_HEADER_BYTES {
        return Err(backup_error("malformed header"));
    }
    let header_bytes = take(bytes, &mut cursor, header_length)?;
    let aad = &bytes[..cursor];
    let header: BackupHeader =
        serde_json::from_slice(header_bytes).map_err(|_| backup_error("malformed header"))?;
    if header.format != BACKUP_FORMAT
        || header.version != BACKUP_FORMAT_VERSION
        || header.aead.algorithm != "xchacha20poly1305-stream"
        || header.aead.chunk_bytes != CHUNK_BYTES
        || header.payload != "loom.portable-export+json"
    {
        return Err(backup_error("unsupported backup format"));
    }
    let prefix = unhex(&header.aead.nonce_prefix, NONCE_PREFIX_BYTES)?;
    let key = derive_key(password, &header.kdf)?;
    let cipher = XChaCha20Poly1305::new_from_slice(key.as_ref())
        .map_err(|_| backup_error("invalid key length"))?;

    // Reserve the upper bound up front so the plaintext buffer never reallocates and leaves
    // unwiped copies behind.
    let mut plaintext = Zeroizing::new(Vec::with_capacity(bytes.len().saturating_sub(cursor)));
    let mut counter: u32 = 0;
    loop {
        if cursor == bytes.len() {
            return Err(backup_error("backup is truncated"));
        }
        let length = read_u32(bytes, &mut cursor)?;
        if !(TAG_BYTES..=CHUNK_BYTES + TAG_BYTES).contains(&length) {
            return Err(backup_error("malformed chunk"));
        }
        let ciphertext = take(bytes, &mut cursor, length)?;
        let last = cursor == bytes.len();
        let chunk = cipher
            .decrypt(
                &chunk_nonce(&prefix, counter, last),
                Payload {
                    msg: ciphertext,
                    aad,
                },
            )
            .map_err(|_| backup_error("authentication failed"))?;
        let chunk = Zeroizing::new(chunk);
        if !last && chunk.len() != CHUNK_BYTES {
            return Err(backup_error("malformed chunk"));
        }
        plaintext.extend_from_slice(&chunk);
        if last {
            return Ok(plaintext);
        }
        counter = counter
            .checked_add(1)
            .ok_or_else(|| backup_error("backup is too large"))?;
    }
}

fn write_new_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let file_name = path
        .file_name()
        .ok_or_else(|| backup_error("backup path has no file name"))?
        .to_string_lossy();
    let temporary = path.with_file_name(format!(".{file_name}.{}.tmp", uuid::Uuid::new_v4()));
    let written = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .and_then(|mut file| {
            file.write_all(bytes)?;
            file.sync_all()
        });
    if let Err(source) = written {
        let _ = fs::remove_file(&temporary);
        return Err(io_error(&temporary, source));
    }
    // A hard link commits without replacing an existing destination on every platform.
    let committed = fs::hard_link(&temporary, path);
    let _ = fs::remove_file(&temporary);
    committed.map_err(|source| io_error(path, source))
}

impl Library {
    /// Writes a password-encrypted backup of every canonical row and setting to a new file.
    /// An existing file at `destination` is never overwritten.
    pub fn write_encrypted_backup(
        &self,
        destination: impl AsRef<Path>,
        password: &[u8],
        options: BackupOptions,
    ) -> Result<BackupReport> {
        let destination = destination.as_ref();
        if fs::symlink_metadata(destination).is_ok() {
            return Err(backup_error(format!(
                "refusing to overwrite {}",
                destination.display()
            )));
        }
        if options.kdf_memory_kib < MIN_KDF_MEMORY_KIB
            || options.kdf_iterations < MIN_KDF_ITERATIONS
        {
            return Err(backup_error(format!(
                "key derivation cost is below the {MIN_KDF_MEMORY_KIB} KiB / \
                 {MIN_KDF_ITERATIONS}-pass minimum"
            )));
        }
        let export = self.export_portable()?;
        let rows = export
            .tables
            .values()
            .map(|table| table.rows.len() as u64)
            .sum();
        let plaintext = Zeroizing::new(serde_json::to_vec(&export)?);
        drop(export);
        let (bytes, chunks) = encrypt_backup(&plaintext, password, options)?;
        write_new_file(destination, &bytes)?;
        Ok(BackupReport {
            path: destination.to_path_buf(),
            bytes: bytes.len() as u64,
            chunks,
            rows,
        })
    }

    /// Restores an encrypted backup into a new library at `database`.
    ///
    /// The backup is fully authenticated and imported into a temporary database next to the
    /// destination; only a verified import is moved into place. A wrong password, tampered or
    /// truncated backup, or failed import leaves no file at `database`.
    pub fn restore_encrypted_backup(
        backup: impl AsRef<Path>,
        password: &[u8],
        database: impl AsRef<Path>,
    ) -> Result<RestoreReport> {
        let backup = backup.as_ref();
        let database = database.as_ref();
        if fs::symlink_metadata(database).is_ok() {
            return Err(backup_error(format!(
                "refusing to restore over {}",
                database.display()
            )));
        }
        let bytes = read_bounded_file(backup)?;
        let plaintext = decrypt_backup(&bytes, password)?;
        let export: PortableExport = serde_json::from_slice(&plaintext)
            .map_err(|error| LoomError::PortableExport(error.to_string()))?;
        drop(plaintext);

        let parent = database
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::create_dir_all(parent).map_err(|source| io_error(parent, source))?;
        sweep_stale_staging(parent);
        // The staging database holds decrypted rows, so it lives in a private (0700) directory
        // that is removed on every path.
        let staging_dir = parent.join(format!(".loom-restore-{}", uuid::Uuid::new_v4()));
        create_private_dir(&staging_dir)?;
        let staging = staging_dir.join("library.sqlite3");
        let imported = Library::open(&staging).and_then(|library| {
            let report = library.import_portable(&export)?;
            // Fold the WAL into the main file before handing it over; otherwise committed rows
            // could remain only in a sidecar that is about to be deleted.
            library.checkpoint_for_handoff()?;
            drop(library);
            Ok(report)
        });
        let import = match imported {
            Ok(report) => report,
            Err(error) => {
                let _ = fs::remove_dir_all(&staging_dir);
                return Err(error);
            }
        };
        let committed = File::open(&staging)
            .and_then(|file| file.sync_all())
            .and_then(|()| fs::hard_link(&staging, database))
            .and_then(|()| sync_directory(parent));
        let _ = fs::remove_dir_all(&staging_dir);
        committed.map_err(|source| io_error(database, source))?;
        Ok(RestoreReport {
            database: database.to_path_buf(),
            import,
        })
    }
}

fn create_private_dir(path: &Path) -> Result<()> {
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(path)
        .map_err(|source| io_error(path, source))
}

fn sync_directory(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        File::open(path)?.sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

/// Removes staging directories left by a restore that crashed more than an hour ago.
fn sweep_stale_staging(parent: &Path) {
    let Ok(entries) = fs::read_dir(parent) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let stale = name.to_string_lossy().starts_with(".loom-restore-")
            && entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .ok()
                .and_then(|modified| std::time::SystemTime::now().duration_since(modified).ok())
                .is_some_and(|age| age > std::time::Duration::from_secs(3600));
        if stale && entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAST: BackupOptions = BackupOptions {
        kdf_memory_kib: 64,
        kdf_iterations: 1,
        kdf_parallelism: 1,
    };
    const PASSWORD: &[u8] = b"correct horse battery";

    #[test]
    fn round_trips_empty_small_and_multi_chunk_payloads() {
        for size in [0usize, 1, CHUNK_BYTES - 1, CHUNK_BYTES, CHUNK_BYTES * 2 + 7] {
            let plaintext = (0..size)
                .map(|index| (index % 251) as u8)
                .collect::<Vec<_>>();
            let (bytes, chunks) = encrypt_backup(&plaintext, PASSWORD, FAST).unwrap();
            assert_eq!(chunks, size.div_ceil(CHUNK_BYTES).max(1) as u64);
            assert_eq!(
                decrypt_backup(&bytes, PASSWORD).unwrap().as_slice(),
                plaintext
            );
        }
    }

    #[test]
    fn wrong_password_and_short_password_fail() {
        let (bytes, _) = encrypt_backup(b"secret rows", PASSWORD, FAST).unwrap();
        assert!(decrypt_backup(&bytes, b"wrong horse battery").is_err());
        assert!(encrypt_backup(b"secret rows", b"short", FAST).is_err());
    }

    #[test]
    fn every_single_byte_flip_is_rejected() {
        let (bytes, _) = encrypt_backup(b"a small secret payload", PASSWORD, FAST).unwrap();
        for index in 0..bytes.len() {
            let mut tampered = bytes.clone();
            tampered[index] ^= 0x01;
            assert!(
                decrypt_backup(&tampered, PASSWORD).is_err(),
                "flip at byte {index} was accepted"
            );
        }
    }

    #[test]
    fn truncation_extension_and_chunk_reordering_are_rejected() {
        let plaintext = vec![7u8; CHUNK_BYTES * 3];
        let (bytes, chunks) = encrypt_backup(&plaintext, PASSWORD, FAST).unwrap();
        assert_eq!(chunks, 3);
        let header_end = 8 + 4 + u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
        let frame = 4 + CHUNK_BYTES + TAG_BYTES;

        // Dropping the final chunk makes the previous chunk look final; its nonce flag differs.
        assert!(decrypt_backup(&bytes[..header_end + frame * 2], PASSWORD).is_err());
        assert!(decrypt_backup(&bytes[..bytes.len() - 1], PASSWORD).is_err());
        assert!(decrypt_backup(&bytes[..header_end], PASSWORD).is_err());
        let mut extended = bytes.clone();
        extended.extend_from_slice(&[0, 0, 0, 0]);
        assert!(decrypt_backup(&extended, PASSWORD).is_err());

        let mut swapped = bytes[..header_end].to_vec();
        swapped.extend_from_slice(&bytes[header_end + frame..header_end + frame * 2]);
        swapped.extend_from_slice(&bytes[header_end..header_end + frame]);
        swapped.extend_from_slice(&bytes[header_end + frame * 2..]);
        assert!(decrypt_backup(&swapped, PASSWORD).is_err());
    }

    #[test]
    fn hostile_headers_are_rejected_before_key_derivation() {
        let (bytes, _) = encrypt_backup(b"payload", PASSWORD, FAST).unwrap();
        let header_length = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
        let header: serde_json::Value =
            serde_json::from_slice(&bytes[12..12 + header_length]).unwrap();
        for (path, value) in [
            ("/kdf/memory_kib", serde_json::json!(u32::MAX)),
            ("/kdf/memory_kib", serde_json::json!(256 * 1024 + 1)),
            ("/kdf/iterations", serde_json::json!(1000)),
            ("/kdf/iterations", serde_json::json!(7)),
            ("/kdf/parallelism", serde_json::json!(5)),
            ("/kdf/algorithm", serde_json::json!("pbkdf2")),
            ("/aead/chunk_bytes", serde_json::json!(1)),
            ("/version", serde_json::json!(2)),
        ] {
            let mut edited = header.clone();
            *edited.pointer_mut(path).unwrap() = value;
            let edited_bytes = serde_json::to_vec(&edited).unwrap();
            let mut hostile = BACKUP_MAGIC.to_vec();
            hostile.extend_from_slice(&(edited_bytes.len() as u32).to_le_bytes());
            hostile.extend_from_slice(&edited_bytes);
            hostile.extend_from_slice(&bytes[12 + header_length..]);
            assert!(decrypt_backup(&hostile, PASSWORD).is_err(), "{path}");
        }
        assert!(decrypt_backup(b"LOOMBAK1\xff\xff\xff\xff", PASSWORD).is_err());
        assert!(decrypt_backup(b"NOTLOOM!", PASSWORD).is_err());
    }

    #[test]
    fn non_ascii_or_signed_hex_is_rejected_without_panicking() {
        let multibyte = format!("a{}b", "é".repeat(15));
        assert_eq!(multibyte.len(), 32);
        assert!(unhex(&multibyte, 16).is_err());
        assert!(unhex(&format!("+f{}", "0".repeat(30)), 16).is_err());
        assert!(
            unhex(&"AB".repeat(16), 16).is_err(),
            "uppercase is not what we write"
        );
        assert_eq!(unhex("00ff", 2).unwrap(), vec![0, 255]);

        let (bytes, _) = encrypt_backup(b"payload", PASSWORD, FAST).unwrap();
        let header_length = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
        let mut header: serde_json::Value =
            serde_json::from_slice(&bytes[12..12 + header_length]).unwrap();
        header["kdf"]["salt"] = serde_json::Value::String(multibyte);
        let edited = serde_json::to_vec(&header).unwrap();
        let mut hostile = BACKUP_MAGIC.to_vec();
        hostile.extend_from_slice(&(edited.len() as u32).to_le_bytes());
        hostile.extend_from_slice(&edited);
        hostile.extend_from_slice(&bytes[12 + header_length..]);
        assert!(decrypt_backup(&hostile, PASSWORD).is_err());
    }

    #[test]
    fn default_options_match_the_documented_cost() {
        assert_eq!(
            BackupOptions::default(),
            BackupOptions {
                kdf_memory_kib: 65_536,
                kdf_iterations: 3,
                kdf_parallelism: 1
            }
        );
    }
}
