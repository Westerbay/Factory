use super::*;
use uuid::Uuid;

fn concat(parts: &[&[u8]]) -> Vec<u8> {
    parts.concat()
}

fn kind(name: &str, head: &[u8]) -> (MediaKind, &'static str) {
    let found = classify(name, head);
    (found.kind, found.media_type)
}

fn ftyp(major: &[u8; 4], compatible: &[&[u8; 4]]) -> Vec<u8> {
    let mut bytes = vec![0, 0, 0, (16 + 4 * compatible.len()) as u8];
    bytes.extend(b"ftyp");
    bytes.extend(major);
    bytes.extend([0; 4]);
    compatible.iter().for_each(|brand| bytes.extend(*brand));
    bytes
}

#[test]
fn limits_match_the_brief() {
    assert_eq!(MAX_ATTACHMENT_BYTES, 50 * 1024 * 1024);
    assert_eq!(MAX_ATTACHMENTS_PER_MESSAGE, 10);
}

#[test]
fn names_are_single_safe_components() {
    let longest = "a".repeat(255);
    for name in ["a.png", "résumé (1).pdf", "back\\slash", longest.as_str()] {
        assert_eq!(validate_name(name), Ok(()), "{name}");
    }
    let too_long = "a".repeat(256);
    let too_wide = "é".repeat(128);
    for name in [
        "",
        ".",
        "..",
        "a/b",
        "a\0b",
        "a\nb",
        "a\u{7f}",
        "a\u{85}",
        too_long.as_str(),
        too_wide.as_str(),
    ] {
        assert_eq!(
            validate_name(name),
            Err(AttachmentError::Invalid),
            "{name:?}"
        );
    }
}

#[test]
fn attachment_ids_are_limited_and_distinct() {
    let ids: Vec<Uuid> = (0..11).map(|_| Uuid::new_v4()).collect();
    assert_eq!(validate_attachment_ids(&ids[..10]), Ok(()));
    assert_eq!(validate_attachment_ids(&ids), Err(AttachmentError::TooMany));
    assert_eq!(
        validate_attachment_ids(&[ids[0], ids[1], ids[0]]),
        Err(AttachmentError::Invalid)
    );
}

#[test]
fn staged_files_expire_after_a_day() {
    let now = Utc::now();
    assert_eq!(now - staged_cutoff(now), Duration::hours(24));
}

#[test]
fn reference_names_are_sanitized() {
    assert_eq!(
        display_name_from_reference("/work/out/chart.png"),
        "chart.png"
    );
    assert_eq!(display_name_from_reference("/work/a\tb.txt"), "a_b.txt");
    assert_eq!(display_name_from_reference("/work/.."), "file");
    assert_eq!(display_name_from_reference("/work/"), "file");
    assert!(display_name_from_reference(&"é".repeat(300)).len() <= MAX_NAME_BYTES);
}

#[test]
fn binary_signatures_are_classified() {
    let webp = *b"RIFF\x10\0\0\0WEBPVP8 ";
    let ebml: &[u8] = &[0x1a, 0x45, 0xdf, 0xa3, 0, 0, 0, 0, 0, 0, 0, 0];
    let webm = concat(&[ebml, b"B\x82\x84webm"]);
    let mkv = concat(&[ebml, b"B\x82\x88matroska"]);
    let cases: [(&[u8], MediaKind, &str); 10] = [
        (b"\x89PNG\r\n\x1a\n....", MediaKind::Image, "image/png"),
        (&[0xff, 0xd8, 0xff, 0xe0], MediaKind::Image, "image/jpeg"),
        (b"GIF89a....", MediaKind::Image, "image/gif"),
        (&webp, MediaKind::Image, "image/webp"),
        (&ftyp(b"avif", &[]), MediaKind::Image, "image/avif"),
        (
            &ftyp(b"mif1", &[b"miaf", b"avif"]),
            MediaKind::Image,
            "image/avif",
        ),
        (&ftyp(b"isom", &[b"avc1"]), MediaKind::Video, "video/mp4"),
        (&webm, MediaKind::Video, "video/webm"),
        (b"OggS\0\x02", MediaKind::Video, "video/ogg"),
        (b"%PDF-1.7\n", MediaKind::Pdf, "application/pdf"),
    ];
    for (head, expected_kind, expected_type) in cases {
        // The name never overrides a signature.
        assert_eq!(kind("file.txt", head), (expected_kind, expected_type));
    }
    assert_eq!(kind("a.mkv", &mkv), (MediaKind::File, OCTET_STREAM));
    assert_eq!(
        kind("a.heic", &ftyp(b"heic", &[b"mif1"])),
        (MediaKind::File, OCTET_STREAM)
    );
    assert_eq!(
        kind("a.png", b"RIFF\0\0\0\0WAVE"),
        (MediaKind::File, OCTET_STREAM)
    );
}

