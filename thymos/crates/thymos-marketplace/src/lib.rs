//! Tool marketplace: publish, discover, and install manifest tools and MCP servers.
//!
//! The marketplace is an in-memory registry (Phase 1) that holds tool packages.
//! Each package describes either a **manifest tool** (JSON schema + executor) or
//! an **MCP server** (command + args) that can be installed into a ToolRegistry.
//!
//! Packages are versioned with semver and content-hashed for integrity.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;
use thiserror::Error;

// ── Error ────────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum MarketplaceError {
    #[error("package not found: {0}")]
    NotFound(String),
    #[error("version conflict: {name}@{version} already exists")]
    VersionConflict { name: String, version: String },
    #[error("integrity mismatch: expected {expected}, got {actual}")]
    IntegrityMismatch { expected: String, actual: String },
    #[error("signature missing — package is unsigned")]
    SignatureMissing,
    #[error("signature invalid: {0}")]
    SignatureInvalid(String),
    #[error("untrusted publisher: {0}")]
    UntrustedPublisher(String),
    #[error("invalid manifest: {0}")]
    InvalidManifest(String),
}

// ── Package schema ───────────────────────────────────────────────────────────

/// How the tool is executed.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PackageKind {
    /// A manifest tool: inline JSON schema + executor config.
    Manifest {
        /// The tool manifest JSON (same format as thymos-tools ToolManifest).
        manifest: serde_json::Value,
    },
    /// An MCP server: spawn a subprocess, discover tools via JSON-RPC.
    McpServer {
        /// Command to run (e.g. "uvx", "npx", "node").
        command: String,
        /// Arguments (e.g. ["my-mcp-server", "--port", "0"]).
        args: Vec<String>,
        /// Environment variables to set.
        #[serde(default)]
        env: HashMap<String, String>,
    },
}

/// A published tool package in the marketplace.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Package {
    /// Unique package name (e.g. "thymos/kv-tools", "community/weather").
    pub name: String,
    /// Semver version string.
    pub version: String,
    /// Human-readable description.
    pub description: String,
    /// Author or org.
    pub author: String,
    /// Tags for search/discovery.
    #[serde(default)]
    pub tags: Vec<String>,
    /// The tool kind and its configuration.
    pub kind: PackageKind,
    /// BLAKE3 hash of the canonical JSON representation (integrity check).
    #[serde(default)]
    pub content_hash: String,
    /// ISO-8601 timestamp of publication.
    #[serde(default)]
    pub published_at: String,
    /// Ed25519 signature over `content_hash` bytes, hex-encoded (128 chars).
    /// Empty string means unsigned.
    #[serde(default)]
    pub signature: String,
    /// Hex-encoded ed25519 public key of the publisher (64 chars).
    /// Empty string means unsigned.
    #[serde(default)]
    pub publisher_pubkey: String,
}

impl Package {
    /// Compute the content hash from the package's kind payload.
    pub fn compute_hash(&self) -> String {
        let payload = serde_json::to_vec(&self.kind).unwrap_or_default();
        let hash = blake3::hash(&payload);
        hex::encode(hash.as_bytes())
    }

    /// Verify the content hash matches the payload.
    pub fn verify_integrity(&self) -> Result<(), MarketplaceError> {
        if self.content_hash.is_empty() {
            return Ok(()); // No hash set, skip verification.
        }
        let actual = self.compute_hash();
        if actual != self.content_hash {
            return Err(MarketplaceError::IntegrityMismatch {
                expected: self.content_hash.clone(),
                actual,
            });
        }
        Ok(())
    }

    /// Sign this package with an ed25519 signing key. Overwrites any previous
    /// signature. Fills in `content_hash` if empty.
    pub fn sign(
        &mut self,
        signing_key: &thymos_core::crypto::SigningKey,
    ) -> Result<(), MarketplaceError> {
        if self.content_hash.is_empty() {
            self.content_hash = self.compute_hash();
        }
        let pubkey = thymos_core::crypto::public_key_of(signing_key);
        let message = self.content_hash.as_bytes();
        let sig = thymos_core::crypto::sign(signing_key, message);
        self.signature = hex::encode(sig);
        self.publisher_pubkey = hex::encode(pubkey);
        Ok(())
    }

