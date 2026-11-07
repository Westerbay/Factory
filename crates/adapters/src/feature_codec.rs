//! Structured provider replies and canonical feature revision fingerprints.
use factory_core::features::{
    FeatureCodec, FeatureDiagram, FeatureError, FeatureProposal, FeatureReplyFinding,
    FeatureReviewReply, FeatureTask, GateContent,
};
use ring::digest::{Context, SHA256, digest};
use serde::Deserialize;
use uuid::Uuid;

pub struct JsonFeatureCodec;

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
enum WireProposal {
    Scope {
        brief: String,
    },
    Architecture {
        summary: String,
        diagrams: Vec<FeatureDiagram>,
        tasks: Vec<FeatureTask>,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WireReview {
    patch_digest: String,
    findings: Vec<WireFinding>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireFinding {
    summary: String,
    blocking: bool,
}

impl FeatureCodec for JsonFeatureCodec {
    fn parse_proposal(&self, raw: &str) -> Result<FeatureProposal, FeatureError> {
        let wire: WireProposal = serde_json::from_str(raw).map_err(|_| FeatureError::Invalid)?;
        Ok(match wire {
            WireProposal::Scope { brief } => FeatureProposal::Scope { brief },
            WireProposal::Architecture {
                summary,
                diagrams,
                tasks,
            } => FeatureProposal::Architecture {
                summary,
                diagrams,
                tasks,
            },
        })
    }

    fn revision_digest(&self, content: &GateContent) -> Result<String, FeatureError> {
        let canonical = serde_json::to_vec(content).map_err(|_| FeatureError::Invalid)?;
        Ok(digest(&SHA256, &canonical)
            .as_ref()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect())
    }

    fn render_revision(&self, content: &GateContent) -> Result<String, FeatureError> {
        serde_json::to_string(content).map_err(|_| FeatureError::Invalid)
    }

    fn parse_review(&self, raw: &str) -> Result<FeatureReviewReply, FeatureError> {
        let wire: WireReview = serde_json::from_str(raw).map_err(|_| FeatureError::Invalid)?;
        Ok(FeatureReviewReply {
            patch_digest: wire.patch_digest,
            findings: wire
                .findings
                .into_iter()
                .map(|finding| FeatureReplyFinding {
                    summary: finding.summary,
                    blocking: finding.blocking,
                })
                .collect(),
        })
    }

    fn derived_id(&self, namespace: Uuid, label: &str) -> Uuid {
        let mut hash = Context::new(&SHA256);
        hash.update(namespace.as_bytes());
        hash.update(label.as_bytes());
        let hash = hash.finish();
        let mut bytes = [0_u8; 16];
        bytes.copy_from_slice(&hash.as_ref()[..16]);
        Uuid::from_bytes(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structured_reply_cannot_claim_checks_or_approve_a_proposal() {
        assert!(
            JsonFeatureCodec
                .parse_review(r#"{"patchDigest":"reviewed","findings":[]}"#)
                .is_ok()
        );
        assert!(
            JsonFeatureCodec
                .parse_review(r#"{"patchDigest":"reviewed","findings":[],"checks":[true]}"#)
                .is_err()
        );
        assert!(
            JsonFeatureCodec
                .parse_proposal(r#"{"kind":"scope","brief":"A brief","approved":true}"#)
                .is_err()
        );
    }
}