#[test]
fn text_is_utf8_only() {
    assert_eq!(
        kind("notes.md", b"# Title\n\ttab\r\n"),
        (MediaKind::Text, "text/plain")
    );
    assert_eq!(kind("empty", b""), (MediaKind::Text, "text/plain"));
    assert_eq!(
        kind("bom.txt", "\u{feff}héllo".as_bytes()),
        (MediaKind::Text, "text/plain")
    );
    // A head cut inside a character is a truncated read, not binary data.
    let cut = &"é".as_bytes()[..1];
    assert_eq!(
        kind("a.txt", &concat(&[b"ok", cut])),
        (MediaKind::Text, "text/plain")
    );
    let binary: [&[u8]; 4] = [
        b"a\0b",
        b"\xff\xfe text",
        b"caf\xe9 latin1",
        b"\x01\x02\x03",
    ];
    for head in binary {
        assert_eq!(kind("a.txt", head), (MediaKind::File, OCTET_STREAM));
    }
}

#[test]
fn html_and_svg_are_recognized_as_text() {
    let svg = b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>";
    let prolog = b"<?xml version=\"1.0\"?>\n<svg/>";
    assert_eq!(kind("a.svg", svg), (MediaKind::Svg, "image/svg+xml"));
    assert_eq!(kind("noext", svg), (MediaKind::Svg, "image/svg+xml"));
    assert_eq!(kind("a.xml", prolog), (MediaKind::Svg, "image/svg+xml"));
    assert_eq!(kind("a.svg", b"just text"), (MediaKind::Text, "text/plain"));
    assert_eq!(
        kind("a.txt", b"x <svg/> y"),
        (MediaKind::Text, "text/plain")
    );
    assert_eq!(kind("a.HTML", b"hello"), (MediaKind::Html, "text/html"));
    assert_eq!(kind("a.htm", b"<p>x</p>"), (MediaKind::Html, "text/html"));
    assert_eq!(
        kind("page", b"\n <!DOCTYPE HTML><p>"),
        (MediaKind::Html, "text/html")
    );
    assert_eq!(
        kind("page", b"<HTML><body>"),
        (MediaKind::Html, "text/html")
    );
    assert_eq!(
        kind("page.txt", b"<div>not a document</div>"),
        (MediaKind::Text, "text/plain")
    );
    assert_eq!(
        kind("é.txt", "é<!doctype".as_bytes()),
        (MediaKind::Text, "text/plain")
    );
}

#[test]
fn only_the_leading_bytes_are_examined() {
    let mut head = vec![b'a'; CLASSIFY_HEAD_BYTES];
    head.push(0);
    assert_eq!(kind("a.txt", &head), (MediaKind::Text, "text/plain"));
}

#[test]
fn served_responses_follow_the_stored_classification() {
    let sandbox = "default-src 'none'; style-src 'unsafe-inline'; sandbox";
    for (kind, media_type, content_type) in [
        (MediaKind::Image, "image/png", "image/png"),
        (MediaKind::Video, "video/webm", "video/webm"),
        (MediaKind::Svg, "image/svg+xml", "image/svg+xml"),
        (MediaKind::Text, "text/plain", "text/plain; charset=utf-8"),
        (MediaKind::Html, "text/html", "text/plain; charset=utf-8"),
    ] {
        let served = served_response(kind, media_type);
        assert_eq!(served.content_type, content_type);
        assert_eq!(served.disposition, Disposition::Inline);
        assert_eq!(
            served.content_security_policy(&["https://app.example"]),
            sandbox
        );
    }
    let file = served_response(MediaKind::File, "application/zip");
    assert_eq!(file.content_type, OCTET_STREAM);
    assert_eq!(file.disposition, Disposition::Attachment);
    assert_eq!(file.content_security_policy(&[]), sandbox);
    // An inconsistent record is never inlined with its claimed type.
    assert_eq!(served_response(MediaKind::Image, "text/html"), file);
    assert_eq!(served_response(MediaKind::Video, "image/png"), file);
}