    /// Verify the ed25519 signature. Also re-checks the content hash.
    /// Returns `SignatureMissing` if the package is unsigned.
    pub fn verify_signature(&self) -> Result<(), MarketplaceError> {
        self.verify_integrity()?;
        if self.signature.is_empty() || self.publisher_pubkey.is_empty() {
            return Err(MarketplaceError::SignatureMissing);
        }
        let sig_bytes: [u8; 64] = hex::decode(&self.signature)
            .map_err(|e| MarketplaceError::SignatureInvalid(format!("sig hex: {e}")))?
            .try_into()
            .map_err(|_| MarketplaceError::SignatureInvalid("sig must be 64 bytes".into()))?;
        let pk_bytes: [u8; 32] = hex::decode(&self.publisher_pubkey)
            .map_err(|e| MarketplaceError::SignatureInvalid(format!("pubkey hex: {e}")))?
            .try_into()
            .map_err(|_| MarketplaceError::SignatureInvalid("pubkey must be 32 bytes".into()))?;
        let message = self.content_hash.as_bytes();
        thymos_core::crypto::verify(&pk_bytes, message, &sig_bytes)
            .map_err(|e| MarketplaceError::SignatureInvalid(e.to_string()))
    }
}

/// A trust policy for package installation. Holds the set of publisher
/// public keys (hex-encoded) that are allowed to install into a runtime.
#[derive(Clone, Debug, Default)]
pub struct TrustedPublishers {
    keys: std::collections::HashSet<String>,
}

impl TrustedPublishers {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_keys(keys: impl IntoIterator<Item = String>) -> Self {
        Self {
            keys: keys.into_iter().collect(),
        }
    }

    pub fn trust(&mut self, pubkey_hex: impl Into<String>) {
        self.keys.insert(pubkey_hex.into());
    }

    pub fn contains(&self, pubkey_hex: &str) -> bool {
        self.keys.contains(pubkey_hex)
    }

    /// Require that the package is signed by a trusted publisher.
    pub fn enforce(&self, pkg: &Package) -> Result<(), MarketplaceError> {
        pkg.verify_signature()?;
        if !self.contains(&pkg.publisher_pubkey) {
            return Err(MarketplaceError::UntrustedPublisher(
                pkg.publisher_pubkey.clone(),
            ));
        }
        Ok(())
    }
}

// ── Search ───────────────────────────────────────────────────────────────────

/// Search query for the marketplace.
#[derive(Clone, Debug, Default)]
pub struct SearchQuery {
    /// Substring match on name or description.
    pub text: Option<String>,
    /// Filter by tags (all must match).
    pub tags: Vec<String>,
    /// Filter by author.
    pub author: Option<String>,
    /// Filter by kind ("manifest" or "mcp_server").
    pub kind: Option<String>,
}

// ── Registry ─────────────────────────────────────────────────────────────────

/// In-memory tool marketplace registry.
#[derive(Default)]
pub struct Marketplace {
    /// name -> (version -> Package). Supports multiple versions per package.
    packages: HashMap<String, HashMap<String, Package>>,
}

/// SQLite-backed package store for persistent marketplace state.
pub struct SqliteMarketplaceStore {
    conn: Mutex<rusqlite::Connection>,
}

