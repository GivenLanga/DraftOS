//! API key resolution. Keys live in the OS keychain (CLAUDE.md §9) — never in
//! SQLite or config files. Environment variables win when set, so headless and
//! CI environments work without a keychain daemon.

use crate::{ModelError, Result};

const KEYRING_SERVICE: &str = "draftos";

/// Conventional environment variable for an adapter's key.
pub(crate) fn env_var_for(adapter: &str) -> String {
    match adapter {
        "anthropic" => "ANTHROPIC_API_KEY".to_string(),
        "openai" => "OPENAI_API_KEY".to_string(),
        other => format!("DRAFTOS_{}_API_KEY", other.to_ascii_uppercase()),
    }
}

/// Resolve an adapter's API key: env var first, then the OS keychain.
/// `Ok(None)` means no key anywhere (fine for local endpoints).
pub(crate) fn resolve_key(adapter: &str) -> Result<Option<String>> {
    let var = env_var_for(adapter);
    if let Ok(v) = std::env::var(&var) {
        if !v.trim().is_empty() {
            return Ok(Some(v.trim().to_string()));
        }
    }
    match keyring::Entry::new(KEYRING_SERVICE, adapter) {
        Ok(entry) => match entry.get_password() {
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            // No keychain daemon (headless Linux) etc. — treat as "no key";
            // the env var path above still works.
            Err(e) => {
                tracing::debug!("keychain lookup failed for {adapter}: {e}");
                Ok(None)
            }
        },
        Err(e) => {
            tracing::debug!("keychain unavailable: {e}");
            Ok(None)
        }
    }
}

/// Require a key, with an actionable error naming the env var.
pub(crate) fn require_key(adapter: &str) -> Result<String> {
    resolve_key(adapter)?.ok_or_else(|| ModelError::MissingKey {
        adapter: adapter.to_string(),
        env_var: env_var_for(adapter),
    })
}

/// Store an API key in the OS keychain.
pub fn store_key(adapter: &str, key: &str) -> Result<()> {
    keyring::Entry::new(KEYRING_SERVICE, adapter)
        .and_then(|e| e.set_password(key))
        .map_err(|e| ModelError::Keychain(e.to_string()))
}

/// Remove an adapter's key from the OS keychain.
pub fn delete_key(adapter: &str) -> Result<()> {
    match keyring::Entry::new(KEYRING_SERVICE, adapter).and_then(|e| e.delete_password()) {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(ModelError::Keychain(e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_var_names() {
        assert_eq!(env_var_for("anthropic"), "ANTHROPIC_API_KEY");
        assert_eq!(env_var_for("openai"), "OPENAI_API_KEY");
        assert_eq!(env_var_for("ollama"), "DRAFTOS_OLLAMA_API_KEY");
    }
}
