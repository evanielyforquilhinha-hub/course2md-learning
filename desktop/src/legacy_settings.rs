//! One-time import of pre-v2 preferences. The original TOML is never rewritten.

use crate::credentials::{CredentialRef, CredentialVault, Secret};
use crate::preferences::{
    ApplicationPreferences, Authentication, BindingScope, GenerationPreferences, PreferenceGroup,
    ServiceDraft, ServiceProtocol, Store,
};
use course2md::settings::ConfigFile;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

pub(crate) struct Bootstrap {
    pub store: Store,
    pub library: Option<PathBuf>,
    pub message: Option<String>,
}

/// Called once during startup, before UI construction and before the workspace is opened.
/// This only reads the old settings file and writes local preferences; it never tests services.
pub(crate) fn bootstrap(directory: &Path, vault: Arc<dyn CredentialVault>) -> Bootstrap {
    let root = directory.join("desktop-preferences");
    let mut library = None;
    let mut message = None;
    if matches!(absent(&root), Ok(true)) && directory.join("config.toml").is_file() {
        // `load` intentionally reports malformed old TOML rather than inventing defaults.
        // Do not show its detailed error: it can include an offending credential-bearing line.
        match course2md::settings::load() {
            Ok(config) => {
                library = initial_library(directory, &config);
                match try_import(directory, &config, Arc::clone(&vault)) {
                    Ok(true) => {
                        message = Some(
                            course2md::tr!(
                                "已导入旧版设置和服务，原 config.toml 已保留；尚未发送 API 请求。"
                            )
                            .into(),
                        )
                    }
                    Ok(false) => {}
                    Err(_) => message = Some(import_failure().into()),
                }
            }
            Err(_) => message = Some(import_failure().into()),
        }
    }
    Bootstrap {
        store: Store::open(root, vault),
        library,
        message,
    }
}

fn initial_library(directory: &Path, config: &ConfigFile) -> Option<PathBuf> {
    matches!(absent(&directory.join("desktop-workspace.json")), Ok(true))
        .then(|| config.defaults.out.clone())
        .flatten()
}

fn absent(path: &Path) -> std::io::Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(false),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(error) => Err(error),
    }
}

fn import_failure() -> &'static str {
    course2md::tr!(
        "旧版设置导入未完成，原 config.toml 已保留。请检查配置和凭据存储，或在设置中重新配置服务。"
    )
}

fn add_service(
    store: &mut Store,
    protocol: ServiceProtocol,
    address: &str,
    model: &str,
    key: &str,
    created: &mut Vec<CredentialRef>,
) -> anyhow::Result<()> {
    let mut draft = ServiceDraft::new(protocol.purpose());
    draft.protocol = protocol;
    draft.address = address.to_owned();
    draft.model = model.to_owned();
    draft.authentication = if key.trim().is_empty() || protocol.keyless() {
        Authentication::None
    } else {
        Authentication::ApiKey
    };
    let secret = (draft.authentication == Authentication::ApiKey).then(|| Secret::new(key));
    let draft = store.save_service_draft(draft, secret)?;
    if let Some(reference) = &draft.credential {
        created.push(reference.clone());
    }
    store.publish_service(&draft.id, BindingScope::Defaults)?;
    Ok(())
}