impl SqliteMarketplaceStore {
    pub fn open(path: &str) -> Result<Self, MarketplaceError> {
        let conn = rusqlite::Connection::open(path)
            .map_err(|e| MarketplaceError::InvalidManifest(e.to_string()))?;
        Self::bootstrap(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn open_in_memory() -> Result<Self, MarketplaceError> {
        let conn = rusqlite::Connection::open_in_memory()
            .map_err(|e| MarketplaceError::InvalidManifest(e.to_string()))?;
        Self::bootstrap(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn bootstrap(conn: &rusqlite::Connection) -> Result<(), MarketplaceError> {
        conn.execute_batch(
            r#"
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = NORMAL;

            CREATE TABLE IF NOT EXISTS marketplace_packages (
                name              TEXT NOT NULL,
                version           TEXT NOT NULL,
                description       TEXT NOT NULL,
                author            TEXT NOT NULL,
                tags_json         TEXT NOT NULL,
                kind_json         TEXT NOT NULL,
                content_hash      TEXT NOT NULL,
                published_at      TEXT NOT NULL DEFAULT '',
                signature         TEXT NOT NULL DEFAULT '',
                publisher_pubkey  TEXT NOT NULL DEFAULT '',
                created_at        INTEGER NOT NULL DEFAULT (unixepoch()),
                PRIMARY KEY (name, version)
            );

            CREATE INDEX IF NOT EXISTS idx_marketplace_packages_name
                ON marketplace_packages(name);
            "#,
        )
        .map_err(|e| MarketplaceError::InvalidManifest(e.to_string()))?;

        // Best-effort migration for pre-existing databases.
        let _ = conn.execute(
            "ALTER TABLE marketplace_packages ADD COLUMN signature TEXT NOT NULL DEFAULT ''",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE marketplace_packages ADD COLUMN publisher_pubkey TEXT NOT NULL DEFAULT ''",
            [],
        );
        Ok(())
    }

    pub fn load_all(&self) -> Result<Vec<Package>, MarketplaceError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT name, version, description, author, tags_json, kind_json, content_hash, published_at, signature, publisher_pubkey
                 FROM marketplace_packages
                 ORDER BY name ASC, version ASC",
            )
            .map_err(|e| MarketplaceError::InvalidManifest(e.to_string()))?;

        let rows = stmt
            .query_map([], |row| {
                let tags_json: String = row.get(4)?;
                let kind_json: String = row.get(5)?;
                let tags: Vec<String> = serde_json::from_str(&tags_json).map_err(|err| {
                    rusqlite::Error::FromSqlConversionFailure(
                        tags_json.len(),
                        rusqlite::types::Type::Text,
                        Box::new(err),
                    )
                })?;
                let kind: PackageKind = serde_json::from_str(&kind_json).map_err(|err| {
                    rusqlite::Error::FromSqlConversionFailure(
                        kind_json.len(),
                        rusqlite::types::Type::Text,
                        Box::new(err),
                    )
                })?;

                Ok(Package {
                    name: row.get(0)?,
                    version: row.get(1)?,
                    description: row.get(2)?,
                    author: row.get(3)?,
                    tags,
                    kind,
                    content_hash: row.get(6)?,
                    published_at: row.get(7)?,
                    signature: row.get(8)?,
                    publisher_pubkey: row.get(9)?,
                })
            })
            .map_err(|e| MarketplaceError::InvalidManifest(e.to_string()))?;

        let mut packages = Vec::new();
        for row in rows {
            packages.push(row.map_err(|e| MarketplaceError::InvalidManifest(e.to_string()))?);
        }
        Ok(packages)
    }

    pub fn publish(&self, pkg: &Package) -> Result<(), MarketplaceError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO marketplace_packages
                (name, version, description, author, tags_json, kind_json, content_hash, published_at, signature, publisher_pubkey)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            rusqlite::params![
                pkg.name,
                pkg.version,
                pkg.description,
                pkg.author,
                serde_json::to_string(&pkg.tags)
                    .map_err(|e| MarketplaceError::InvalidManifest(e.to_string()))?,
                serde_json::to_string(&pkg.kind)
                    .map_err(|e| MarketplaceError::InvalidManifest(e.to_string()))?,
                pkg.content_hash,
                pkg.published_at,
                pkg.signature,
                pkg.publisher_pubkey,
            ],
        )
        .map_err(|e| {
            if matches!(e, rusqlite::Error::SqliteFailure(_, Some(_))) {
                MarketplaceError::VersionConflict {
                    name: pkg.name.clone(),
                    version: pkg.version.clone(),
                }
            } else {
                MarketplaceError::InvalidManifest(e.to_string())
            }
        })?;
        Ok(())
    }

