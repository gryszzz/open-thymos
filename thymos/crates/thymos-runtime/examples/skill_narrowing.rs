//! `skill_narrowing` — watch a skill *shrink* authority, provably.
//!
//! A skill never grants authority; binding one to a run can only narrow it. This
//! example takes a broad writ, applies a read-only skill, and shows the effective
//! authority is a **strict subset** (write + external stripped, tools restricted,
//! budget capped). It then records the binding in a ledger and proves replay
//! re-verifies the inlined skill by hash — and rejects a tampered definition.
//!
//! Run:
//!     cargo run --example skill_narrowing -p thymos-runtime
//!
//! The same properties are asserted in `thymos-core`'s `skill` unit tests
//! (incl. a 4096-case randomized subset proof) and the server's e2e binding test.

use thymos_core::skill::SkillDef;
use thymos_ledger::{replay, EntryPayload, Ledger, ReplayConfig};
use thymos_runtime::{Budget, EffectCeiling, ToolPattern};
use thymos_core::TrajectoryId;

fn show_ceiling(c: &EffectCeiling) -> String {
    let mut f: Vec<&str> = Vec::new();
    if c.read {
        f.push("read");
    }
    if c.write {
        f.push("write");
    }
    if c.external {
        f.push("external");
    }
    if c.irreversible {
        f.push("irreversible");
    }
    if f.is_empty() {
        "(none)".into()
    } else {
        f.join("+")
    }
}

fn main() -> anyhow::Result<()> {
    println!("== thymos skill narrowing: cognition proposes, the writ governs ==\n");

    // 1. A BROAD writ: read + write + external, kv_* and http_*, generous budget.
    let writ_ceiling = EffectCeiling {
        read: true,
        write: true,
        external: true,
        irreversible: false,
    };
    let writ_budget = Budget {
        tokens: 100_000,
        tool_calls: 64,
        wall_clock_ms: 300_000,
        usd_millicents: 500,
    };
    let writ_tools = [ToolPattern::exact("kv_*"), ToolPattern::exact("http_*")];

    // 2. A read-only triage skill: kv_get only, no write/external, tight budget.
    let skill = SkillDef {
        name: "read-only-triage".into(),
        version: 1,
        title: "Read-only triage".into(),
        instructions: "Inspect state to answer; never mutate or call out.".into(),
        tools: vec![ToolPattern::exact("kv_get")],
        ceiling: EffectCeiling {
            read: true,
            write: false,
            external: false,
            irreversible: false,
        },
        budget_cap: Some(Budget {
            tokens: 10_000,
            tool_calls: 4,
            wall_clock_ms: 30_000,
            usd_millicents: 0,
        }),
        params: vec![],
        model_hint: Default::default(),
    };

    // 3. Effective authority = writ ∩ skill — exactly what the server computes
    //    before signing the run's writ.
    let eff_ceiling = skill.cap_ceiling(&writ_ceiling);
    let eff_budget = skill.cap_budget(&writ_budget);
    let eff_tools: Vec<ToolPattern> = skill
        .tools
        .iter()
        .filter(|st| writ_tools.iter().any(|w| w.covers(st)))
        .cloned()
        .collect();

    println!("ceiling   writ={:<22} skill={:<12} → effective={}", show_ceiling(&writ_ceiling), show_ceiling(&skill.ceiling), show_ceiling(&eff_ceiling));
    println!(
        "budget    writ.tool_calls={:<5} cap={:<5} → effective={}",
        writ_budget.tool_calls, 4, eff_budget.tool_calls
    );
    println!(
        "tools     writ={:?} ∩ skill={:?} → effective={:?}\n",
        writ_tools.iter().map(|t| &t.tool).collect::<Vec<_>>(),
        skill.tools.iter().map(|t| &t.tool).collect::<Vec<_>>(),
        eff_tools.iter().map(|t| &t.tool).collect::<Vec<_>>(),
    );

    // The invariant: a skill can only ever shrink authority.
    assert!(!eff_ceiling.write, "skill stripped write");
    assert!(!eff_ceiling.external, "skill stripped external");
    assert!(eff_budget.tool_calls <= writ_budget.tool_calls);
    assert!(eff_budget.usd_millicents <= writ_budget.usd_millicents);
    assert!(!skill.allows_tool("http_post"), "skill denies http_post");
    println!("✓ effective authority ⊊ writ — write + external stripped, tools + budget capped\n");

    // 4. Record the binding in a ledger and prove replay re-verifies it.
    let ledger = Ledger::open_in_memory()?;
    let traj = TrajectoryId::new_from_seed(b"skill-narrowing-demo");
    ledger.append_root(traj, "skill demo")?;
    let bound = ledger.append_skill_bound(traj, skill.clone(), vec![])?;
    println!(
        "ledger    seq {} = skill_bound({} v{})  id={}",
        bound.seq, skill.name, skill.version, skill.id()
    );

    let entries = ledger.entries(traj)?;
    let (_world, report) = replay(&entries, &ReplayConfig::default())?;
    println!(
        "replay    verified {} entries — incl. recomputing the skill hash ✓\n",
        report.entries_seen
    );

    // 5. Tamper proof: mutate the inlined definition, re-seal the entry id past
    //    the hash chain, and watch replay reject it on the skill self-check.
    let mut tampered = entries.clone();
    if let EntryPayload::SkillBound { skill, .. } = &mut tampered[1].payload {
        skill.ceiling.write = true; // try to grant write after the fact
    }
    tampered[1].id = thymos_core::content_hash(&tampered[1].payload)?;
    match replay(&tampered, &ReplayConfig::default()) {
        Ok(_) => anyhow::bail!("tampered skill should NOT replay"),
        Err(e) => println!("✓ tampered skill definition rejected by replay: {e}"),
    }

    println!("\nthymos: a skill narrows; the ledger remembers; replay proves it.");
    Ok(())
}
