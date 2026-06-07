//! Skill — a content-addressed, authority-**narrowing** capability template.
//!
//! A skill never grants authority. It packages *how* to do a recurring task:
//! instructions for cognition, an allow-list of tools, and caps on the effect
//! ceiling and budget. When a run binds a skill, the effective authority is the
//! **intersection** of the caller's writ and the skill — enforced as additional
//! constraints (logical AND / field-wise min) at authorization time, so a skill
//! can only ever *shrink* what the writ already permits. It is never used to
//! mint or re-sign a writ.
//!
//! The skill identity is `blake3(canonical_json(SkillDef))`; "tuning" any field
//! yields a new [`SkillId`] (and should bump `version`), so a bound skill is an
//! immutable, replay-stable reference. See `docs/rfcs/skills.md`.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::hash::{content_hash, ContentHash};
use crate::writ::{Budget, EffectCeiling, ToolPattern};

/// Content-addressed skill identity = `blake3(canonical_json(SkillDef))`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SkillId(pub ContentHash);

impl SkillId {
    pub const ZERO: Self = SkillId(ContentHash::ZERO);
    pub fn inner(&self) -> &ContentHash {
        &self.0
    }
}

impl fmt::Debug for SkillId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "skill({})", self.0.short())
    }
}

impl fmt::Display for SkillId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "skill:{}", self.0)
    }
}

/// A tunable knob exposed by a skill, interpolated into `instructions` as
/// `{key}`. `values` non-empty constrains it to an enum; empty = free string.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillParam {
    pub key: String,
    #[serde(default)]
    pub description: String,
    pub default: String,
    #[serde(default)]
    pub values: Vec<String>,
}

impl SkillParam {
    /// Validate a supplied value: enum params must be one of `values`.
    pub fn accepts(&self, value: &str) -> bool {
        self.values.is_empty() || self.values.iter().any(|v| v == value)
    }
}

/// Advisory provider/model preference. **Never authoritative** — it is overridden
/// by the request and operator config, and confers no execution authority.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelHint {
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
}

/// The content-addressed skill definition. Editing any field ("tuning") changes
/// the [`SkillId`]; bump `version` alongside.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SkillDef {
    /// Human handle, resolved through a registry to the current id.
    pub name: String,
    pub version: u32,
    #[serde(default)]
    pub title: String,
    /// Prompt fragment prepended to the task. `{param}` placeholders are filled
    /// from the bound params (see [`SkillDef::render_instructions`]).
    pub instructions: String,
    /// Tool allow-list. **Empty = no additional tool restriction** (the writ
    /// still gates every call); non-empty = the skill permits only these.
    #[serde(default)]
    pub tools: Vec<ToolPattern>,
    /// Cap on the effect ceiling: the effective ceiling is the field-wise AND
    /// with the writ's. A `false` here removes that effect even if the writ
    /// allows it; a `true` can never *add* one the writ forbids.
    pub ceiling: EffectCeiling,
    /// Optional field-wise cap on the budget; the effective budget is the
    /// field-wise min with the writ's. `None` = the skill imposes no budget cap.
    #[serde(default)]
    pub budget_cap: Option<Budget>,
    #[serde(default)]
    pub params: Vec<SkillParam>,
    #[serde(default)]
    pub model_hint: ModelHint,
}

impl SkillDef {
    /// Content-addressed identity. Canonical JSON is deterministic, so this is
    /// stable across serialization boundaries (spec Section 7).
    pub fn id(&self) -> SkillId {
        // canonical_json of a plain serializable struct cannot fail; fall back
        // to ZERO defensively rather than panicking in a hashing helper.
        SkillId(content_hash(self).unwrap_or(ContentHash::ZERO))
    }

    /// True iff this skill's allow-list permits `tool`. An empty allow-list
    /// imposes no restriction (the writ alone decides).
    pub fn allows_tool(&self, tool: &str) -> bool {
        self.tools.is_empty() || self.tools.iter().any(|p| p.matches(tool))
    }

    /// Effective effect ceiling = field-wise AND with the writ's ceiling. The
    /// result is always `⊆` the input (AND can only clear bits), never wider.
    pub fn cap_ceiling(&self, writ: &EffectCeiling) -> EffectCeiling {
        EffectCeiling {
            read: writ.read && self.ceiling.read,
            write: writ.write && self.ceiling.write,
            external: writ.external && self.ceiling.external,
            irreversible: writ.irreversible && self.ceiling.irreversible,
        }
    }

