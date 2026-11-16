use super::protocol::*;
use super::*;
use factory_core::attachments::MediaKind;
use serde_json::{Value, json};

const SHA: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

fn location(project: Option<Uuid>, name: &str) -> AttachmentLocation {
    AttachmentLocation::new(project, Uuid::new_v4(), Uuid::new_v4(), name).unwrap()
}

fn reply(value: Value) -> Vec<u8> {
    let mut line = serde_json::to_vec(&value).unwrap();
    line.push(b'\n');
    line
}

fn stored(size: u64, head: &str) -> Value {
    json!({"size": size, "sha256": SHA, "head": head})
}

#[test]
fn controller_runs_isolated_with_the_embedded_source_and_no_request_data_in_argv() {
    let options = FixedProcess::AttachmentController.options();
    assert_eq!(
        options.cmd,
        Some(vec![
            "/usr/bin/python3".to_owned(),
            "-I".to_owned(),
            "-S".to_owned(),
            "-c".to_owned(),
            CONTROLLER_SOURCE.to_owned(),
        ])
    );
    assert_eq!(options.user.as_deref(), Some("10000:10000"));
    assert_eq!(options.attach_stdin, Some(true));
    assert_eq!(options.attach_stdout, Some(true));
    assert_eq!(options.tty, Some(false));
    assert_eq!(options.privileged, Some(false));
    // The source is fixed text: it has no placeholders for request data.
    assert!(CONTROLLER_SOURCE.starts_with("\"\"\"Bounded attachment file operations"));
    assert!(CONTROLLER_SOURCE.contains("sys.stdin.buffer.readline"));
}

#[test]
fn requests_name_their_operation_and_scope_and_stay_bounded() {
    let project = Uuid::new_v4();
    let at = location(Some(project), "report.pdf");
    let put = put_request(&at);
    assert_eq!(put["op"], "put");
    assert_eq!(put["project"], project.to_string());
    assert_eq!(put["conversation"], at.conversation_id().to_string());
    assert_eq!(put["attachment"], at.attachment_id().to_string());
    assert_eq!(put["name"], "report.pdf");
    assert_eq!(put.as_object().unwrap().len(), 5);
    assert_eq!(put_request(&location(None, "a"))["project"], Value::Null);

    let roots = vec!["/workspace".to_owned()];
    let capture = capture_request(&at, "/workspace/out.png", &roots);
    assert_eq!(capture["op"], "capture");
    assert_eq!(capture["reference"], "/workspace/out.png");
    assert_eq!(capture["roots"], json!(["/workspace"]));

    let read = read_request(&at, 5, 10);
    assert_eq!(
        (read["offset"].as_u64(), read["length"].as_u64()),
        (Some(5), Some(10))
    );

    let removal = delete_attachment_request(&at);
    assert_eq!(removal["op"], "delete_attachment");
    assert!(removal.get("name").is_none());
    let conversation = Uuid::new_v4();
    assert_eq!(
        delete_conversation_request(None, conversation),
        json!({"op": "delete_conversation", "project": null, "conversation": conversation.to_string()})
    );
    assert_eq!(
        delete_project_request(project),
        json!({"op": "delete_project", "project": project.to_string()})
    );
    let attempt = Uuid::new_v4();
    assert_eq!(
        prepare_worker_request(project, conversation, attempt),
        json!({"op": "prepare_worker", "project": project.to_string(),
            "conversation": conversation.to_string(), "attempt": attempt.to_string()})
    );

    let line = header(&put).unwrap();
    assert!(line.ends_with(b"\n") && line.len() <= HEADER_LIMIT);
    assert_eq!(line.iter().filter(|byte| **byte == b'\n').count(), 1);
    let oversized = capture_request(&at, "/a", &vec!["/x".repeat(2000); 3]);
    assert_eq!(header(&oversized), Err(AttachmentError::Invalid));
}

#[test]
fn upload_frames_are_big_endian_and_a_zero_length_ends_the_upload() {
    assert_eq!(frame_prefix(1), [0, 0, 0, 1]);
    assert_eq!(frame_prefix(FRAME_LIMIT), [0, 0x10, 0, 0]);
    assert_eq!(END_OF_UPLOAD, [0, 0, 0, 0]);
}

#[test]
fn controller_error_codes_map_to_attachment_errors() {
    for (code, expected) in [
        ("invalid", AttachmentError::Invalid),
        ("not_found", AttachmentError::NotFound),
        ("conflict", AttachmentError::Conflict),
        ("too_large", AttachmentError::TooLarge),
        ("unavailable", AttachmentError::Unavailable),
        ("", AttachmentError::Unavailable),
        ("something new", AttachmentError::Unavailable),
    ] {
        assert_eq!(
            parse_result(&reply(json!({"ok": false, "error": code}))),
            Err(expected),
            "{code}"
        );
    }
    assert_eq!(
        parse_result(&reply(json!({"ok": false}))),
        Err(AttachmentError::Unavailable)
    );
}

#[test]
fn incomplete_or_malformed_replies_are_never_success() {
    for bytes in [
        &b""[..],
        b"not json\n",
        b"{\"ok\":true,\"result\":\"deleted\"}",
        b"{\"ok\":true}\n",
        b"{\"ok\":true,\"result\":null}\n",
        b"{\"ok\":\"yes\",\"result\":\"deleted\"}\n",
    ] {
        assert_eq!(
            parse_acknowledgement(bytes, "deleted"),
            Err(AttachmentError::Unavailable),
            "{:?}",
            String::from_utf8_lossy(bytes)
        );
    }
    assert_eq!(
        parse_acknowledgement(&reply(json!({"ok": true, "result": "deleted"})), "deleted"),
        Ok(())
    );
    assert_eq!(
        parse_acknowledgement(&reply(json!({"ok": true, "result": "ready"})), "deleted"),
        Err(AttachmentError::Unavailable)
    );
    // A non-streaming operation never carries bytes after its reply.
    let mut with_body = reply(json!({"ok": true, "result": "deleted"}));
    with_body.extend_from_slice(b"extra");
    assert_eq!(parse_result(&with_body), Err(AttachmentError::Unavailable));
}

