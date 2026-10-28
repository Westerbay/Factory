use super::RuntimeConfig;
use std::{collections::HashMap, io::ErrorKind, path::Path};

impl RuntimeConfig {
    /// Read only the working directory's .env; explicit process values take precedence.
    /// Keep values local instead of mutating the environment of Tokio worker threads.
    pub fn from_environment() -> Result<Self, &'static str> {
        let directory = std::env::current_dir().map_err(|_| "WORKSPACE_UNAVAILABLE")?;
        Self::from_env_file(&directory.join(".env"), |key| std::env::var(key).ok())
    }

    fn from_env_file(
        path: &Path,
        process_value: impl Fn(&str) -> Option<String>,
    ) -> Result<Self, &'static str> {
        let values = match dotenvy::from_path_iter(path) {
            Ok(entries) => entries
                .collect::<Result<HashMap<_, _>, _>>()
                // Parser errors can contain source lines, including credentials.
                .map_err(|error| match error {
                    dotenvy::Error::Io(_) => "DOTENV_UNREADABLE",
                    _ => "DOTENV_INVALID",
                })?,
            Err(dotenvy::Error::Io(error)) if error.kind() == ErrorKind::NotFound => HashMap::new(),
            Err(_) => return Err("DOTENV_UNREADABLE"),
        };
        Self::from_values(|key| process_value(key).or_else(|| values.get(key).cloned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::PathBuf};

    struct Fixture(PathBuf);

    impl Fixture {
        fn new(contents: Option<&str>) -> Self {
            let directory =
                std::env::temp_dir().join(format!("factory-dotenv-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&directory).unwrap();
            if let Some(contents) = contents {
                fs::write(directory.join(".env"), contents).unwrap();
            }
            Self(directory)
        }

        fn path(&self) -> PathBuf {
            self.0.join(".env")
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            if self.path().is_file() {
                fs::remove_file(self.path()).unwrap();
            }
            fs::remove_dir(&self.0).unwrap();
        }
    }

    #[test]
    fn dotenv_enables_identity_and_process_values_override_it() {
        let fixture = Fixture::new(Some(
            "# Synthetic credentials only\nFACTORY_GITHUB_CLIENT_ID='file-client'\n\
             FACTORY_GITHUB_CLIENT_SECRET=\"file-secret#quoted\"\n",
        ));
        let config = RuntimeConfig::from_env_file(&fixture.path(), |_| None).unwrap();
        let identity = config.identity().unwrap();
        assert_eq!(identity.client_id, "file-client");
        assert_eq!(identity.client_secret, "file-secret#quoted");
        let overridden = RuntimeConfig::from_env_file(&fixture.path(), |key| {
            (key == "FACTORY_GITHUB_CLIENT_SECRET").then(|| "process-secret".into())
        })
        .unwrap();
        assert_eq!(
            overridden.identity().unwrap().client_secret,
            "process-secret"
        );
        let disabled = RuntimeConfig::from_env_file(&fixture.path(), |key| {
            (key == "FACTORY_GITHUB_CLIENT_SECRET").then(String::new)
        })
        .unwrap();
        assert!(disabled.identity().is_none());
    }

    #[test]
    fn missing_file_keeps_defaults_and_does_not_search_parent_directories() {
        let fixture = Fixture::new(Some("FACTORY_API_BIND=invalid\n"));
        let child = fixture.0.join("child");
        fs::create_dir(&child).unwrap();
        let config = RuntimeConfig::from_env_file(&child.join(".env"), |_| None).unwrap();
        assert_eq!(config.bind(), "127.0.0.1:3001".parse().unwrap());
        assert!(config.identity().is_none());
        fs::remove_dir(child).unwrap();
    }

    #[test]
    fn checked_in_example_starts_with_identity_disabled() {
        let fixture = Fixture::new(Some(include_str!("../../../../.env.example")));
        let config = RuntimeConfig::from_env_file(&fixture.path(), |_| None).unwrap();
        assert!(config.identity().is_none());
        assert_eq!(config.api_origin(), "http://localhost:3001");
    }

    #[test]
    fn invalid_or_unreadable_files_fail_without_disclosing_values() {
        let fixture = Fixture::new(Some("FACTORY_GITHUB_CLIENT_SECRET=\"synthetic-secret\n"));
        assert!(matches!(
            RuntimeConfig::from_env_file(&fixture.path(), |_| None),
            Err("DOTENV_INVALID")
        ));
        assert!(matches!(
            RuntimeConfig::from_env_file(&fixture.0, |_| None),
            Err("DOTENV_UNREADABLE")
        ));
    }
}
