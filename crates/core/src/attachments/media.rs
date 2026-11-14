//! Pure media classification by signature and the policy for serving stored bytes.
use super::MediaKind;

/// Only this many leading bytes are examined.
pub const CLASSIFY_HEAD_BYTES: usize = 8192;
pub const NOSNIFF: &str = "nosniff";
pub const CROSS_ORIGIN_RESOURCE_POLICY: &str = "same-site";
pub const OCTET_STREAM: &str = "application/octet-stream";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Classification {
    pub media_type: &'static str,
    pub kind: MediaKind,
}

const fn class(media_type: &'static str, kind: MediaKind) -> Classification {
    Classification { media_type, kind }
}

const FILE: Classification = class(OCTET_STREAM, MediaKind::File);
const MP4_BRANDS: [&[u8; 4]; 15] = [
    b"isom", b"iso2", b"iso3", b"iso4", b"iso5", b"iso6", b"iso7", b"iso8", b"iso9", b"mp41",
    b"mp42", b"avc1", b"dash", b"M4V ", b"mmp4",
];

/// Classify from the file name and its leading bytes. Signatures win over the name;
/// text is UTF-8 only, and a head cut inside a character is accepted.
pub fn classify(name: &str, head: &[u8]) -> Classification {
    let head = &head[..head.len().min(CLASSIFY_HEAD_BYTES)];
    if let Some(found) = classify_binary(head) {
        return found;
    }
    let Some(text) = utf8_text(head) else {
        return FILE;
    };
    let trimmed = text.trim_start_matches('\u{feff}').trim_start();
    let extension = name
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase());
    let extension = extension.as_deref();
    if text.contains("<svg")
        && (extension == Some("svg") || trimmed.starts_with("<svg") || trimmed.starts_with("<?xml"))
    {
        return class("image/svg+xml", MediaKind::Svg);
    }
    let lower: String = trimmed
        .chars()
        .take(14)
        .collect::<String>()
        .to_ascii_lowercase();
    if matches!(extension, Some("html" | "htm"))
        || lower.starts_with("<!doctype html")
        || lower.starts_with("<html")
    {
        return class("text/html", MediaKind::Html);
    }
    class("text/plain", MediaKind::Text)
}

fn classify_binary(head: &[u8]) -> Option<Classification> {
    if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some(class("image/png", MediaKind::Image));
    }
    if head.starts_with(&[0xff, 0xd8, 0xff]) {
        return Some(class("image/jpeg", MediaKind::Image));
    }
    if head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a") {
        return Some(class("image/gif", MediaKind::Image));
    }
    if head.len() >= 12 && &head[..4] == b"RIFF" && &head[8..12] == b"WEBP" {
        return Some(class("image/webp", MediaKind::Image));
    }
    if head.starts_with(b"%PDF-") {
        return Some(class("application/pdf", MediaKind::Pdf));
    }
    if head.starts_with(b"OggS") {
        return Some(class("video/ogg", MediaKind::Video));
    }
    if head.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
        // Matroska shares the container; only the WebM document type is playable.
        let probe = &head[..head.len().min(64)];
        return Some(if probe.windows(4).any(|w| w == b"webm") {
            class("video/webm", MediaKind::Video)
        } else {
            FILE
        });
    }
    if head.len() >= 12 && &head[4..8] == b"ftyp" {
        return Some(classify_ftyp(head));
    }
    None
}

fn classify_ftyp(head: &[u8]) -> Classification {
    let major = &head[8..12];
    let size = u32::from_be_bytes([head[0], head[1], head[2], head[3]]) as usize;
    let end = size.min(head.len());
    let compatible = head[16.min(end)..end].chunks_exact(4);
    let avif = |brand: &[u8]| brand == b"avif" || brand == b"avis";
    if avif(major) || (matches!(major, b"mif1" | b"miaf" | b"msf1") && compatible.clone().any(avif))
    {
        return class("image/avif", MediaKind::Image);
    }
    if MP4_BRANDS.iter().any(|brand| brand.as_slice() == major) {
        return class("video/mp4", MediaKind::Video);
    }
    FILE
}