#[test]
fn pdf_is_not_sandboxed_but_restricts_embedding() {
    let pdf = served_response(MediaKind::Pdf, "application/pdf");
    assert_eq!(pdf.content_type, "application/pdf");
    assert_eq!(pdf.disposition, Disposition::Inline);
    assert_eq!(
        pdf.content_security_policy(&["https://app.example", "https://b.example"]),
        "frame-ancestors https://app.example https://b.example"
    );
    assert_eq!(pdf.content_security_policy(&[]), "frame-ancestors 'none'");
    assert_eq!(
        pdf.content_security_policy(&["https://a.example; script-src *", "a b", ""]),
        "frame-ancestors 'none'"
    );
}

#[test]
fn store_paths_separate_chats_and_projects() {
    let (project, conversation, attachment) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    assert_eq!(
        conversation_store(None, conversation),
        format!(".factory/attachments/chats/{conversation}")
    );
    assert_eq!(
        conversation_store(Some(project), conversation),
        format!("{}/{conversation}", project_store(project))
    );
    let location =
        AttachmentLocation::new(Some(project), conversation, attachment, "a.png").unwrap();
    assert_eq!(
        location.file(),
        format!(".factory/attachments/{project}/{conversation}/{attachment}/a.png")
    );
    assert!(location.file().starts_with(&location.directory()));
    assert!(
        worker_outputs_store(Some(project), conversation, attachment)
            .starts_with(&conversation_store(Some(project), conversation))
    );
    for name in ["", "..", "a/b"] {
        assert_eq!(
            AttachmentLocation::new(None, conversation, attachment, name),
            Err(AttachmentError::Invalid)
        );
    }
}

#[test]
fn references_stay_inside_the_allowed_roots() {
    let roots = [
        "/home/u/work",
        "/workspace/outputs/",
        "/home/u/.factory/attachments/chats/c",
    ];
    let ok = |reference: &str| resolve_reference(reference, "/home/u/work/app", &roots);
    let too_long = format!("/home/u/work/{}", "a".repeat(1024));
    assert_eq!(
        ok("/home/u/work/out/a.png").unwrap(),
        "/home/u/work/out/a.png"
    );
    assert_eq!(ok("out//./a.png").unwrap(), "/home/u/work/app/out/a.png");
    assert_eq!(ok("../shared/a.png").unwrap(), "/home/u/work/shared/a.png");
    assert_eq!(
        ok("/workspace/outputs/r.html").unwrap(),
        "/workspace/outputs/r.html"
    );
    assert_eq!(ok("/home/u/work/x/../y.txt").unwrap(), "/home/u/work/y.txt");
    for reference in [
        "",
        "/home/u/work",
        "/home/u/work/",
        "/home/u/work/../.ssh/id_rsa",
        "/home/u/work/../../../../etc/passwd",
        "../../.ssh/id_rsa",
        "/home/u/workspace/a.png",
        "/etc/passwd",
        "/home/u/work/a\0b",
        "/home/u/work/a\nb",
        too_long.as_str(),
    ] {
        assert_eq!(
            ok(reference),
            Err(AttachmentError::Invalid),
            "{reference:?}"
        );
    }
    let longest = format!("/home/u/work/{}", "a".repeat(1024 - 13));
    assert_eq!(ok(&longest).unwrap(), longest);
}

#[test]
fn reference_roots_and_base_fail_closed() {
    let no_roots: [&str; 0] = [];
    assert!(resolve_reference("/a/b", "/a", &no_roots).is_err());
    assert!(resolve_reference("/a/b", "/a", &["/"]).is_err());
    assert!(resolve_reference("/a/b", "/a", &["a"]).is_err());
    assert!(resolve_reference("b", "relative", &["/a"]).is_err());
    assert!(resolve_reference("..", "/", &["/a"]).is_err());
}

