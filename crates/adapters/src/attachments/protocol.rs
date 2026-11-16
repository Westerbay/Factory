//! Wire format shared with `controller.py`: one JSON request line on stdin, followed
//! by length-prefixed frames for an upload, and one JSON reply line on stdout, followed
//! by the raw bytes of a read. Nothing here is ever passed as a process argument.
use factory_core::attachments::{
    AttachmentError, AttachmentLocation, CLASSIFY_HEAD_BYTES, MAX_ATTACHMENT_BYTES, StoredFile,
    classify,
};
use serde_json::{Value, json};
use uuid::Uuid;

pub(super) const HEADER_LIMIT: usize = 8192;
/// Largest upload frame; the controller refuses anything bigger.
pub(super) const FRAME_LIMIT: usize = 1024 * 1024;
/// Replies that carry no file bytes.
pub(super) const REPLY_LIMIT: usize = 32 * 1024;
/// The JSON line before the bytes of a read.
pub(super) const READ_HEADER_LIMIT: usize = 256;

pub(super) fn header(request: &Value) -> Result<Vec<u8>, AttachmentError> {
    let mut line = serde_json::to_vec(request).map_err(|_| AttachmentError::Invalid)?;
    line.push(b'\n');
    if line.len() > HEADER_LIMIT {
        return Err(AttachmentError::Invalid);
    }
    Ok(line)
}

/// The length prefix of one upload frame; a zero length ends the upload.
pub(super) fn frame_prefix(length: usize) -> [u8; 4] {
    u32::try_from(length)
        .expect("frames never exceed FRAME_LIMIT")
        .to_be_bytes()
}

pub(super) const END_OF_UPLOAD: [u8; 4] = [0; 4];

fn scope(location_project: Option<Uuid>) -> Value {
    location_project.map_or(Value::Null, |id| json!(id.to_string()))
}

/// The display name is the last component of the validated stored file path.
fn name_of(location: &AttachmentLocation) -> String {
    let file = location.file();
    file.rsplit('/').next().unwrap_or_default().to_owned()
}

fn locate(op: &str, location: &AttachmentLocation) -> Value {
    json!({
        "op": op,
        "project": scope(location.project_id()),
        "conversation": location.conversation_id().to_string(),
        "attachment": location.attachment_id().to_string(),
        "name": name_of(location),
    })
}

pub(super) fn put_request(location: &AttachmentLocation) -> Value {
    locate("put", location)
}

pub(super) fn capture_request(
    location: &AttachmentLocation,
    reference: &str,
    roots: &[String],
) -> Value {
    let mut request = locate("capture", location);
    request["reference"] = json!(reference);
    request["roots"] = json!(roots);
    request
}

pub(super) fn read_request(location: &AttachmentLocation, offset: u64, length: u64) -> Value {
    let mut request = locate("read", location);
    request["offset"] = json!(offset);
    request["length"] = json!(length);
    request
}

pub(super) fn delete_attachment_request(location: &AttachmentLocation) -> Value {
    json!({
        "op": "delete_attachment",
        "project": scope(location.project_id()),
        "conversation": location.conversation_id().to_string(),
        "attachment": location.attachment_id().to_string(),
    })
}

pub(super) fn delete_conversation_request(project: Option<Uuid>, conversation: Uuid) -> Value {
    json!({
        "op": "delete_conversation",
        "project": scope(project),
        "conversation": conversation.to_string(),
    })
}

pub(super) fn delete_project_request(project: Uuid) -> Value {
    json!({"op": "delete_project", "project": project.to_string()})
}

pub(super) fn prepare_worker_request(project: Uuid, conversation: Uuid, attempt: Uuid) -> Value {
    json!({
        "op": "prepare_worker",
        "project": project.to_string(),
        "conversation": conversation.to_string(),
        "attempt": attempt.to_string(),
    })
}

pub(super) fn map_error(code: &str) -> AttachmentError {
    match code {
        "invalid" => AttachmentError::Invalid,
        "not_found" => AttachmentError::NotFound,
        "conflict" => AttachmentError::Conflict,
        "too_large" => AttachmentError::TooLarge,
        _ => AttachmentError::Unavailable,
    }
}

pub(super) struct Reply<'a> {
    pub result: Value,
    /// Bytes after the reply line; only a read has any.
    pub body: &'a [u8],
}

/// Anything that is not a complete, well-formed reply is `Unavailable`: the caller
/// cannot tell what the controller did.
pub(super) fn parse_reply(bytes: &[u8]) -> Result<Reply<'_>, AttachmentError> {
    let end = bytes
        .iter()
        .position(|byte| *byte == b'\n')
        .ok_or(AttachmentError::Unavailable)?;
    let (line, body) = (&bytes[..end], &bytes[end + 1..]);
    let reply: Value = serde_json::from_slice(line).map_err(|_| AttachmentError::Unavailable)?;
    if reply["ok"] == true {
        let result = reply
            .get("result")
            .filter(|value| !value.is_null())
            .cloned()
            .ok_or(AttachmentError::Unavailable)?;
        return Ok(Reply { result, body });
    }
    Err(map_error(reply["error"].as_str().unwrap_or_default()))
}

/// A reply of a non-streaming operation: success carries a result and no bytes.
pub(super) fn parse_result(bytes: &[u8]) -> Result<Value, AttachmentError> {
    let reply = parse_reply(bytes)?;
    if reply.body.is_empty() {
        Ok(reply.result)
    } else {
        Err(AttachmentError::Unavailable)
    }
}

pub(super) fn parse_acknowledgement(bytes: &[u8], expected: &str) -> Result<(), AttachmentError> {
    if parse_result(bytes)?.as_str() == Some(expected) {
        Ok(())
    } else {
        Err(AttachmentError::Unavailable)
    }
}

/// The bytes of a read; the announced length must match what arrived.
pub(super) fn parse_read(bytes: &[u8]) -> Result<Vec<u8>, AttachmentError> {
    let reply = parse_reply(bytes)?;
    if reply.result["length"].as_u64() == Some(reply.body.len() as u64) {
        Ok(reply.body.to_vec())
    } else {
        Err(AttachmentError::Unavailable)
    }
}

fn decode_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) || !text.is_ascii() {
        return None;
    }
    text.as_bytes()
        .chunks(2)
        .map(|pair| {
            let digit = |byte: u8| char::from(byte).to_digit(16);
            Some((digit(pair[0])? * 16 + digit(pair[1])?) as u8)
        })
        .collect()
}

/// Validate the controller's digest of stored bytes and classify them from their
/// head. The client-declared type never reaches this point.
pub(super) fn parse_stored_file(name: &str, result: &Value) -> Result<StoredFile, AttachmentError> {
    let size = result["size"]
        .as_u64()
        .ok_or(AttachmentError::Unavailable)?;
    let sha256 = result["sha256"]
        .as_str()
        .filter(|digest| {
            digest.len() == 64
                && digest
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
        .ok_or(AttachmentError::Unavailable)?;
    let head = result["head"]
        .as_str()
        .and_then(decode_hex)
        .ok_or(AttachmentError::Unavailable)?;
    if size > MAX_ATTACHMENT_BYTES || head.len() as u64 != size.min(CLASSIFY_HEAD_BYTES as u64) {
        return Err(AttachmentError::Unavailable);
    }
    let class = classify(name, &head);
    Ok(StoredFile {
        media_type: class.media_type.to_owned(),
        kind: class.kind,
        size,
        sha256: sha256.to_owned(),
    })
}

pub(super) fn stored_name(location: &AttachmentLocation) -> String {
    name_of(location)
}
