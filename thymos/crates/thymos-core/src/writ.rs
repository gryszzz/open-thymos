//! Capability Writ.
//!
//! The sole source of authority in the runtime. A Writ authorizes a subject
//! (a cognitive process) to emit Intents that propose certain actions, within
//! declared tool scopes, under a budget, for a time window. Writs are
//! decomposable: a child Writ must be a strict subset of its parent, and the
//! child's issuer must be the parent's subject (no lateral minting).
//!
//! Signatures are mandatory. `signature` is an ed25519 signature over
//! `canonical_json(body)` produced with the private key that corresponds to
//! `body.issuer_pubkey`. Runtime verifies every writ before admitting it.

use ed25519_dalek::SigningKey;
use serde::{Deserialize, Serialize};

use crate::crypto::{self, hex32, hex64, PublicKey, SignatureBytes};
use crate::error::{Error, Result};
use crate::ids::WritId;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Writ {
    pub id: WritId,
    pub body: WritBody,
    /// ed25519 signature over `canonical_json(body)` by the issuer.
    #[serde(with = "hex64")]
    pub signature: SignatureBytes,
}

impl Writ {
    /// Sign `body` with `signing_key` and construct a Writ. The key must
    /// correspond to `body.issuer_pubkey`, otherwise returns AuthorityVoid.
    pub fn sign(body: WritBody, signing_key: &SigningKey) -> Result<Self> {
        let derived_pk = crypto::public_key_of(signing_key);
        if derived_pk != body.issuer_pubkey {
            return Err(Error::AuthorityVoid(
                "signing key does not match issuer_pubkey".into(),
            ));
        }
        let msg = crate::hash::canonical_json_bytes(&body)?;
        let signature = crypto::sign(signing_key, &msg);
        let id = WritId::new_from_seed(&msg);
        Ok(Writ {
            id,
            body,
            signature,
        })
    }

    /// Verify the ed25519 signature on this writ. This is a structural check;
    /// the runtime additionally checks the delegation chain against the
    /// parent writ (if any).
    pub fn verify_signature(&self) -> Result<()> {
        let msg = crate::hash::canonical_json_bytes(&self.body)?;
        crypto::verify(&self.body.issuer_pubkey, &msg, &self.signature)
    }

    /// Mint a child Writ as a strict subset of `self`. The child's
    /// `issuer_pubkey` must equal this writ's `subject_pubkey` (no lateral
    /// minting), and `signing_key` must correspond to that pubkey.
    pub fn mint_child(&self, child_body: WritBody, signing_key: &SigningKey) -> Result<Self> {
        child_body.verify_subset_of(&self.body)?;
        if child_body.issuer_pubkey != self.body.subject_pubkey {
            return Err(Error::AuthorityVoid(
                "child issuer must equal parent subject (broken delegation chain)".into(),
            ));
        }
        let mut body = child_body;
        body.parent = Some(self.id);
        Self::sign(body, signing_key)
    }

    /// Verify that this Writ authorizes a given tool call.
    pub fn authorizes_tool(&self, tool: &str) -> bool {
        self.body.tool_scopes.iter().any(|pat| pat.matches(tool))
    }

    /// Check remaining budget for a projected cost.
    pub fn check_budget(&self, cost: &BudgetCost) -> Result<()> {
        self.body.budget.check(cost)
    }