#[test]
fn block_is_removed_and_references_collected() {
    let found = extract_factory_files(
        "Here is the chart.\n\n```factory-files\n/work/a.png\n  /work/b.pdf  \n\n/work/a.png\n```\n\nDone.",
    );
    assert_eq!(found.text, "Here is the chart.\n\nDone.");
    assert_eq!(found.references, ["/work/a.png", "/work/b.pdf"]);
    assert_eq!(found.ignored, 0);
}

#[test]
fn text_without_a_block_is_unchanged() {
    for text in [
        "",
        "plain\n",
        "trailing newline kept\n\n",
        "```rust\nfn main() {}\n```\n",
        "```factory-files-extra\n/a\n```",
        "```factory-files\n/unclosed",
        "inline ```factory-files``` mention",
        "    ```factory-files\n/indented code\n    ```",
    ] {
        let found = extract_factory_files(text);
        assert_eq!(found.text, text);
        assert!(found.references.is_empty(), "{text:?}");
    }
}

#[test]
fn blocks_inside_other_code_are_examples() {
    let text = "Use:\n````markdown\n```factory-files\n/work/a.png\n```\n````\n\n~~~\n```factory-files\n/work/b.png\n```\n~~~";
    let found = extract_factory_files(text);
    assert_eq!(found.text, text);
    assert!(found.references.is_empty());
}

#[test]
fn several_blocks_share_one_limit() {
    let first: String = (0..8).map(|i| format!("/w/{i}.txt\n")).collect();
    let second: String = (6..12).map(|i| format!("/w/{i}.txt\n")).collect();
    let text = format!("a\n```factory-files\n{first}```\nb\n```factory-files\n{second}```\nc");
    let found = extract_factory_files(&text);
    assert_eq!(found.text, "a\nb\nc");
    assert_eq!(found.references.len(), MAX_ATTACHMENTS_PER_MESSAGE);
    assert_eq!(found.references[9], "/w/9.txt");
    assert_eq!(found.ignored, 2);
}

#[test]
fn malformed_lines_are_ignored_not_fatal() {
    let long = "a".repeat(MAX_REFERENCE_BYTES + 1);
    let found = extract_factory_files(&format!(
        "```factory-files\n/ok.txt\n{long}\n/bad\u{1}.txt\n```"
    ));
    assert_eq!(found.text, "");
    assert_eq!(found.references, ["/ok.txt"]);
    assert_eq!(found.ignored, 2);
}

#[test]
fn removal_leaves_no_gap_and_handles_crlf() {
    let found = extract_factory_files("a\r\n\r\n```factory-files\r\n/x.txt\r\n```\r\n\r\nb\r\n");
    assert_eq!(found.text, "a\n\nb");
    assert_eq!(found.references, ["/x.txt"]);
    let only = extract_factory_files("```factory-files\n```");
    assert_eq!((only.text.as_str(), only.references.len()), ("", 0));
    let tilde = extract_factory_files("~~~factory-files\n/x\n~~~");
    assert_eq!(tilde.text, "~~~factory-files\n/x\n~~~");
}

#[test]
fn enums_round_trip_through_their_storage_names() {
    for kind in [
        MediaKind::Image,
        MediaKind::Svg,
        MediaKind::Video,
        MediaKind::Pdf,
        MediaKind::Text,
        MediaKind::Html,
        MediaKind::File,
    ] {
        assert_eq!(MediaKind::parse(kind.as_str()), Some(kind));
    }
    for state in [
        AttachmentState::Uploading,
        AttachmentState::Staged,
        AttachmentState::Attached,
        AttachmentState::Unavailable,
    ] {
        assert_eq!(AttachmentState::parse(state.as_str()), Some(state));
    }
    for source in [AttachmentSource::User, AttachmentSource::Agent] {
        assert_eq!(AttachmentSource::parse(source.as_str()), Some(source));
    }
    assert_eq!(MediaKind::parse("audio"), None);
}