    pub fn unpublish(&self, name: &str, version: &str) -> Result<(), MarketplaceError> {
        let conn = self.conn.lock().unwrap();
        let affected = conn
            .execute(
                "DELETE FROM marketplace_packages WHERE name = ?1 AND version = ?2",
                rusqlite::params![name, version],
            )
            .map_err(|e| MarketplaceError::InvalidManifest(e.to_string()))?;
        if affected == 0 {
            return Err(MarketplaceError::NotFound(format!("{name}@{version}")));
        }
        Ok(())
    }
}

/// Marketplace facade that keeps an in-memory index while persisting package
/// operations to an optional SQLite store.
pub struct MarketplaceService {
    registry: Mutex<Marketplace>,
    store: Option<SqliteMarketplaceStore>,
}

impl MarketplaceService {
    pub fn in_memory() -> Self {
        Self {
            registry: Mutex::new(Marketplace::new()),
            store: None,
        }
    }

    pub fn with_store(store: SqliteMarketplaceStore) -> Result<Self, MarketplaceError> {
        let mut registry = Marketplace::new();
        for pkg in store.load_all()? {
            registry.publish(pkg)?;
        }
        Ok(Self {
            registry: Mutex::new(registry),
            store: Some(store),
        })
    }

    pub fn open_sqlite(path: &str) -> Result<Self, MarketplaceError> {
        Self::with_store(SqliteMarketplaceStore::open(path)?)
    }

    pub fn list(&self) -> Vec<Package> {
        let registry = self.registry.lock().unwrap();
        registry.list().into_iter().cloned().collect()
    }

    pub fn total_packages(&self) -> usize {
        self.registry.lock().unwrap().total_packages()
    }

    pub fn get(&self, name: &str, version: Option<&str>) -> Result<Package, MarketplaceError> {
        self.registry.lock().unwrap().get(name, version).cloned()
    }

    pub fn search(&self, query: &SearchQuery) -> Vec<Package> {
        let registry = self.registry.lock().unwrap();
        registry.search(query).into_iter().cloned().collect()
    }

    pub fn publish(&self, pkg: Package) -> Result<(), MarketplaceError> {
        let mut registry = self.registry.lock().unwrap();
        registry.publish(pkg.clone())?;
        if let Some(store) = &self.store {
            if let Err(err) = store.publish(&pkg) {
                let _ = registry.unpublish(&pkg.name, &pkg.version);
                return Err(err);
            }
        }
        Ok(())
    }

    pub fn unpublish(&self, name: &str, version: &str) -> Result<Package, MarketplaceError> {
        let mut registry = self.registry.lock().unwrap();
        let pkg = registry.unpublish(name, version)?;
        if let Some(store) = &self.store {
            store.unpublish(name, version)?;
        }
        Ok(pkg)
    }
}

impl Marketplace {
    pub fn new() -> Self {
        Self::default()
    }

    /// Publish a package to the marketplace.
    pub fn publish(&mut self, mut pkg: Package) -> Result<(), MarketplaceError> {
        // Compute content hash if not set.
        if pkg.content_hash.is_empty() {
            pkg.content_hash = pkg.compute_hash();
        } else {
            pkg.verify_integrity()?;
        }

        let versions = self.packages.entry(pkg.name.clone()).or_default();
        if versions.contains_key(&pkg.version) {
            return Err(MarketplaceError::VersionConflict {
                name: pkg.name.clone(),
                version: pkg.version.clone(),
            });
        }
        versions.insert(pkg.version.clone(), pkg);
        Ok(())
    }

