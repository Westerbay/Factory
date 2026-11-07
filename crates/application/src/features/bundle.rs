use factory_core::features::TaskRole;

// Curated Factory adaptation of Westerbay/agent-skills at
// 24d6f541f1f4b7a619e36b095fd17701212f6984. Increment this when the workflow,
// instructions, or role profiles change. An existing feature pins this value.
pub const WORKFLOW_REVISION: &str = "factory-build-feature-v1-24d6f54";

pub(super) fn role_instructions(role: TaskRole) -> &'static str {
    use TaskRole::*;
    match role {
        Coordinator => include_str!("bundle/product.md"),
        Architecture => include_str!("bundle/architecture.md"),
        Implementation | ComplexImplementation | Correction => {
            include_str!("bundle/implementation.md")
        }
        Visual => {
            "Compare the scoped implementation to the approved design. Exercise the actual interaction and report visible differences and observed checks."
        }
        Triage => {
            "Summarize the supplied evidence, grouped by consequence. Distinguish observed results, unknowns and remaining decisions."
        }
        GeneralReview => {
            "Review the exact supplied integrated diff for correctness, regressions, maintainability and behavioral test coverage. Use concrete findings with file and line evidence. Read-only."
        }
        ArchitectureReview => {
            "Critique the proposed architecture or integrated diff for responsibility, dependency direction, reuse, and the approved variations. Explain concrete failure modes. Read-only."
        }
        WebReview => {
            "Review the supplied integrated diff for user-visible behavior, accessibility, responsive layout and reusable web components. Use concrete findings tied to observable behavior. Read-only."
        }
        SecurityReview => {
            "Review the supplied integrated diff for changed trust boundaries, authorization, secrets, isolation, and unsafe I/O. Demonstrate reachability and impact. Keep the review source-based and read-only."
        }
    }
}
