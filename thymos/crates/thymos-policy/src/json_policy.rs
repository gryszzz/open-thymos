//! Declarative JSON policy bundles — the implementation of the
//! `policy-language-v1` RFC (Option A: a minimal, closed predicate DSL).
//!
//! A bundle is loadable at runtime (no recompile) and evaluates deterministically
//! over `(Intent, Writ)` — no wall clock, no RNG, no floats in the rule data, no
//! network — preserving the determinism the ledger and replay depend on. It
//! plugs into the existing [`PolicyEngine`](crate::PolicyEngine) as one
//! [`Policy`](crate::Policy).
//!
//! Example bundle:
//! ```json
//! {
//!   "name": "ops.policy",
//!   "version": "3",
//!   "rules": [
//!     { "name": "no-deletes",
//!       "when": { "field": "intent.target", "op": "eq", "value": "kv_del" },
//!       "decision": { "kind": "deny", "reason": "deletes are not allowed" } },
//!     { "name": "big-spend-approval",
//!       "when": { "field": "intent.args.amount", "op": "gt", "value": 1000 },
//!       "decision": { "kind": "require_approval", "channel": "ops", "reason": "amount over 1000" } }
//!   ]
//! }
//! ```
//!
//! Evaluation semantics: rules are tried in order; the **first rule whose
//! `when` matches** decides (its `decision` is returned). If no rule matches,
//! the bundle permits. A field path that does not resolve makes its leaf
//! condition false.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use thymos_core::{
    canonical_json_bytes,
    crypto::{self, PublicKey, SignatureBytes, SigningKey},
    error::{Error, Result},
    intent::Intent,
    proposal::PolicyDecision,
    world::World,
    writ::Writ,
};

use crate::Policy;

/// A loaded, named set of declarative rules. Construct with
/// [`JsonPolicySet::from_json`] (unsigned) or [`JsonPolicySet::from_signed_json`]
/// (verified ed25519 bundle).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonPolicySet {
    name: String,
    #[serde(default = "default_version")]
    version: String,
    rules: Vec<Rule>,
}

