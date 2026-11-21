//! Attachments of a request's messages, as CortexCode stores them (copied from CortexCode's chat
//! flow, `plugins/chatflow/backend/src/attachments.rs`): bytes are stored content-addressed in
//! `$CORTEX_HOME/attachments/<sha256>.<ext>`; a folder is referenced by its path. Nothing is ever
//! written into a repository.

use base64::Engine;
use cc_protocol::contracts::chatflow::{AttachmentPutParams, AttachmentPutResult, attachment_kinds, limits};
use cc_protocol::RpcError;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};

/// The folder under `CORTEX_HOME` where attachment bytes are stored.
pub const AREA: &str = "attachments";
const HEADER_BYTES: u64 = 1024 * 1024;

fn invalid(message: impl Into<String>) -> RpcError {
    RpcError::invalid_params(message.into())
}

/// Human-readable size (mechanical: B, KB, MB, GB with one decimal).
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 { format!("{bytes} B") } else { format!("{value:.1} {}", UNITS[unit]) }
}

/// The MIME type of a file name by its extension (a short table; unknown -> octet-stream).
pub fn mime_of(name: &str) -> &'static str {
    let extension = Path::new(name).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    match extension.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "tif" | "tiff" => "image/tiff",
        "heic" => "image/heic",
        "pdf" => "application/pdf",
        "json" => "application/json",
        "zip" => "application/zip",
        "txt" | "log" => "text/plain",
        "md" => "text/markdown",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        _ => "application/octet-stream",
    }
}

/// The extension a stored copy gets: the name's own when it is a plain one, else the MIME type's.
fn stored_extension(name: &str, mime: &str) -> String {
    let own = Path::new(name).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if !own.is_empty() && own.len() <= 10 && own.chars().all(|c| c.is_ascii_alphanumeric()) {
        return own;
    }
    match mime {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/bmp" => "bmp",
        "image/svg+xml" => "svg",
        "image/tiff" => "tiff",
        "application/pdf" => "pdf",
        "text/plain" => "txt",
        _ => "bin",
    }
    .to_owned()
}

fn kind_of(mime: &str) -> &'static str {
    if mime.starts_with("image/") { attachment_kinds::IMAGE } else { attachment_kinds::FILE }
}

/// Width and height of a PNG, GIF, JPEG, WebP or BMP from its first bytes (structural parse).
pub fn image_size(head: &[u8]) -> Option<(u32, u32)> {
    let be16 = |at: usize| head.get(at..at + 2).map(|b| u16::from_be_bytes([b[0], b[1]]) as u32);
    let le16 = |at: usize| head.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]]) as u32);
    let be32 = |at: usize| head.get(at..at + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]));
    let le24 = |at: usize| head.get(at..at + 3).map(|b| u32::from_le_bytes([b[0], b[1], b[2], 0]));
    let le32 = |at: usize| head.get(at..at + 4).map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    if head.starts_with(b"\x89PNG\r\n\x1a\n") && head.get(12..16) == Some(b"IHDR") {
        return Some((be32(16)?, be32(20)?));
    }
    if head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a") {
        return Some((le16(6)?, le16(8)?));
    }
    if head.starts_with(b"BM") {
        return Some((le32(18)?.unsigned_abs(), le32(22)?.unsigned_abs()));
    }
    if head.starts_with(b"RIFF") && head.get(8..12) == Some(b"WEBP") {
        return match head.get(12..16)? {
            b"VP8X" => Some((le24(24)? + 1, le24(27)? + 1)),
            b"VP8L" => {
                let bits = u32::from_le_bytes([*head.get(21)?, *head.get(22)?, *head.get(23)?, *head.get(24)?]);
                Some(((bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1))
            }
            b"VP8 " => Some((le16(26)? & 0x3fff, le16(28)? & 0x3fff)),
            _ => None,
        };
    }
    if head.starts_with(&[0xff, 0xd8]) {
        let mut at = 2;
        while at + 4 <= head.len() {
            if head[at] != 0xff {
                at += 1;
                continue;
            }
            let marker = head[at + 1];
            if marker == 0xff {
                at += 1;
                continue;
            }
            // Markers without a length.
            if marker == 0x01 || (0xd0..=0xd9).contains(&marker) {
                at += 2;
                continue;
            }
            let length = be16(at + 2)? as usize;
            let frame = (0xc0..=0xcf).contains(&marker) && !matches!(marker, 0xc4 | 0xc8 | 0xcc);
            if frame {
                return Some((be16(at + 7)?, be16(at + 5)?));
            }
            at += 2 + length;
        }
    }
    None
}

