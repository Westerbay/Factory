use factory_core::{
    conversations::execution::ExecutionModel,
    features::{FeatureError, TaskRole},
};

#[derive(Debug, PartialEq, Eq)]
pub(super) struct ModelSelection {
    pub model: String,
    pub effort: Option<String>,
}

pub(super) fn resolve(
    provider: &str,
    role: TaskRole,
    catalog: &[ExecutionModel],
) -> Result<ModelSelection, FeatureError> {
    let profile = preferred(provider, role).ok_or(FeatureError::Invalid)?;
    let candidate = catalog
        .iter()
        .filter_map(|entry| {
            let (namespace, native_id) = entry.id.strip_prefix("factory:")?.split_once(':')?;
            if namespace != provider {
                return None;
            }
            let resolved = entry.resolved_model.as_deref();
            let rank = match provider {
                "codex" if native_id == profile => 0,
                "claude"
                    if native_id == profile && resolved.is_none_or(|id| family(id, profile)) =>
                {
                    0
                }
                "claude" if family(native_id, profile) => 1,
                _ => return None,
            };
            // A Claude alias may point to a new release between attempts. Prefer
            // its advertised concrete version when the probe can resolve it.
            let model = resolved
                .filter(|id| provider == "claude" && family(id, profile))
                .unwrap_or(native_id);
            Some((rank, model, entry))
        })
        .min_by_key(|(rank, _, _)| *rank)
        .ok_or(FeatureError::Unavailable)?;

    let preferred_effort = match role {
        TaskRole::Architecture
        | TaskRole::ArchitectureReview
        | TaskRole::GeneralReview
        | TaskRole::WebReview
        | TaskRole::SecurityReview
        | TaskRole::ComplexImplementation => "high",
        TaskRole::Triage => "low",
        _ => "medium",
    };
    let (_, model, entry) = candidate;
    let effort = [Some(preferred_effort), entry.default_effort.as_deref()]
        .into_iter()
        .flatten()
        .find(|requested| {
            entry
                .effort_levels
                .iter()
                .any(|allowed| allowed == requested)
        })
        .map(str::to_owned);
    Ok(ModelSelection {
        model: model.to_owned(),
        effort,
    })
}

pub(super) fn preferred(provider: &str, role: TaskRole) -> Option<&'static str> {
    use TaskRole::*;
    match provider {
        "codex" => Some(match role {
            Implementation | Visual | Correction => "gpt-6.1-terra",
            Architecture | ArchitectureReview | SecurityReview => "gpt-6-astra",
            Triage => "gpt-6-luna",
            Coordinator | ComplexImplementation | GeneralReview | WebReview => "gpt-6.1-sol",
        }),
        "claude" => Some(match role {
            Architecture | ArchitectureReview | SecurityReview => "opus",
            Triage => "haiku",
            _ => "sonnet",
        }),
        _ => None,
    }
}

fn family(model: &str, profile: &str) -> bool {
    model
        .strip_prefix("claude-")
        .and_then(|id| id.strip_prefix(profile))
        .and_then(|id| id.strip_prefix('-'))
        .is_some_and(|version| version.starts_with(|c: char| c.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(id: &str, resolved: Option<&str>, efforts: &[&str]) -> ExecutionModel {
        ExecutionModel {
            id: id.into(),
            name: id.into(),
            resolved_model: resolved.map(str::to_owned),
            effort_levels: efforts.iter().map(|effort| (*effort).into()).collect(),
            default_effort: None,
        }
    }

    #[test]
    fn coding_assignments_use_terra_sonnet_and_explicit_complex_sol() {
        let models = [
            model("factory:codex:gpt-6.1-terra", None, &["medium"]),
            model("factory:codex:gpt-6.1-sol", None, &["high"]),
            model(
                "factory:claude:sonnet",
                Some("claude-sonnet-5-5"),
                &["medium"],
            ),
        ];
        let terra = resolve("codex", TaskRole::Implementation, &models).unwrap();
        let sol = resolve("codex", TaskRole::ComplexImplementation, &models).unwrap();
        let sonnet = resolve("claude", TaskRole::Implementation, &models).unwrap();
        assert_eq!(terra.model, "gpt-6.1-terra");
        assert_eq!(terra.effort.as_deref(), Some("medium"));
        assert_eq!(sol.model, "gpt-6.1-sol");
        assert_eq!(sol.effort.as_deref(), Some("high"));
        assert_eq!(sonnet.model, "claude-sonnet-5-5");
    }

    #[test]
    fn unavailable_or_misrouted_models_are_not_silently_replaced() {
        let models = [
            model("factory:other:gpt-6.1-terra", None, &[]),
            model("factory:codex:gpt-6.1-sol", None, &[]),
            model("factory:claude:sonnet", Some("claude-haiku-5-5"), &[]),
        ];
        assert_eq!(
            resolve("codex", TaskRole::Implementation, &models),
            Err(FeatureError::Unavailable)
        );
        assert_eq!(
            resolve("claude", TaskRole::Implementation, &models),
            Err(FeatureError::Unavailable)
        );
        let general = resolve("codex", TaskRole::GeneralReview, &models).unwrap();
        assert_eq!(general.effort, None);
    }
}
