use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Number of threads to generate.
    pub threads: u32,
    /// Number of accounts (1-4: gmail, imap, graph, jmap).
    pub accounts: u32,
    /// Locale mode: "mixed", "latin", or "intl".
    pub locale: String,
    /// RNG seed for deterministic generation.
    pub seed: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            threads: 500,
            accounts: 4,
            locale: "mixed".to_string(),
            seed: 42,
        }
    }
}

impl Config {
    /// Load config from `dev-seed.toml`, searching from `CARGO_MANIFEST_DIR`
    /// up to the workspace root. Falls back to defaults if not found.
    pub fn load_or_default() -> Self {
        // Try CARGO_MANIFEST_DIR first (set by cargo during `cargo run`)
        if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
            let path = Path::new(&manifest_dir);
            // Walk up to find dev-seed.toml (manifest_dir is the app crate,
            // dev-seed.toml is at workspace root)
            let mut dir = Some(path);
            while let Some(d) = dir {
                let candidate = d.join("dev-seed.toml");
                if candidate.exists() {
                    return Self::load_from(&candidate);
                }
                dir = d.parent();
            }
        }

        // Try current working directory
        let cwd_candidate = Path::new("dev-seed.toml");
        if cwd_candidate.exists() {
            return Self::load_from(cwd_candidate);
        }

        log::info!("dev-seed.toml not found, using defaults");
        Self::default()
    }

    fn load_from(path: &Path) -> Self {
        log::info!("Loading dev-seed config from {}", path.display());
        match std::fs::read_to_string(path) {
            Ok(contents) => match toml::from_str(&contents) {
                Ok(config) => config,
                Err(e) => {
                    log::warn!("Failed to parse {}: {e}, using defaults", path.display());
                    Self::default()
                }
            },
            Err(e) => {
                log::warn!("Failed to read {}: {e}, using defaults", path.display());
                Self::default()
            }
        }
    }
}