fn read_head(path: &Path) -> Option<Vec<u8>> {
    let file = std::fs::File::open(path).ok()?;
    let mut head = Vec::new();
    file.take(HEADER_BYTES).read_to_end(&mut head).ok()?;
    Some(head)
}

/// Whether `path` is inside one of `roots` (both canonicalized by the caller).
fn inside(path: &Path, roots: &[PathBuf]) -> bool {
    roots.iter().any(|root| path.starts_with(root))
}

/// The SHA-256 of `bytes` as lowercase hex.
fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Store `bytes` content-addressed in the attachment area; returns the path and the hash.
pub fn store(home: &Path, bytes: &[u8], extension: &str) -> Result<(PathBuf, String), RpcError> {
    let sha = sha256_hex(bytes);
    let dir = home.join(AREA);
    std::fs::create_dir_all(&dir).map_err(|error| RpcError::internal(format!("cannot create {}: {error}", dir.display())))?;
    let path = dir.join(format!("{sha}.{extension}"));
    if !path.is_file() {
        // Written beside it, then renamed: a reader never sees half a file.
        let partial = dir.join(format!(".{sha}.{}.partial", uuid::Uuid::new_v4().simple()));
        std::fs::write(&partial, bytes).map_err(|error| RpcError::internal(format!("cannot write {}: {error}", partial.display())))?;
        std::fs::rename(&partial, &path).map_err(|error| {
            let _ = std::fs::remove_file(&partial);
            RpcError::internal(format!("cannot store {}: {error}", path.display()))
        })?;
    }
    Ok((path, sha))
}

fn too_big(name: &str, bytes: u64, what: &str) -> RpcError {
    invalid(format!(
        "{name} is {}; {what} may be at most {}.",
        human_size(bytes),
        human_size(limits::MAX_ATTACHMENT_BYTES)
    ))
}

fn check_thumb(thumb: Option<String>) -> Result<Option<String>, RpcError> {
    match thumb.filter(|thumb| !thumb.is_empty()) {
        None => Ok(None),
        Some(thumb) if !thumb.starts_with("data:image/") => Err(invalid("a preview must be a data:image/ URL")),
        Some(thumb) if thumb.len() > limits::MAX_THUMB_BYTES => {
            Err(invalid(format!("the preview is {}; at most {}", human_size(thumb.len() as u64), human_size(limits::MAX_THUMB_BYTES as u64))))
        }
        Some(thumb) => Ok(Some(thumb)),
    }
}

/// Where the shell writes bytes that never were a file (a pasted screenshot) before they are put:
/// `$CORTEX_HOME/attachments/incoming/`. A put moves them into the store.
pub const INCOMING: &str = "incoming";

/// What a put stored: hash it, keep it once by content (`<sha256>.<ext>`).
fn stored_result(home: &Path, bytes: &[u8], name: String, mime: String, thumb: Option<String>) -> Result<AttachmentPutResult, RpcError> {
    let (path, sha) = store(home, bytes, &stored_extension(&name, &mime))?;
    let size = (kind_of(&mime) == attachment_kinds::IMAGE).then(|| image_size(bytes)).flatten();
    Ok(AttachmentPutResult {
        sha256: Some(sha),
        path: path.to_string_lossy().into_owned(),
        size: bytes.len() as u64,
        width: size.map(|(width, _)| width),
        height: size.map(|(_, height)| height),
        kind: kind_of(&mime).to_owned(),
        name,
        mime,
        stored: true,
        thumb,
    })
}