#[test]
fn a_read_is_complete_only_when_the_announced_length_arrived() {
    let mut bytes = reply(json!({"ok": true, "result": {"length": 5}}));
    bytes.extend_from_slice(b"hello");
    assert_eq!(parse_read(&bytes), Ok(b"hello".to_vec()));
    assert_eq!(
        parse_read(&reply(json!({"ok": true, "result": {"length": 0}}))),
        Ok(Vec::new())
    );
    bytes.pop();
    assert_eq!(parse_read(&bytes), Err(AttachmentError::Unavailable));
    let mut longer = reply(json!({"ok": true, "result": {"length": 2}}));
    longer.extend_from_slice(b"toolong");
    assert_eq!(parse_read(&longer), Err(AttachmentError::Unavailable));
    assert_eq!(
        parse_read(&reply(json!({"ok": false, "error": "not_found"}))),
        Err(AttachmentError::NotFound)
    );
}

#[test]
fn stored_files_are_classified_from_their_bytes_never_a_declared_type() {
    let png = "89504e470d0a1a0a0000";
    let file = parse_stored_file("photo.txt", &stored(10, png)).unwrap();
    assert_eq!(file.kind, MediaKind::Image);
    assert_eq!(file.media_type, "image/png");
    assert_eq!((file.size, file.sha256.as_str()), (10, SHA));

    let text = "68656c6c6f"; // "hello"
    let plain = parse_stored_file("notes.md", &stored(5, text)).unwrap();
    assert_eq!(
        (plain.kind, plain.media_type.as_str()),
        (MediaKind::Text, "text/plain")
    );
    let page = parse_stored_file("page.html", &stored(5, text)).unwrap();
    assert_eq!(page.kind, MediaKind::Html);
    let empty = parse_stored_file("empty.bin", &stored(0, "")).unwrap();
    assert_eq!(empty.size, 0);
}

#[test]
fn inconsistent_stored_file_results_are_unavailable() {
    let too_large = MAX_ATTACHMENT_BYTES + 1;
    for result in [
        json!({}),
        json!({"size": "5", "sha256": SHA, "head": "68656c6c6f"}),
        json!({"size": 5, "sha256": "ABC", "head": "68656c6c6f"}),
        json!({"size": 5, "sha256": SHA.to_uppercase(), "head": "68656c6c6f"}),
        json!({"size": 5, "sha256": SHA, "head": "68656c6c6"}),
        json!({"size": 5, "sha256": SHA, "head": "zz"}),
        json!({"size": 5, "sha256": SHA, "head": "6865"}),
        json!({"size": 1, "sha256": SHA, "head": "6865"}),
        json!({"size": too_large, "sha256": SHA, "head": ""}),
        json!({"size": 5, "sha256": SHA}),
    ] {
        assert_eq!(
            parse_stored_file("a.txt", &result),
            Err(AttachmentError::Unavailable),
            "{result}"
        );
    }
    // The head is capped at the classification window, not at the file size.
    let head = "61".repeat(factory_core::attachments::CLASSIFY_HEAD_BYTES);
    assert!(parse_stored_file("a.txt", &stored(100_000, &head)).is_ok());
    let longer = "61".repeat(factory_core::attachments::CLASSIFY_HEAD_BYTES + 1);
    assert_eq!(
        parse_stored_file("a.txt", &stored(100_000, &longer)),
        Err(AttachmentError::Unavailable)
    );
}

#[test]
fn stored_name_is_the_validated_file_component() {
    assert_eq!(stored_name(&location(None, "a b.txt")), "a b.txt");
}

#[test]
fn capture_refuses_references_outside_the_roots() {
    let roots = vec!["/home/factory/Documents/project".to_owned()];
    let reference = "/home/factory/Documents/project/out.png";
    assert_eq!(validate_capture(reference, &roots), Ok(()));
    for reference in [
        "/etc/passwd",
        "relative/out.png",
        "/home/factory/Documents/project",
        "/home/factory/Documents/projectile/out.png",
        "/home/factory/Documents/project/../other/out.png",
        "/home/factory/Documents/project/./out.png",
        "/home/factory/Documents/project//out.png",
        "/home/factory/Documents/project/out.png\n",
        "",
    ] {
        assert_eq!(
            validate_capture(reference, &roots),
            Err(AttachmentError::Invalid),
            "{reference:?}"
        );
    }
    assert_eq!(
        validate_capture(reference, &[]),
        Err(AttachmentError::Invalid)
    );
    let many = vec!["/home/factory/Documents/project".to_owned(); MAX_ROOTS + 1];
    assert_eq!(
        validate_capture(reference, &many),
        Err(AttachmentError::Invalid)
    );
    let eight = &many[..MAX_ROOTS];
    assert_eq!(validate_capture(reference, eight), Ok(()));
}

#[test]
fn read_refuses_ranges_beyond_the_limits() {
    assert_eq!(validate_read(0, MAX_READ_CHUNK_BYTES), Ok(()));
    assert_eq!(validate_read(MAX_ATTACHMENT_BYTES, 0), Ok(()));
    assert_eq!(
        validate_read(0, MAX_READ_CHUNK_BYTES + 1),
        Err(AttachmentError::Invalid)
    );
    assert_eq!(
        validate_read(MAX_ATTACHMENT_BYTES + 1, 1),
        Err(AttachmentError::Invalid)
    );
}
