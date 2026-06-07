//! Server-side skill registry — the authoring store behind the `/skills`
//! endpoints (and thus the CLI `thymos skill` + desktop Skills tab). It only
//! resolves and persists skill *definitions*; the authority-narrowing and the
//! replay-verified `skill_bound` ledger entry happen in the run pipeline
//! (`create_run`). File-backed in production, pure in-memory for tests.
//!
//! See `docs/rfcs/skills.md`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use thymos_core::skill::SkillDef;

/// A small name→definition registry. The cache is the source of truth at
/// runtime; when `dir` is set, every save is also written as `<name>.json` so it
/// survives a restart.
pub struct SkillRegistry {
    dir: Option<PathBuf>,
    cache: Mutex<HashMap<String, SkillDef>>,
}

impl SkillRegistry {
    /// Build a registry. `Some(dir)` persists to (and loads from) that directory;
    /// `None` is in-memory only (tests).
    pub fn new(dir: Option<PathBuf>) -> Self {
        let reg = SkillRegistry {
            dir,
            cache: Mutex::new(HashMap::new()),
        };
        reg.load_dir();
        reg
    }

    fn load_dir(&self) {
        let Some(dir) = &self.dir else { return };
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        let mut cache = self.cache.lock().unwrap();
        for entry in rd.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if let Ok(text) = std::fs::read_to_string(&path) {
                if let Ok(def) = serde_json::from_str::<SkillDef>(&text) {
                    cache.insert(def.name.clone(), def);
                }
            }
        }
    }

    /// All skills, sorted by name.
    pub fn list(&self) -> Vec<SkillDef> {
        let mut v: Vec<SkillDef> = self.cache.lock().unwrap().values().cloned().collect();
        v.sort_by(|a, b| a.name.cmp(&b.name));
        v
    }

    pub fn get_by_name(&self, name: &str) -> Option<SkillDef> {
        self.cache.lock().unwrap().get(name).cloned()
    }

    pub fn get_by_id(&self, id: &str) -> Option<SkillDef> {
        let want = id.trim();
        self.cache
            .lock()
            .unwrap()
            .values()
            .find(|d| {
                let sid = d.id();
                sid.to_string() == want || sid.0.to_string() == want
            })
            .cloned()
    }

    /// Resolve a `name` or content-id reference. Names take precedence.
    pub fn resolve(&self, name_or_id: &str) -> Option<SkillDef> {
        self.get_by_name(name_or_id)
            .or_else(|| self.get_by_id(name_or_id))
    }

    /// Create or tune a skill. If one with the same name exists and the content
    /// differs, `version` is bumped to existing+1 (so each edit mints a fresh,
    /// content-addressed id). Returns the stored definition.
    pub fn save(&self, mut skill: SkillDef) -> Result<SkillDef, String> {
        if skill.name.trim().is_empty() {
            return Err("skill name is required".into());
        }
        {
            let cache = self.cache.lock().unwrap();
            if let Some(existing) = cache.get(&skill.name) {
                if existing.id() != skill.id() {
                    skill.version = existing.version.saturating_add(1);
                }
            }
        }
        if let Some(dir) = &self.dir {
            std::fs::create_dir_all(dir).map_err(|e| format!("create skills dir: {e}"))?;
            let path = dir.join(format!("{}.json", sanitize(&skill.name)));
            let json = serde_json::to_string_pretty(&skill).map_err(|e| e.to_string())?;
            std::fs::write(&path, json).map_err(|e| format!("write {}: {e}", path.display()))?;
        }
        self.cache
            .lock()
            .unwrap()
            .insert(skill.name.clone(), skill.clone());
        Ok(skill)
    }
}

/// Restrict a skill name to a safe filename stem.
fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use thymos_core::writ::EffectCeiling;

    fn skill(name: &str) -> SkillDef {
        SkillDef {
            name: name.into(),
            version: 1,
            title: String::new(),
            instructions: "do x".into(),
            tools: vec![],
            ceiling: EffectCeiling::read_write_local(),
            budget_cap: None,
            params: vec![],
            model_hint: Default::default(),
        }
    }

    #[test]
    fn in_memory_save_resolve_and_tune_bumps_version() {
        let reg = SkillRegistry::new(None);
        let saved = reg.save(skill("triage")).unwrap();
        assert_eq!(saved.version, 1);
        // Resolve by name and by id.
        assert!(reg.get_by_name("triage").is_some());
        let id = saved.id().to_string();
        assert!(reg.resolve(&id).is_some());
        // Tuning the content bumps the version.
        let mut tuned = skill("triage");
        tuned.instructions = "do y".into();
        let saved2 = reg.save(tuned).unwrap();
        assert_eq!(saved2.version, 2, "changed content bumps version");
        assert_eq!(reg.list().len(), 1, "same name replaces, not appends");
    }
}