    /// Get a specific package version. If version is None, returns the latest.
    pub fn get(&self, name: &str, version: Option<&str>) -> Result<&Package, MarketplaceError> {
        let versions = self
            .packages
            .get(name)
            .ok_or_else(|| MarketplaceError::NotFound(name.to_string()))?;

        match version {
            Some(v) => versions
                .get(v)
                .ok_or_else(|| MarketplaceError::NotFound(format!("{name}@{v}"))),
            None => {
                // Return the "latest" — simple lexicographic max for now.
                versions
                    .values()
                    .max_by(|a, b| a.version.cmp(&b.version))
                    .ok_or_else(|| MarketplaceError::NotFound(name.to_string()))
            }
        }
    }

    /// List all packages (latest version of each).
    pub fn list(&self) -> Vec<&Package> {
        self.packages
            .values()
            .filter_map(|versions| versions.values().max_by(|a, b| a.version.cmp(&b.version)))
            .collect()
    }

    /// Search packages by query.
    pub fn search(&self, query: &SearchQuery) -> Vec<&Package> {
        self.list()
            .into_iter()
            .filter(|pkg| {
                // Text filter.
                if let Some(text) = &query.text {
                    let t = text.to_lowercase();
                    if !pkg.name.to_lowercase().contains(&t)
                        && !pkg.description.to_lowercase().contains(&t)
                    {
                        return false;
                    }
                }
                // Tags filter (all must match).
                for tag in &query.tags {
                    if !pkg.tags.contains(tag) {
                        return false;
                    }
                }
                // Author filter.
                if let Some(author) = &query.author {
                    if pkg.author != *author {
                        return false;
                    }
                }
                // Kind filter.
                if let Some(kind) = &query.kind {
                    let matches = matches!(
                        (&pkg.kind, kind.as_str()),
                        (PackageKind::Manifest { .. }, "manifest")
                            | (PackageKind::McpServer { .. }, "mcp_server")
                    );
                    if !matches {
                        return false;
                    }
                }
                true
            })
            .collect()
    }

    /// Remove a specific version. Returns the removed package if found.
    pub fn unpublish(&mut self, name: &str, version: &str) -> Result<Package, MarketplaceError> {
        let versions = self
            .packages
            .get_mut(name)
            .ok_or_else(|| MarketplaceError::NotFound(name.to_string()))?;

        let pkg = versions
            .remove(version)
            .ok_or_else(|| MarketplaceError::NotFound(format!("{name}@{version}")))?;

        // Clean up empty version maps.
        if versions.is_empty() {
            self.packages.remove(name);
        }

        Ok(pkg)
    }

    /// Total number of packages (counting each version separately).
    pub fn total_versions(&self) -> usize {
        self.packages.values().map(|v| v.len()).sum()
    }

    /// Number of unique package names.
    pub fn total_packages(&self) -> usize {
        self.packages.len()
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_manifest_pkg(name: &str, version: &str) -> Package {
        Package {
            name: name.into(),
            version: version.into(),
            description: "A sample manifest tool".into(),
            author: "thymos".into(),
            tags: vec!["kv".into(), "storage".into()],
            kind: PackageKind::Manifest {
                manifest: serde_json::json!({
                    "name": "kv_set",
                    "description": "Set a key-value pair",
                    "parameters": { "key": "string", "value": "string" },
                    "executor": { "type": "noop" }
                }),
            },
            content_hash: String::new(),
            published_at: String::new(),
            signature: String::new(),
            publisher_pubkey: String::new(),
        }
    }

    #[test]
    fn sqlite_store_roundtrip() {
        let store = SqliteMarketplaceStore::open_in_memory().unwrap();
        let pkg = sample_manifest_pkg("thymos/test", "1.0.0");
        store.publish(&pkg).unwrap();

        let loaded = store.load_all().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, "thymos/test");
        assert_eq!(loaded[0].version, "1.0.0");
    }