    /// Effective budget = field-wise min with the writ's budget (or the writ's
    /// budget unchanged when the skill sets no cap). Never exceeds the writ.
    pub fn cap_budget(&self, writ: &Budget) -> Budget {
        match &self.budget_cap {
            None => writ.clone(),
            Some(c) => Budget {
                tokens: writ.tokens.min(c.tokens),
                tool_calls: writ.tool_calls.min(c.tool_calls),
                wall_clock_ms: writ.wall_clock_ms.min(c.wall_clock_ms),
                usd_millicents: writ.usd_millicents.min(c.usd_millicents),
            },
        }
    }

    /// Render `instructions` with `{key}` placeholders filled from `params`
    /// (supplied overrides first, then each param's `default`). Unknown keys are
    /// left untouched. This is the prompt fragment cognition sees; it changes
    /// *inputs to* the pipeline, never authority.
    pub fn render_instructions(&self, overrides: &[(String, String)]) -> String {
        let mut out = self.instructions.clone();
        for p in &self.params {
            let val = overrides
                .iter()
                .find(|(k, _)| *k == p.key)
                .map(|(_, v)| v.as_str())
                .unwrap_or(p.default.as_str());
            out = out.replace(&format!("{{{}}}", p.key), val);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ceiling(read: bool, write: bool, external: bool, irreversible: bool) -> EffectCeiling {
        EffectCeiling {
            read,
            write,
            external,
            irreversible,
        }
    }

    fn budget(t: u64, c: u64, w: u64, u: u64) -> Budget {
        Budget {
            tokens: t,
            tool_calls: c,
            wall_clock_ms: w,
            usd_millicents: u,
        }
    }

    fn skill() -> SkillDef {
        SkillDef {
            name: "diff-review".into(),
            version: 1,
            title: "Review a diff".into(),
            instructions: "Review with {strictness} strictness.".into(),
            tools: vec![ToolPattern::exact("fs.read"), ToolPattern::exact("git_*")],
            ceiling: ceiling(true, true, false, false),
            budget_cap: Some(budget(1000, 10, 60_000, 50)),
            params: vec![SkillParam {
                key: "strictness".into(),
                description: String::new(),
                default: "high".into(),
                values: vec!["low".into(), "high".into()],
            }],
            model_hint: ModelHint::default(),
        }
    }

    #[test]
    fn id_is_deterministic_and_tuning_changes_it() {
        let s = skill();
        assert_eq!(s.id(), skill().id(), "same def → same id");
        let mut tuned = skill();
        tuned.instructions = "Be terse.".into();
        assert_ne!(s.id(), tuned.id(), "tuning instructions mints a new id");
        let mut v2 = skill();
        v2.version = 2;
        assert_ne!(s.id(), v2.id(), "version bump mints a new id");
    }

    #[test]
    fn allow_list_only_narrows() {
        let s = skill();
        assert!(s.allows_tool("fs.read"));
        assert!(s.allows_tool("git_commit")); // covered by git_*
        assert!(!s.allows_tool("http.post")); // not in allow-list
        // Empty allow-list imposes no restriction.
        let mut open = skill();
        open.tools.clear();
        assert!(open.allows_tool("anything"));
    }

    #[test]
    fn ceiling_cap_is_field_wise_and_never_widens() {
        let s = skill(); // skill forbids external + irreversible
        // Writ allows everything; skill must strip external + irreversible.
        let eff = s.cap_ceiling(&ceiling(true, true, true, true));
        assert_eq!(eff.read, true);
        assert_eq!(eff.write, true);
        assert_eq!(eff.external, false);
        assert_eq!(eff.irreversible, false);
        // Skill cannot add an effect the writ forbids: writ has no write.
        let eff2 = s.cap_ceiling(&ceiling(true, false, false, false));
        assert_eq!(eff2.write, false, "AND can never add write the writ lacks");
    }

    #[test]
    fn budget_cap_is_field_wise_min() {
        let s = skill(); // cap 1000/10/60000/50
        let eff = s.cap_budget(&budget(500, 100, 1_000_000, 100));
        assert_eq!(eff.tokens, 500, "min(500,1000)");
        assert_eq!(eff.tool_calls, 10, "min(100,10)");
        assert_eq!(eff.wall_clock_ms, 60_000);
        assert_eq!(eff.usd_millicents, 50);
        // No cap → writ budget passes through unchanged.
        let mut nocap = skill();
        nocap.budget_cap = None;
        let same = nocap.cap_budget(&budget(7, 7, 7, 7));
        assert_eq!((same.tokens, same.tool_calls), (7, 7));
    }

    #[test]
    fn render_fills_params_with_overrides_then_defaults() {
        let s = skill();
        assert_eq!(s.render_instructions(&[]), "Review with high strictness.");
        assert_eq!(
            s.render_instructions(&[("strictness".into(), "low".into())]),
            "Review with low strictness."
        );
    }

    #[test]
    fn param_enum_validation() {
        let p = &skill().params[0];
        assert!(p.accepts("low"));
        assert!(!p.accepts("medium"));
    }
}
