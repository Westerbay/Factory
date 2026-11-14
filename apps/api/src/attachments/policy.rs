//! Response headers for stored bytes. The type and disposition come from the core
//! policy for the stored classification; nothing is taken from the client.
use axum::http::{HeaderMap, HeaderName, HeaderValue, header};
use factory_core::attachments::{
    Attachment, CROSS_ORIGIN_RESOURCE_POLICY, Disposition, NOSNIFF, served_response,
};

const RESOURCE_POLICY: HeaderName = HeaderName::from_static("cross-origin-resource-policy");

/// `download` forces a save dialog for any kind. `web_origin` is the only origin
/// allowed to frame a PDF.
pub(super) fn headers(attachment: &Attachment, web_origin: &str, download: bool) -> HeaderMap {
    let served = served_response(attachment.kind, &attachment.media_type);
    let disposition = if download {
        Disposition::Attachment
    } else {
        served.disposition
    };
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(served.content_type),
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        content_disposition(disposition, &attachment.name),
    );
    if let Ok(policy) = HeaderValue::from_str(&served.content_security_policy(&[web_origin])) {
        headers.insert(header::CONTENT_SECURITY_POLICY, policy);
    }
    headers.insert(
        RESOURCE_POLICY,
        HeaderValue::from_static(CROSS_ORIGIN_RESOURCE_POLICY),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static(NOSNIFF),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    headers
}

/// An ASCII fallback plus the RFC 5987 `filename*` so any stored name survives.
fn content_disposition(disposition: Disposition, name: &str) -> HeaderValue {
    let kind = match disposition {
        Disposition::Inline => "inline",
        Disposition::Attachment => "attachment",
    };
    let fallback: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_graphic() && !matches!(c, '"' | '\\' | '%') || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let mut encoded = String::with_capacity(name.len());
    for byte in name.bytes() {
        if byte.is_ascii_alphanumeric()
            || matches!(
                byte,
                b'!' | b'#' | b'$' | b'&' | b'+' | b'-' | b'.' | b'^' | b'_' | b'`' | b'|' | b'~'
            )
        {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    HeaderValue::from_str(&format!(
        "{kind}; filename=\"{fallback}\"; filename*=UTF-8''{encoded}"
    ))
    .unwrap_or_else(|_| HeaderValue::from_static(kind))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn disposition(name: &str) -> String {
        content_disposition(Disposition::Attachment, name)
            .to_str()
            .unwrap()
            .to_owned()
    }

    #[test]
    fn names_are_encoded_without_breaking_the_header() {
        assert_eq!(
            disposition("report.pdf"),
            "attachment; filename=\"report.pdf\"; filename*=UTF-8''report.pdf"
        );
        assert_eq!(
            disposition("é \"q\"; x.txt"),
            "attachment; filename=\"_ _q_; x.txt\"; filename*=UTF-8''%C3%A9%20%22q%22%3B%20x.txt"
        );
        assert_eq!(
            disposition("100%'*.png"),
            "attachment; filename=\"100_'*.png\"; filename*=UTF-8''100%25%27%2A.png"
        );
    }
}