    #[test]
    fn marketplace_service_rehydrates_from_store() {
        let store = SqliteMarketplaceStore::open_in_memory().unwrap();
        store
            .publish(&sample_manifest_pkg("thymos/test", "1.0.0"))
            .unwrap();

        let service = MarketplaceService::with_store(store).unwrap();
        let pkg = service.get("thymos/test", None).unwrap();
        assert_eq!(pkg.version, "1.0.0");
    }

    fn sample_mcp_pkg(name: &str, version: &str) -> Package {
        Package {
            name: name.into(),
            version: version.into(),
            description: "An MCP weather server".into(),
            author: "community".into(),
            tags: vec!["weather".into(), "mcp".into()],
            kind: PackageKind::McpServer {
                command: "uvx".into(),
                args: vec!["weather-server".into()],
                env: HashMap::new(),
            },
            content_hash: String::new(),
            published_at: String::new(),
            signature: String::new(),
            publisher_pubkey: String::new(),
        }
    }

    #[test]
    fn publish_and_get() {
        let mut mp = Marketplace::new();
        mp.publish(sample_manifest_pkg("thymos/kv", "0.1.0"))
            .unwrap();

        let pkg = mp.get("thymos/kv", Some("0.1.0")).unwrap();
        assert_eq!(pkg.name, "thymos/kv");
        assert!(!pkg.content_hash.is_empty());
    }

    #[test]
    fn get_latest_version() {
        let mut mp = Marketplace::new();
        mp.publish(sample_manifest_pkg("thymos/kv", "0.1.0"))
            .unwrap();
        mp.publish(sample_manifest_pkg("thymos/kv", "0.2.0"))
            .unwrap();

        let pkg = mp.get("thymos/kv", None).unwrap();
        assert_eq!(pkg.version, "0.2.0");
    }

    #[test]
    fn version_conflict() {
        let mut mp = Marketplace::new();
        mp.publish(sample_manifest_pkg("thymos/kv", "0.1.0"))
            .unwrap();
        let err = mp
            .publish(sample_manifest_pkg("thymos/kv", "0.1.0"))
            .unwrap_err();
        assert!(matches!(err, MarketplaceError::VersionConflict { .. }));
    }