fn default_version() -> String {
    "1".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Rule {
    name: String,
    when: Condition,
    decision: Decision,
}

/// A boolean predicate over the evaluation context. Untagged: the JSON shape
/// (`all`/`any`/`not`/leaf) selects the variant.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
enum Condition {
    All { all: Vec<Condition> },
    Any { any: Vec<Condition> },
    Not { not: Box<Condition> },
    Leaf { field: String, op: Op, value: Value },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Op {
    Eq,
    Ne,
    Gt,
    Lt,
    Gte,
    Lte,
    Contains,
    StartsWith,
    In,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Decision {
    Permit,
    Deny { reason: String },
    RequireApproval { channel: String, reason: String },
}

/// A signed policy bundle: the policy plus the issuer's ed25519 signature over
/// `canonical_json(policy)`. Lets a deployment prove *which* rules governed a
/// trajectory and *who* authorized them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedPolicyBundle {
    policy: JsonPolicySet,
    #[serde(with = "crypto::hex32")]
    issuer_pubkey: PublicKey,
    #[serde(with = "crypto::hex64")]
    signature: SignatureBytes,
}

impl JsonPolicySet {
    /// Parse an **unsigned** bundle from JSON. Fails on malformed JSON / unknown
    /// ops (fail-closed: a bundle that does not parse is not loaded).
    pub fn from_json(s: &str) -> std::result::Result<Self, serde_json::Error> {
        serde_json::from_str(s)
    }

    /// Sign this bundle, returning a JSON `SignedPolicyBundle` string. The
    /// signature covers `canonical_json(policy)`.
    pub fn to_signed_json(&self, signing_key: &SigningKey) -> Result<String> {
        let msg = canonical_json_bytes(self)?;
        let bundle = SignedPolicyBundle {
            policy: self.clone(),
            issuer_pubkey: crypto::public_key_of(signing_key),
            signature: crypto::sign(signing_key, &msg),
        };
        serde_json::to_string(&bundle)
            .map_err(|e| Error::Other(format!("serialize signed policy bundle: {e}")))
    }

    /// Load a **signed** bundle, verifying its ed25519 signature (fail-closed).
    /// If `expected_issuer` is `Some`, the bundle's issuer must match it. The
    /// signature is checked over `canonical_json(policy)`.
    pub fn from_signed_json(s: &str, expected_issuer: Option<&PublicKey>) -> Result<Self> {
        let bundle: SignedPolicyBundle = serde_json::from_str(s)
            .map_err(|e| Error::Other(format!("parse signed policy bundle: {e}")))?;
        if let Some(expected) = expected_issuer {
            if &bundle.issuer_pubkey != expected {
                return Err(Error::AuthorityVoid(
                    "policy bundle issuer does not match the expected key".into(),
                ));
            }
        }
        let msg = canonical_json_bytes(&bundle.policy)?;
        crypto::verify(&bundle.issuer_pubkey, &msg, &bundle.signature)?;
        Ok(bundle.policy)
    }
}

impl Policy for JsonPolicySet {
    fn name(&self) -> &str {
        &self.name
    }

    fn version(&self) -> &str {
        &self.version
    }

    fn evaluate(&self, intent: &Intent, writ: &Writ, world: &World) -> PolicyDecision {
        for rule in &self.rules {
            if rule.when.eval(intent, writ, world) {
                return match &rule.decision {
                    Decision::Permit => PolicyDecision::Permit,
                    Decision::Deny { reason } => PolicyDecision::Deny(reason.clone()),
                    Decision::RequireApproval { channel, reason } => {
                        PolicyDecision::RequireApproval {
                            channel: channel.clone(),
                            reason: reason.clone(),
                        }
                    }
                };
            }
        }
        PolicyDecision::Permit
    }
}

impl Condition {
    fn eval(&self, intent: &Intent, writ: &Writ, world: &World) -> bool {
        match self {
            Condition::All { all } => all.iter().all(|c| c.eval(intent, writ, world)),
            Condition::Any { any } => any.iter().any(|c| c.eval(intent, writ, world)),
            Condition::Not { not } => !not.eval(intent, writ, world),
            Condition::Leaf { field, op, value } => {
                eval_leaf(resolve(field, intent, writ, world).as_ref(), *op, value)
            }
        }
    }
}

/// Resolve a dotted field path against the `(Intent, Writ, World)` context.
/// Returns `None` for unknown paths or missing args/resources.
///
/// `world.<kind>.<id>` resolves to the resource value (or `None` if absent);
/// `world.<kind>.<id>.version` resolves to its version. The world is read-only
/// and deterministic, so policies stay pure.
fn resolve(path: &str, intent: &Intent, writ: &Writ, world: &World) -> Option<Value> {
    match path {
        "intent.target" => Some(Value::String(intent.body.target.clone())),
        "intent.author" => Some(Value::String(intent.body.author.clone())),
        "intent.kind" => serde_json::to_value(intent.body.kind).ok(),
        "writ.tenant_id" => Some(Value::String(writ.body.tenant_id.clone())),
        "writ.subject" => Some(Value::String(writ.body.subject.clone())),
        "writ.issuer" => Some(Value::String(writ.body.issuer.clone())),
        _ => {
            if let Some(key) = path.strip_prefix("intent.args.") {
                return intent.body.args.get(key).cloned();
            }
            if let Some(rest) = path.strip_prefix("world.") {
                return resolve_world(rest, world);
            }
            None
        }
    }
}

/// `world.<kind>.<id>` → value; `world.<kind>.<id>.version` → version.
fn resolve_world(rest: &str, world: &World) -> Option<Value> {
    let (path, want_version) = match rest.strip_suffix(".version") {
        Some(p) => (p, true),
        None => (rest, false),
    };
    let (kind, id) = path.split_once('.')?;
    let state = world.get(&thymos_core::world::ResourceKey::new(kind, id))?;
    if want_version {
        Some(Value::from(state.version))
    } else {
        Some(state.value.clone())
    }
}

fn eval_leaf(field: Option<&Value>, op: Op, rule_value: &Value) -> bool {
    let field = match field {
        Some(v) => v,
        None => return false, // unresolved path → leaf is false
    };
    match op {
        Op::Eq => field == rule_value,
        Op::Ne => field != rule_value,
        Op::Gt => num_or_str_cmp(field, rule_value).map(|o| o.is_gt()).unwrap_or(false),
        Op::Lt => num_or_str_cmp(field, rule_value).map(|o| o.is_lt()).unwrap_or(false),
        Op::Gte => num_or_str_cmp(field, rule_value).map(|o| o.is_ge()).unwrap_or(false),
        Op::Lte => num_or_str_cmp(field, rule_value).map(|o| o.is_le()).unwrap_or(false),
        Op::Contains => match (field, rule_value) {
            (Value::String(s), Value::String(sub)) => s.contains(sub.as_str()),
            (Value::Array(arr), v) => arr.contains(v),
            _ => false,
        },
        Op::StartsWith => match (field.as_str(), rule_value.as_str()) {
            (Some(s), Some(p)) => s.starts_with(p),
            _ => false,
        },
        Op::In => rule_value
            .as_array()
            .map(|arr| arr.contains(field))
            .unwrap_or(false),
    }
}

/// Compare two JSON values as integers when both are integers, otherwise as
/// strings. Returns `None` for incomparable pairs. Floats are intentionally not
/// special-cased — rule data should use integers (Section: determinism).
fn num_or_str_cmp(a: &Value, b: &Value) -> Option<std::cmp::Ordering> {
    if let (Some(x), Some(y)) = (a.as_i64(), b.as_i64()) {
        return Some(x.cmp(&y));
    }
    if let (Some(x), Some(y)) = (a.as_str(), b.as_str()) {
        return Some(x.cmp(y));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use thymos_core::{
        crypto::{generate_signing_key, public_key_of},
        intent::{Intent, IntentBody, IntentKind},
        writ::{Budget, DelegationBounds, EffectCeiling, TimeWindow, ToolPattern, Writ, WritBody},
    };

    fn intent(target: &str, args: serde_json::Value) -> Intent {
        Intent::new(IntentBody {
            parent_commit: None,
            author: "tester".into(),
            kind: IntentKind::Act,
            target: target.into(),
            args,
            rationale: "t".into(),
            nonce: [0u8; 16],
        })
        .unwrap()
    }

    fn writ() -> Writ {
        let k = generate_signing_key();
        Writ::sign(
            WritBody {
                issuer: "root".into(),
                issuer_pubkey: public_key_of(&k),
                subject: "agent".into(),
                subject_pubkey: public_key_of(&generate_signing_key()),
                nonce: [0u8; 16],
                parent: None,
                tenant_id: "acme".into(),
                tool_scopes: vec![ToolPattern::exact("*")],
                budget: Budget {
                    tokens: 1,
                    tool_calls: 1,
                    wall_clock_ms: 1,
                    usd_millicents: 1,
                },
                effect_ceiling: EffectCeiling::read_write_local(),
                time_window: TimeWindow {
                    not_before: 0,
                    expires_at: u64::MAX,
                },
                delegation: DelegationBounds {
                    max_depth: 0,
                    may_subdivide: false,
                },
            },
            &k,
        )
        .unwrap()
    }

    fn eval(bundle: &str, target: &str, args: serde_json::Value) -> PolicyDecision {
        let set = JsonPolicySet::from_json(bundle).unwrap();
        set.evaluate(&intent(target, args), &writ(), &World::default())
    }

    #[test]
    fn deny_rule_matches_target() {
        let b = r#"{"name":"p","rules":[
            {"name":"no-del","when":{"field":"intent.target","op":"eq","value":"kv_del"},
             "decision":{"kind":"deny","reason":"no deletes"}}]}"#;
        assert!(matches!(eval(b, "kv_del", json!({})), PolicyDecision::Deny(r) if r == "no deletes"));
        assert!(matches!(eval(b, "kv_set", json!({})), PolicyDecision::Permit));
    }

    #[test]
    fn numeric_threshold_requires_approval() {
        let b = r#"{"name":"p","rules":[
            {"name":"big","when":{"field":"intent.args.amount","op":"gt","value":1000},
             "decision":{"kind":"require_approval","channel":"ops","reason":"big"}}]}"#;
        assert!(matches!(
            eval(b, "pay", json!({"amount": 5000})),
            PolicyDecision::RequireApproval { .. }
        ));
        assert!(matches!(eval(b, "pay", json!({"amount": 10})), PolicyDecision::Permit));
    }

    #[test]
    fn boolean_combinators_and_first_match_wins() {
        let b = r#"{"name":"p","version":"7","rules":[
            {"name":"r","when":{"all":[
                {"field":"intent.target","op":"eq","value":"deploy"},
                {"any":[
                    {"field":"writ.tenant_id","op":"eq","value":"acme"},
                    {"field":"writ.subject","op":"eq","value":"root"}]},
                {"not":{"field":"intent.args.dryrun","op":"eq","value":true}}]},
             "decision":{"kind":"deny","reason":"guarded deploy"}}]}"#;
        // all() true: target=deploy, tenant=acme, dryrun != true
        assert!(matches!(eval(b, "deploy", json!({})), PolicyDecision::Deny(_)));
        // not() flips: dryrun == true → all() false → no match → permit
        assert!(matches!(eval(b, "deploy", json!({"dryrun": true})), PolicyDecision::Permit));
        // version carried through
        assert_eq!(JsonPolicySet::from_json(b).unwrap().version(), "7");
    }

    #[test]
    fn in_and_contains_ops() {
        let b = r#"{"name":"p","rules":[
            {"name":"r","when":{"field":"intent.target","op":"in","value":["a","b","c"]},
             "decision":{"kind":"deny","reason":"listed"}}]}"#;
        assert!(matches!(eval(b, "b", json!({})), PolicyDecision::Deny(_)));
        assert!(matches!(eval(b, "z", json!({})), PolicyDecision::Permit));
    }

    #[test]
    fn malformed_bundle_fails_closed() {
        assert!(JsonPolicySet::from_json("{ not json").is_err());
        // unknown op is rejected
        assert!(JsonPolicySet::from_json(
            r#"{"name":"p","rules":[{"name":"r","when":{"field":"x","op":"regex","value":"y"},"decision":{"kind":"permit"}}]}"#
        )
        .is_err());
    }

    #[test]
    fn world_accessor_reads_resource_state() {
        use thymos_core::{
            delta::{DeltaOp, StructuredDelta},
            world::World,
            CommitId,
        };
        let mut world = World::default();
        world
            .apply(
                &StructuredDelta::single(DeltaOp::Create {
                    kind: "kv".into(),
                    id: "flag".into(),
                    value: json!("locked"),
                }),
                CommitId::ZERO,
            )
            .unwrap();

        let b = r#"{"name":"p","rules":[
            {"name":"locked","when":{"field":"world.kv.flag","op":"eq","value":"locked"},
             "decision":{"kind":"deny","reason":"resource is locked"}}]}"#;
        let set = JsonPolicySet::from_json(b).unwrap();

        // With the resource present and == "locked" → deny.
        assert!(matches!(
            set.evaluate(&intent("x", json!({})), &writ(), &world),
            PolicyDecision::Deny(_)
        ));
        // Empty world → path unresolved → leaf false → permit.
        assert!(matches!(
            set.evaluate(&intent("x", json!({})), &writ(), &World::default()),
            PolicyDecision::Permit
        ));
        // version accessor resolves too.
        let b2 = r#"{"name":"p","rules":[
            {"name":"v","when":{"field":"world.kv.flag.version","op":"gte","value":1},
             "decision":{"kind":"deny","reason":"exists"}}]}"#;
        assert!(matches!(
            JsonPolicySet::from_json(b2).unwrap().evaluate(&intent("x", json!({})), &writ(), &world),
            PolicyDecision::Deny(_)
        ));
    }

    #[test]
    fn signed_bundle_round_trips_and_rejects_tampering() {
        let key = generate_signing_key();
        let pubkey = public_key_of(&key);
        let set = JsonPolicySet::from_json(
            r#"{"name":"ops","version":"2","rules":[
                {"name":"r","when":{"field":"intent.target","op":"eq","value":"x"},
                 "decision":{"kind":"deny","reason":"no x"}}]}"#,
        )
        .unwrap();

        let signed = set.to_signed_json(&key).unwrap();

        // Verifies with the correct (and any-issuer) key.
        let loaded = JsonPolicySet::from_signed_json(&signed, Some(&pubkey)).unwrap();
        assert_eq!(loaded.version(), "2");
        JsonPolicySet::from_signed_json(&signed, None).unwrap();

        // Wrong expected issuer → rejected.
        let other = public_key_of(&generate_signing_key());
        assert!(JsonPolicySet::from_signed_json(&signed, Some(&other)).is_err());

        // Tampered rule body → signature no longer verifies (fail-closed).
        let tampered = signed.replace("no x", "ALLOW");
        assert!(JsonPolicySet::from_signed_json(&tampered, None).is_err());
    }
}