    /// Debit the budget (in place). Callers must persist the resulting Writ
    /// state via the ledger.
    pub fn debit(&mut self, cost: &BudgetCost) -> Result<()> {
        self.body.budget.debit(cost)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WritBody {
    pub issuer: String,
    #[serde(with = "hex32")]
    pub issuer_pubkey: PublicKey,
    pub subject: String,
    #[serde(with = "hex32")]
    pub subject_pubkey: PublicKey,
    /// 16 random bytes that make each issued writ uniquely identified and
    /// signed. Without it, two byte-identical writ bodies would share a
    /// `WritId` and could not be revoked independently; it is the anti-replay /
    /// unique-identity component of the capability. Use
    /// [`crate::crypto::random_nonce`] when minting.
    pub nonce: [u8; 16],
    pub parent: Option<WritId>,
    /// Tenant isolation: every writ belongs to exactly one tenant. Child writs
    /// must inherit the same tenant_id — cross-tenant delegation is forbidden.
    /// Empty string means "system" / no tenant scoping (backwards-compatible).
    #[serde(default)]
    pub tenant_id: String,
    pub tool_scopes: Vec<ToolPattern>,
    pub budget: Budget,
    pub effect_ceiling: EffectCeiling,
    pub time_window: TimeWindow,
    pub delegation: DelegationBounds,
}

impl WritBody {
    /// Verify that `self` is a strict subset of `parent`.
    pub fn verify_subset_of(&self, parent: &WritBody) -> Result<()> {
        // Tenant isolation: child must have the same tenant_id as parent.
        if self.tenant_id != parent.tenant_id {
            return Err(Error::AuthorityVoid(format!(
                "cross-tenant delegation forbidden: parent tenant '{}', child tenant '{}'",
                parent.tenant_id, self.tenant_id
            )));
        }

        // Tool scopes: every child pattern must be matched by some parent pattern.
        for child in &self.tool_scopes {
            let covered = parent.tool_scopes.iter().any(|p| p.covers(child));
            if !covered {
                return Err(Error::AuthorityVoid(format!(
                    "child tool scope '{}' not covered by parent",
                    child.tool
                )));
            }
        }
        // Budget: strictly less-than-or-equal on each dimension.
        if self.budget.tokens > parent.budget.tokens
            || self.budget.tool_calls > parent.budget.tool_calls
            || self.budget.wall_clock_ms > parent.budget.wall_clock_ms
            || self.budget.usd_millicents > parent.budget.usd_millicents
        {
            return Err(Error::AuthorityVoid("child budget exceeds parent".into()));
        }
        // Effect ceiling: child cannot grant effects parent forbids.
        if self.effect_ceiling.write && !parent.effect_ceiling.write {
            return Err(Error::AuthorityVoid(
                "child grants write, parent does not".into(),
            ));
        }
        if self.effect_ceiling.external && !parent.effect_ceiling.external {
            return Err(Error::AuthorityVoid(
                "child grants external, parent does not".into(),
            ));
        }
        if self.effect_ceiling.irreversible && !parent.effect_ceiling.irreversible {
            return Err(Error::AuthorityVoid(
                "child grants irreversible, parent does not".into(),
            ));
        }
        // Time window: child must fit within parent's window.
        if self.time_window.not_before < parent.time_window.not_before
            || self.time_window.expires_at > parent.time_window.expires_at
        {
            return Err(Error::AuthorityVoid(
                "child time window outside parent".into(),
            ));
        }
        // Delegation depth — overflow-safe comparison. A child's max_depth
        // plus one (the edge that creates the child itself) must not exceed
        // the parent's remaining depth.
        if self.delegation.max_depth.saturating_add(1) > parent.delegation.max_depth {
            return Err(Error::AuthorityVoid(
                "child delegation depth exceeds parent remaining depth".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolPattern {
    /// Literal name or simple glob (`*` suffix, e.g. `order_*`).
    pub tool: String,
}

impl ToolPattern {
    pub fn exact(name: impl Into<String>) -> Self {
        ToolPattern { tool: name.into() }
    }

    pub fn matches(&self, tool: &str) -> bool {
        if let Some(prefix) = self.tool.strip_suffix('*') {
            tool.starts_with(prefix)
        } else {
            self.tool == tool
        }
    }

    /// Returns true if `self` covers every tool that `other` covers.
    pub fn covers(&self, other: &ToolPattern) -> bool {
        if let Some(prefix) = self.tool.strip_suffix('*') {
            if let Some(other_prefix) = other.tool.strip_suffix('*') {
                other_prefix.starts_with(prefix)
            } else {
                other.tool.starts_with(prefix)
            }
        } else {
            self.tool == other.tool
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Budget {
    pub tokens: u64,
    pub tool_calls: u64,
    pub wall_clock_ms: u64,
    pub usd_millicents: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BudgetCost {
    pub tokens: u64,
    pub tool_calls: u64,
    pub wall_clock_ms: u64,
    pub usd_millicents: u64,
}

impl BudgetCost {
    pub fn saturating_add(&self, other: &BudgetCost) -> BudgetCost {
        BudgetCost {
            tokens: self.tokens.saturating_add(other.tokens),
            tool_calls: self.tool_calls.saturating_add(other.tool_calls),
            wall_clock_ms: self.wall_clock_ms.saturating_add(other.wall_clock_ms),
            usd_millicents: self.usd_millicents.saturating_add(other.usd_millicents),
        }
    }
}

impl Budget {
    pub fn check(&self, cost: &BudgetCost) -> Result<()> {
        if cost.tokens > self.tokens {
            return Err(Error::BudgetExhausted(format!(
                "tokens: need {}, have {}",
                cost.tokens, self.tokens
            )));
        }
        if cost.tool_calls > self.tool_calls {
            return Err(Error::BudgetExhausted("tool_calls".into()));
        }
        if cost.wall_clock_ms > self.wall_clock_ms {
            return Err(Error::BudgetExhausted("wall_clock_ms".into()));
        }
        if cost.usd_millicents > self.usd_millicents {
            return Err(Error::BudgetExhausted("usd_millicents".into()));
        }
        Ok(())
    }

    pub fn debit(&mut self, cost: &BudgetCost) -> Result<()> {
        self.check(cost)?;
        self.tokens -= cost.tokens;
        self.tool_calls -= cost.tool_calls;
        self.wall_clock_ms -= cost.wall_clock_ms;
        self.usd_millicents -= cost.usd_millicents;
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EffectCeiling {
    pub read: bool,
    pub write: bool,
    pub external: bool,
    pub irreversible: bool,
}

impl EffectCeiling {
    pub fn read_write_local() -> Self {
        Self {
            read: true,
            write: true,
            external: false,
            irreversible: false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TimeWindow {
    pub not_before: u64,
    pub expires_at: u64,
}

impl TimeWindow {
    /// True if `now` (unix seconds) lies within [not_before, expires_at].
    pub fn contains(&self, now: u64) -> bool {
        now >= self.not_before && now <= self.expires_at
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DelegationBounds {
    pub max_depth: u32,
    pub may_subdivide: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::{generate_signing_key, public_key_of};

    fn parent_pair() -> (Writ, SigningKey, SigningKey) {
        // Root issuer signs; subject is a different key (the "agent") that
        // may mint children.
        let issuer = generate_signing_key();
        let subject = generate_signing_key();
        let body = WritBody {
            issuer: "root".into(),
            issuer_pubkey: public_key_of(&issuer),
            subject: "agent".into(),
            subject_pubkey: public_key_of(&subject),
            nonce: [0u8; 16],
            parent: None,
            tenant_id: String::new(),
            tool_scopes: vec![ToolPattern::exact("kv_*")],
            budget: Budget {
                tokens: 1000,
                tool_calls: 10,
                wall_clock_ms: 60_000,
                usd_millicents: 100,
            },
            effect_ceiling: EffectCeiling::read_write_local(),
            time_window: TimeWindow {
                not_before: 0,
                expires_at: u64::MAX,
            },
            delegation: DelegationBounds {
                max_depth: 3,
                may_subdivide: true,
            },
        };
        let writ = Writ::sign(body, &issuer).expect("sign root");
        (writ, issuer, subject)
    }

    #[test]
    fn signature_verifies() {
        let (writ, _, _) = parent_pair();
        writ.verify_signature().expect("signature should verify");
    }

    #[test]
    fn tampered_body_fails_verification() {
        let (mut writ, _, _) = parent_pair();
        writ.body.subject = "evil".into();
        assert!(writ.verify_signature().is_err());
    }

    #[test]
    fn subset_valid_and_delegated() {
        let (parent, _issuer, agent) = parent_pair();
        let grandchild_subject = generate_signing_key();
        let child_body = WritBody {
            issuer: "agent".into(),
            issuer_pubkey: public_key_of(&agent),
            subject: "grandchild".into(),
            subject_pubkey: public_key_of(&grandchild_subject),
            nonce: [0u8; 16],
            parent: None,
            tenant_id: String::new(),
            tool_scopes: vec![ToolPattern::exact("kv_set")],
            budget: Budget {
                tokens: 100,
                tool_calls: 1,
                wall_clock_ms: 1000,
                usd_millicents: 10,
            },
            effect_ceiling: EffectCeiling::read_write_local(),
            time_window: parent.body.time_window.clone(),
            delegation: DelegationBounds {
                max_depth: 1,
                may_subdivide: false,
            },
        };
        let child = parent.mint_child(child_body, &agent).expect("mint");
        child.verify_signature().expect("child sig");
    }

    #[test]
    fn mint_child_rejects_wrong_signer() {
        let (parent, issuer, _agent) = parent_pair();
        let someone_else = generate_signing_key();
        let child_body = WritBody {
            issuer: "intruder".into(),
            issuer_pubkey: public_key_of(&someone_else),
            subject: "child".into(),
            subject_pubkey: public_key_of(&someone_else),
            nonce: [0u8; 16],
            parent: None,
            tenant_id: String::new(),
            tool_scopes: vec![ToolPattern::exact("kv_set")],
            budget: Budget {
                tokens: 100,
                tool_calls: 1,
                wall_clock_ms: 1000,
                usd_millicents: 10,
            },
            effect_ceiling: EffectCeiling::read_write_local(),
            time_window: parent.body.time_window.clone(),
            delegation: DelegationBounds {
                max_depth: 1,
                may_subdivide: false,
            },
        };
        // Even if we ignore the chain check, the original issuer trying to
        // sign a child whose issuer_pubkey isn't theirs fails.
        assert!(parent.mint_child(child_body, &issuer).is_err());
    }

    #[test]
    fn subset_rejects_budget_overflow() {
        let (parent, _issuer, agent) = parent_pair();
        let sub = generate_signing_key();
        let child_body = WritBody {
            issuer: "agent".into(),
            issuer_pubkey: public_key_of(&agent),
            subject: "child".into(),
            subject_pubkey: public_key_of(&sub),
            nonce: [0u8; 16],
            parent: None,
            tenant_id: String::new(),
            tool_scopes: vec![ToolPattern::exact("kv_set")],
            budget: Budget {
                tokens: 10_000,
                tool_calls: 1,
                wall_clock_ms: 1000,
                usd_millicents: 10,
            },
            effect_ceiling: EffectCeiling::read_write_local(),
            time_window: parent.body.time_window.clone(),
            delegation: DelegationBounds {
                max_depth: 1,
                may_subdivide: false,
            },
        };
        assert!(parent.mint_child(child_body, &agent).is_err());
    }

    #[test]
    fn time_window_contains() {
        let w = TimeWindow {
            not_before: 100,
            expires_at: 200,
        };
        assert!(w.contains(150));
        assert!(!w.contains(50));
        assert!(!w.contains(250));
    }

    #[test]
    fn delegation_depth_does_not_overflow_at_max() {
        // F3 regression: an attacker-supplied `max_depth = u32::MAX` previously
        // panicked here in debug builds due to `max_depth + 1` overflow. The
        // saturating add must instead return AuthorityVoid cleanly.
        let (parent, _issuer, agent) = parent_pair();
        let sub = generate_signing_key();
        let child_body = WritBody {
            issuer: "agent".into(),
            issuer_pubkey: public_key_of(&agent),
            subject: "child".into(),
            subject_pubkey: public_key_of(&sub),
            nonce: [0u8; 16],
            parent: None,
            tenant_id: String::new(),
            tool_scopes: vec![ToolPattern::exact("kv_set")],
            budget: Budget {
                tokens: 100,
                tool_calls: 1,
                wall_clock_ms: 1000,
                usd_millicents: 10,
            },
            effect_ceiling: EffectCeiling::read_write_local(),
            time_window: parent.body.time_window.clone(),
            delegation: DelegationBounds {
                max_depth: u32::MAX,
                may_subdivide: false,
            },
        };
        // Must return a clean AuthorityVoid, not panic, and not silently
        // succeed (parent.max_depth is 3, far below u32::MAX).
        let err = parent
            .mint_child(child_body, &agent)
            .err()
            .expect("u32::MAX child depth must reject");
        assert!(
            err.to_string().contains("delegation depth"),
            "expected delegation-depth rejection, got: {err}"
        );
    }

    #[test]
    fn nonce_makes_otherwise_identical_writs_distinct() {
        let issuer = generate_signing_key();
        let subject = generate_signing_key();
        let mk = |nonce: [u8; 16]| {
            let body = WritBody {
                issuer: "root".into(),
                issuer_pubkey: public_key_of(&issuer),
                subject: "agent".into(),
                subject_pubkey: public_key_of(&subject),
                nonce,
                parent: None,
                tenant_id: String::new(),
                tool_scopes: vec![ToolPattern::exact("kv_*")],
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
            };
            Writ::sign(body, &issuer).unwrap()
        };

        let a = mk([1u8; 16]);
        let b = mk([2u8; 16]);
        assert_ne!(a.id, b.id, "different nonces must yield different WritIds");
        a.verify_signature().expect("a signature valid");
        b.verify_signature().expect("b signature valid");

        // Same nonce → same id (deterministic content-addressing).
        let a2 = mk([1u8; 16]);
        assert_eq!(a.id, a2.id);
    }
}