/// Build a complete secret-free Store beside the destination, then publish the directory
/// atomically. An existing v2 Store, including a broken one, is never overwritten.
fn try_import(
    directory: &Path,
    config: &ConfigFile,
    vault: Arc<dyn CredentialVault>,
) -> anyhow::Result<bool> {
    let target = directory.join("desktop-preferences");
    if !absent(&target)? {
        return Ok(false);
    }
    std::fs::create_dir_all(directory)?;
    let stage = tempfile::Builder::new()
        .prefix(".desktop-legacy-import-")
        .tempdir_in(directory)?;
    let mut store = Store::open(stage.path(), Arc::clone(&vault));
    let mut created = Vec::new();
    let prepared = (|| -> anyhow::Result<()> {
        let mut options = config.defaults.clone();
        options.out = None;
        let generation = GenerationPreferences {
            options,
            ai_proofread: config.llm.enabled,
            ai_summary: config.llm.summarize,
            vision: config.llm.vision,
            prompt: config.llm.prompt.clone(),
            ai_concurrency: config.llm.concurrency,
            last_local_provider: config
                .defaults
                .provider
                .filter(|provider| *provider != course2md::config::AsrProvider::Api),
            ..Default::default()
        };
        store.save_generation(generation)?;
        store.save_application(ApplicationPreferences {
            desktop: config.desktop.clone(),
            ..Default::default()
        })?;
        if !config.llm.model.trim().is_empty() || config.llm.enabled || config.llm.summarize {
            let (protocol, address) = match config.llm.provider {
                course2md::llm::LlmProvider::OpenAiCompatible => {
                    (ServiceProtocol::AiChat, config.llm.base_url.as_str())
                }
                course2md::llm::LlmProvider::Ollama => {
                    (ServiceProtocol::OllamaChat, config.llm.base_url.as_str())
                }
                course2md::llm::LlmProvider::Codex => (
                    ServiceProtocol::CodexResponses,
                    crate::preferences::CODEX_ADDRESS,
                ),
            };
            add_service(
                &mut store,
                protocol,
                address,
                &config.llm.model,
                &config.llm.api_key,
                &mut created,
            )?;
        }
        // The legacy ASR struct has a populated default URL/model even if never configured.
        // Only a stored key establishes a user-configured online recognition service.
        if !config.asr_api.api_key.trim().is_empty() {
            let protocol = match config.asr_api.mode {
                course2md::settings::AsrApiMode::Transcriptions => {
                    ServiceProtocol::SpeechTranscriptions
                }
                course2md::settings::AsrApiMode::Chat => ServiceProtocol::SpeechChat,
            };
            add_service(
                &mut store,
                protocol,
                &config.asr_api.base_url,
                &config.asr_api.model,
                &config.asr_api.api_key,
                &mut created,
            )?;
        }
        Ok(())
    })();
    // Production Store writes asynchronously; a queued write is not migration success.
    // Flush even after failure so the staging directory cannot be recreated by a late writer.
    store.flush();
    let completed = store.process_writes();
    let saved = completed
        .into_iter()
        .try_for_each(|(_, result)| result.map_err(anyhow::Error::msg))
        .and_then(|()| {
            anyhow::ensure!(
                ![
                    PreferenceGroup::Generation,
                    PreferenceGroup::Application,
                    PreferenceGroup::Services
                ]
                .into_iter()
                .any(|group| store.is_pending(group)),
                import_failure()
            );
            Ok(())
        });
    store.flush();
    drop(store);
    let result = prepared.and(saved).and_then(|()| {
        // Recheck after preparation to protect a second application instance's first run.
        anyhow::ensure!(absent(&target)?, import_failure());
        std::fs::rename(stage.path(), &target)?;
        Ok(true)
    });
    if result.is_err() {
        for reference in created {
            let _ = vault.remove(&reference);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credentials::{CredentialVault, MemoryCredentialVault, Secret};
    use crate::preferences::ServicePurpose;
    use std::sync::Arc;

    fn fixture(root: &std::path::Path) -> course2md::settings::ConfigFile {
        let mut value = course2md::settings::ConfigFile::default();
        value.defaults.out = Some(root.join("old-library"));
        value.defaults.provider = Some(course2md::config::AsrProvider::Gpu);
        value.defaults.model_dir = Some(root.join("models"));
        value.llm.base_url = "https://example.test/v1".into();
        value.llm.model = "fixture-vision".into();
        value.llm.api_key = "fixture-secret-only-in-vault".into();
        value.llm.enabled = true;
        value.llm.summarize = true;
        value.llm.vision = true;
        value.llm.prompt = Some("fixture proofreading instruction".into());
        value.llm.concurrency = 3;
        value.desktop.setup_completed = true;
        value.desktop.reduce_motion = true;
        value
    }

    #[test]
    fn imports_defaults_and_service_without_copying_keys_into_preferences() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path();
        let source = root.join("config.toml");
        std::fs::write(&source, "original fixture, do not rewrite").unwrap();
        let config = fixture(root);
        let vault = Arc::new(MemoryCredentialVault::new());
        assert!(try_import(root, &config, vault.clone()).unwrap());
        let store =
            crate::preferences::Store::open(root.join("desktop-preferences"), vault.clone());
        let generation = store.generation();
        assert!(generation.options.out.is_none());
        assert_eq!(generation.options.model_dir, config.defaults.model_dir);
        assert_eq!(generation.options.provider, config.defaults.provider);
        assert!(generation.ai_proofread && generation.ai_summary && generation.vision);
        assert_eq!(generation.ai_concurrency, 3);
        assert_eq!(generation.prompt, config.llm.prompt);
        assert!(store.application().desktop.setup_completed);
        assert!(store.application().desktop.reduce_motion);
        let refs = store.default_refs();
        let service = store.version(refs.llm.as_ref().unwrap()).unwrap();
        assert_eq!(service.config.model, config.llm.model);
        assert_eq!(
            vault
                .resolve(service.config.credential.as_ref().unwrap())
                .unwrap()
                .expose(),
            config.llm.api_key
        );
        assert!(refs.asr.is_none());
        assert_eq!(
            std::fs::read_to_string(source).unwrap(),
            "original fixture, do not rewrite"
        );
        for name in [
            "generation.json",
            "application.json",
            "services.json",
            "services.json.bak",
        ] {
            if let Ok(bytes) = std::fs::read_to_string(root.join("desktop-preferences").join(name))
            {
                assert!(!bytes.contains(&config.llm.api_key));
            }
        }
    }

    #[test]
    fn existing_preferences_are_authoritative_and_not_reimported() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path();
        let preferences = root.join("desktop-preferences");
        std::fs::create_dir(&preferences).unwrap();
        std::fs::write(preferences.join("generation.json"), b"existing bytes").unwrap();
        let vault = Arc::new(MemoryCredentialVault::new());
        assert!(!try_import(root, &fixture(root), vault).unwrap());
        assert_eq!(
            std::fs::read(preferences.join("generation.json")).unwrap(),
            b"existing bytes"
        );
    }

    #[test]
    fn speech_service_is_imported_only_when_it_has_a_saved_key() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path();
        let mut config = fixture(root);
        config.asr_api.api_key = "fixture-speech-secret".into();
        config.asr_api.mode = course2md::settings::AsrApiMode::Chat;
        let vault = Arc::new(MemoryCredentialVault::new());
        assert!(try_import(root, &config, vault.clone()).unwrap());
        let store =
            crate::preferences::Store::open(root.join("desktop-preferences"), vault.clone());
        let refs = store.default_refs();
        let speech = store.version(refs.asr.as_ref().unwrap()).unwrap();
        assert_eq!(speech.config.protocol.purpose(), ServicePurpose::Speech);
        assert_eq!(
            speech.config.protocol,
            crate::preferences::ServiceProtocol::SpeechChat
        );
        assert_eq!(
            vault
                .resolve(speech.config.credential.as_ref().unwrap())
                .unwrap()
                .expose(),
            config.asr_api.api_key
        );
    }

    struct RefusingVault;
    impl CredentialVault for RefusingVault {
        fn insert(&self, _: Secret) -> anyhow::Result<String> {
            anyhow::bail!("fixture vault refused")
        }
        fn resolve(&self, _: &str) -> anyhow::Result<Secret> {
            anyhow::bail!("fixture missing")
        }
        fn remove(&self, _: &str) -> anyhow::Result<()> {
            Ok(())
        }
    }
    #[test]
    fn failed_import_does_not_create_an_authoritative_half_migrated_store() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path();
        std::fs::write(root.join("config.toml"), b"unchanged original").unwrap();
        assert!(try_import(root, &fixture(root), Arc::new(RefusingVault)).is_err());
        assert!(!root.join("desktop-preferences").exists());
        assert_eq!(
            std::fs::read(root.join("config.toml")).unwrap(),
            b"unchanged original"
        );
    }

    #[test]
    fn old_output_is_only_a_first_workspace_default_and_existing_workspace_is_untouched() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path();
        let config = fixture(root);
        assert_eq!(initial_library(root, &config), config.defaults.out);
        let workspace = root.join("desktop-workspace.json");
        std::fs::write(&workspace, b"existing workspace record").unwrap();
        assert!(initial_library(root, &config).is_none());
        assert!(try_import(root, &config, Arc::new(MemoryCredentialVault::new())).unwrap());
        assert_eq!(
            std::fs::read(workspace).unwrap(),
            b"existing workspace record"
        );
    }

    #[derive(Default)]
    struct TrackingVault {
        inner: MemoryCredentialVault,
        inserted: std::sync::Mutex<Vec<String>>,
    }
    impl CredentialVault for TrackingVault {
        fn insert(&self, secret: Secret) -> anyhow::Result<String> {
            let reference = self.inner.insert(secret)?;
            self.inserted.lock().unwrap().push(reference.clone());
            Ok(reference)
        }
        fn resolve(&self, reference: &str) -> anyhow::Result<Secret> {
            self.inner.resolve(reference)
        }
        fn remove(&self, reference: &str) -> anyhow::Result<()> {
            self.inner.remove(reference)
        }
    }
    #[test]
    fn later_service_failure_rolls_back_all_credentials_created_by_this_import() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path();
        let mut config = fixture(root);
        config.asr_api.api_key = "fixture-speech-secret".into();
        config.asr_api.model.clear();
        let vault = Arc::new(TrackingVault::default());
        assert!(try_import(root, &config, vault.clone()).is_err());
        assert!(!root.join("desktop-preferences").exists());
        let references = vault.inserted.lock().unwrap();
        assert_eq!(references.len(), 2);
        for reference in references.iter() {
            assert!(vault.resolve(reference).is_err());
        }
    }
}