/// `chatflow.attachment.put`: bytes or a path into an attachment. `roots` are the project's root
/// and the session's workspace (canonical); a path inside one is referenced, never copied.
pub fn put(home: &Path, roots: &[PathBuf], params: AttachmentPutParams) -> Result<AttachmentPutResult, RpcError> {
    let thumb = check_thumb(params.thumb)?;
    match (params.path.filter(|path| !path.trim().is_empty()), params.data) {
        (Some(_), Some(_)) => Err(invalid("give either a path or data, not both")),
        (None, None) => Err(invalid("give a path or data to attach")),
        (None, Some(data)) => {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(data.trim())
                .map_err(|error| invalid(format!("the data is not base64: {error}")))?;
            let name = if params.name.trim().is_empty() { "attachment".to_owned() } else { params.name.trim().to_owned() };
            if bytes.len() as u64 > limits::MAX_ATTACHMENT_BYTES {
                return Err(too_big(&name, bytes.len() as u64, "an attachment"));
            }
            let mime = if params.mime.trim().is_empty() { mime_of(&name).to_owned() } else { params.mime.trim().to_owned() };
            stored_result(home, &bytes, name, mime, thumb)
        }
        (Some(path), None) => {
            let given = PathBuf::from(path.trim());
            if !given.is_absolute() {
                return Err(invalid(format!("{} is not an absolute path", given.display())));
            }
            let canonical = given.canonicalize().map_err(|error| invalid(format!("{} cannot be attached: {error}", given.display())))?;
            let file_name = canonical.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_else(|| canonical.to_string_lossy().into_owned());
            let name = if params.name.trim().is_empty() { file_name } else { params.name.trim().to_owned() };
            let metadata = std::fs::metadata(&canonical).map_err(|error| invalid(format!("{} cannot be attached: {error}", canonical.display())))?;
            if metadata.is_dir() {
                return Ok(AttachmentPutResult {
                    path: canonical.to_string_lossy().into_owned(),
                    mime: "inode/directory".into(),
                    kind: attachment_kinds::FOLDER.into(),
                    name,
                    ..AttachmentPutResult::default()
                });
            }
            let mime = if params.mime.trim().is_empty() { mime_of(&name).to_owned() } else { params.mime.trim().to_owned() };
            let incoming = home.join(AREA).join(INCOMING).canonicalize().ok();
            if incoming.as_ref().is_some_and(|incoming| canonical.starts_with(incoming)) {
                // Bytes the shell wrote for this put: they move into the store.
                let bytes = std::fs::read(&canonical).map_err(|error| invalid(format!("{} cannot be read: {error}", canonical.display())))?;
                let _ = std::fs::remove_file(&canonical);
                if bytes.len() as u64 > limits::MAX_ATTACHMENT_BYTES {
                    return Err(too_big(&name, bytes.len() as u64, "an attachment"));
                }
                return stored_result(home, &bytes, name, mime, thumb);
            }
            let kind = kind_of(&mime);
            if inside(&canonical, roots) {
                // The user's own project file: referenced where it is (hashed when it is not huge,
                // so an image can be cached by content; never copied).
                let small = metadata.len() <= limits::MAX_ATTACHMENT_BYTES;
                let bytes = if small { std::fs::read(&canonical).ok() } else { None };
                let sha256 = bytes.as_deref().map(|bytes| sha256_hex(bytes));
                let size = if kind == attachment_kinds::IMAGE {
                    bytes.as_deref().and_then(image_size).or_else(|| read_head(&canonical).and_then(|head| image_size(&head)))
                } else {
                    None
                };
                return Ok(AttachmentPutResult {
                    sha256,
                    path: canonical.to_string_lossy().into_owned(),
                    mime,
                    size: metadata.len(),
                    width: size.map(|(width, _)| width),
                    height: size.map(|(_, height)| height),
                    kind: kind.into(),
                    name,
                    stored: false,
                    thumb,
                });
            }
            if metadata.len() > limits::MAX_ATTACHMENT_BYTES {
                return Err(too_big(&name, metadata.len(), "a file from outside the project (it is copied for the model)"));
            }
            let bytes = std::fs::read(&canonical).map_err(|error| invalid(format!("{} cannot be read: {error}", canonical.display())))?;
            stored_result(home, &bytes, name, mime, thumb)
        }
    }
}