/// The valid text prefix of `head`, or `None` when it is not UTF-8 text.
fn utf8_text(head: &[u8]) -> Option<&str> {
    let text = match std::str::from_utf8(head) {
        Ok(text) => text,
        // An incomplete final sequence is a truncated read, not binary data.
        Err(error) if error.error_len().is_none() => {
            std::str::from_utf8(&head[..error.valid_up_to()]).ok()?
        }
        Err(_) => return None,
    };
    let plain = text
        .chars()
        .all(|c| !c.is_control() || matches!(c, '\t' | '\n' | '\r' | '\x0c' | '\x1b'));
    plain.then_some(text)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Disposition {
    Inline,
    Attachment,
}

/// How a response may constrain the document it carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContentPolicy {
    /// Sandboxed with no sources: the default for everything but PDF.
    Sandbox,
    /// Browser PDF viewers do not run in a sandbox, so only embedding is restricted.
    FrameAncestors,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ServedResponse {
    pub content_type: &'static str,
    pub disposition: Disposition,
    pub policy: ContentPolicy,
}

impl ServedResponse {
    /// `embedders` are the origins that may frame the content; anything that could
    /// alter the header structure is dropped, and none means `'none'`.
    pub fn content_security_policy(&self, embedders: &[&str]) -> String {
        match self.policy {
            ContentPolicy::Sandbox => {
                "default-src 'none'; style-src 'unsafe-inline'; sandbox".into()
            }
            ContentPolicy::FrameAncestors => {
                let origins: Vec<&str> = embedders
                    .iter()
                    .copied()
                    .filter(|origin| {
                        let unsafe_char =
                            |c: char| c.is_whitespace() || c.is_control() || matches!(c, ';' | ',');
                        !origin.is_empty() && !origin.chars().any(unsafe_char)
                    })
                    .collect();
                if origins.is_empty() {
                    "frame-ancestors 'none'".into()
                } else {
                    format!("frame-ancestors {}", origins.join(" "))
                }
            }
        }
    }
}

const IMAGE_TYPES: [&str; 5] = [
    "image/png",
    "image/jpeg",
    "image/gif",
    "image/webp",
    "image/avif",
];
const VIDEO_TYPES: [&str; 3] = ["video/mp4", "video/webm", "video/ogg"];

/// Response headers for stored bytes, derived from the stored classification and never
/// from the client. Text and HTML are served as plain text and rendered by the browser
/// app; SVG is meant for an image element only. Unknown combinations are downloads.
pub fn served_response(kind: MediaKind, media_type: &str) -> ServedResponse {
    let inline = |content_type: &'static str, policy: ContentPolicy| ServedResponse {
        content_type,
        disposition: Disposition::Inline,
        policy,
    };
    let from = |allowed: &[&'static str]| allowed.iter().copied().find(|t| *t == media_type);
    let served = match kind {
        MediaKind::Image => from(&IMAGE_TYPES).map(|t| inline(t, ContentPolicy::Sandbox)),
        MediaKind::Video => from(&VIDEO_TYPES).map(|t| inline(t, ContentPolicy::Sandbox)),
        MediaKind::Svg => Some(inline("image/svg+xml", ContentPolicy::Sandbox)),
        MediaKind::Pdf => Some(inline("application/pdf", ContentPolicy::FrameAncestors)),
        MediaKind::Text | MediaKind::Html => {
            Some(inline("text/plain; charset=utf-8", ContentPolicy::Sandbox))
        }
        MediaKind::File => None,
    };
    served.unwrap_or(ServedResponse {
        content_type: OCTET_STREAM,
        disposition: Disposition::Attachment,
        policy: ContentPolicy::Sandbox,
    })
}
