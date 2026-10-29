use factory_adapters::environments::ResourcePolicy;

#[derive(Clone)]
pub struct EnvironmentConfig {
    pub max_running: u32,
    pub lease_seconds: u32,
    pub resources: ResourcePolicy,
}

impl EnvironmentConfig {
    pub(super) fn from_values(get: &impl Fn(&str) -> Option<String>) -> Result<Self, &'static str> {
        let max_running = number(get, "FACTORY_ENV_MAX_RUNNING", "2")?;
        let minutes: u32 = number(get, "FACTORY_ENV_IDLE_MINUTES", "15")?;
        let lease_seconds = minutes
            .checked_mul(60)
            .ok_or("ENVIRONMENT_CONFIG_INVALID")?;
        let memory_mib = number(get, "FACTORY_ENV_MEMORY_MIB", "1024")?;
        let cpus = number(get, "FACTORY_ENV_CPUS", "1")?;
        let pids = number(get, "FACTORY_ENV_PIDS", "128")?;
        if max_running == 0 || lease_seconds == 0 {
            return Err("ENVIRONMENT_CONFIG_INVALID");
        }
        let resources = ResourcePolicy::new(memory_mib, cpus, pids)
            .map_err(|_| "ENVIRONMENT_CONFIG_INVALID")?;
        Ok(Self {
            max_running,
            lease_seconds,
            resources,
        })
    }
}

fn number<T: std::str::FromStr>(
    get: &impl Fn(&str) -> Option<String>,
    name: &str,
    fallback: &str,
) -> Result<T, &'static str> {
    get(name)
        .unwrap_or_else(|| fallback.into())
        .parse()
        .map_err(|_| "ENVIRONMENT_CONFIG_INVALID")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_and_duration_conversions_reject_invalid_or_overflowing_values() {
        for (key, values) in [
            ("FACTORY_ENV_MAX_RUNNING", vec!["0", "-1", "4294967296"]),
            ("FACTORY_ENV_IDLE_MINUTES", vec!["0", "-1", "4294967295"]),
            ("FACTORY_ENV_MEMORY_MIB", vec!["0", "18446744073709551615"]),
            (
                "FACTORY_ENV_CPUS",
                vec!["0", "-1", "NaN", "inf", "1e100", "1e-100"],
            ),
            ("FACTORY_ENV_PIDS", vec!["0", "18446744073709551615"]),
        ] {
            for value in values {
                assert!(
                    EnvironmentConfig::from_values(&|name| (name == key).then(|| value.to_owned()))
                        .is_err(),
                    "{key}={value}"
                );
            }
        }
        let config = EnvironmentConfig::from_values(&|name| {
            (name == "FACTORY_ENV_CPUS").then(|| "0.5".into())
        })
        .unwrap();
        assert_eq!(config.max_running, 2);
        assert_eq!(config.lease_seconds, 900);
    }
}