    #[test]
    fn search_by_text() {
        let mut mp = Marketplace::new();
        mp.publish(sample_manifest_pkg("thymos/kv", "0.1.0"))
            .unwrap();
        mp.publish(sample_mcp_pkg("community/weather", "1.0.0"))
            .unwrap();

        let results = mp.search(&SearchQuery {
            text: Some("weather".into()),
            ..Default::default()
        });
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "community/weather");
    }

    #[test]
    fn search_by_tag() {
        let mut mp = Marketplace::new();
        mp.publish(sample_manifest_pkg("thymos/kv", "0.1.0"))
            .unwrap();
        mp.publish(sample_mcp_pkg("community/weather", "1.0.0"))
            .unwrap();

        let results = mp.search(&SearchQuery {
            tags: vec!["mcp".into()],
            ..Default::default()
        });
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "community/weather");
    }

    #[test]
    fn search_by_kind() {
        let mut mp = Marketplace::new();
        mp.publish(sample_manifest_pkg("thymos/kv", "0.1.0"))
            .unwrap();
        mp.publish(sample_mcp_pkg("community/weather", "1.0.0"))
            .unwrap();

        let manifest_results = mp.search(&SearchQuery {
            kind: Some("manifest".into()),
            ..Default::default()
        });
        assert_eq!(manifest_results.len(), 1);

        let mcp_results = mp.search(&SearchQuery {
            kind: Some("mcp_server".into()),
            ..Default::default()
        });
        assert_eq!(mcp_results.len(), 1);
    }

    #[test]
    fn unpublish() {
        let mut mp = Marketplace::new();
        mp.publish(sample_manifest_pkg("thymos/kv", "0.1.0"))
            .unwrap();
        mp.publish(sample_manifest_pkg("thymos/kv", "0.2.0"))
            .unwrap();

        let removed = mp.unpublish("thymos/kv", "0.1.0").unwrap();
        assert_eq!(removed.version, "0.1.0");
        assert_eq!(mp.total_versions(), 1);

        // Latest is still 0.2.0.
        let pkg = mp.get("thymos/kv", None).unwrap();
        assert_eq!(pkg.version, "0.2.0");
    }

    #[test]
    fn integrity_check() {
        let pkg = sample_manifest_pkg("thymos/kv", "0.1.0");
        let hash = pkg.compute_hash();
        assert!(!hash.is_empty());

        // Correct hash passes.
        let mut pkg2 = pkg.clone();
        pkg2.content_hash = hash;
        pkg2.verify_integrity().unwrap();

        // Wrong hash fails.
        let mut pkg3 = pkg;
        pkg3.content_hash = "deadbeef".into();
        assert!(matches!(
            pkg3.verify_integrity(),
            Err(MarketplaceError::IntegrityMismatch { .. })
        ));
    }

    #[test]
    fn list_shows_latest_only() {
        let mut mp = Marketplace::new();
        mp.publish(sample_manifest_pkg("thymos/kv", "0.1.0"))
            .unwrap();
        mp.publish(sample_manifest_pkg("thymos/kv", "0.2.0"))
            .unwrap();
        mp.publish(sample_mcp_pkg("community/weather", "1.0.0"))
            .unwrap();

        let all = mp.list();
        assert_eq!(all.len(), 2);
        assert_eq!(mp.total_versions(), 3);
        assert_eq!(mp.total_packages(), 2);
    }

    #[test]
    fn sign_and_verify() {
        let mut pkg = sample_manifest_pkg("thymos/kv", "0.1.0");
        let sk = thymos_core::crypto::generate_signing_key();
        pkg.sign(&sk).unwrap();

        assert_eq!(pkg.signature.len(), 128);
        assert_eq!(pkg.publisher_pubkey.len(), 64);
        pkg.verify_signature().unwrap();

        // Tampering with the payload breaks the signature.
        let mut bad = pkg.clone();
        bad.content_hash = "deadbeef".into();
        let err = bad.verify_signature().unwrap_err();
        assert!(matches!(err, MarketplaceError::IntegrityMismatch { .. }));
    }

    #[test]
    fn trusted_publisher_gate() {
        let mut pkg = sample_manifest_pkg("thymos/kv", "0.1.0");
        let sk = thymos_core::crypto::generate_signing_key();
        pkg.sign(&sk).unwrap();

        let mut trust = TrustedPublishers::new();
        let err = trust.enforce(&pkg).unwrap_err();
        assert!(matches!(err, MarketplaceError::UntrustedPublisher(_)));

        trust.trust(pkg.publisher_pubkey.clone());
        trust.enforce(&pkg).unwrap();
    }

    #[test]
    fn signed_package_survives_sqlite_roundtrip() {
        let store = SqliteMarketplaceStore::open_in_memory().unwrap();
        let mut pkg = sample_manifest_pkg("thymos/signed", "1.0.0");
        let sk = thymos_core::crypto::generate_signing_key();
        pkg.sign(&sk).unwrap();
        store.publish(&pkg).unwrap();

        let loaded = store.load_all().unwrap();
        assert_eq!(loaded.len(), 1);
        loaded[0].verify_signature().unwrap();
        assert_eq!(loaded[0].publisher_pubkey, pkg.publisher_pubkey);
    }

    #[test]
    fn serde_roundtrip() {
        let pkg = sample_mcp_pkg("community/weather", "1.0.0");
        let json = serde_json::to_string(&pkg).unwrap();
        let restored: Package = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.name, pkg.name);
        assert_eq!(restored.version, pkg.version);
        match &restored.kind {
            PackageKind::McpServer { command, args, .. } => {
                assert_eq!(command, "uvx");
                assert_eq!(args, &["weather-server"]);
            }
            _ => panic!("expected McpServer"),
        }
    }
}
