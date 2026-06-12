#!/usr/bin/env node
// Desktop UI smoke test — catches the two regressions we hit repeatedly by
// hand: (1) a JS file that doesn't parse, (2) a `$("id")` /
// getElementById("id") reference with no matching element in index.html.
// Pure static analysis (no browser); fast, dependency-free, CI-friendly.
import { readFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "src");
let failures = 0;
const fail = (m) => { console.error("✗ " + m); failures++; };

// 1. Both scripts must parse.
for (const f of ["main.js", "viz.js"]) {
  try {
    execFileSync(process.execPath, ["--check", join(root, f)], { stdio: "pipe" });
    console.log(`✓ ${f} parses`);
  } catch (e) {
    fail(`${f} syntax error:\n${e.stderr || e}`);
  }
}

// 2. Every referenced element id must exist in index.html.
const html = readFileSync(join(root, "index.html"), "utf8");
const htmlIds = new Set([...html.matchAll(/id="([A-Za-z0-9_-]+)"/g)].map((m) => m[1]));
const js = ["main.js", "viz.js"].map((f) => readFileSync(join(root, f), "utf8")).join("\n");
// Ids created at runtime (`el.id = "x"` / `.id="x"`) are legitimately absent
// from the static HTML — exclude them.
const dynamicIds = new Set([...js.matchAll(/\.id\s*=\s*"([A-Za-z0-9_-]+)"/g)].map((m) => m[1]));
const refs = new Set();
for (const m of js.matchAll(/\$\("([A-Za-z0-9_-]+)"\)/g)) refs.add(m[1]);
for (const m of js.matchAll(/getElementById\("([A-Za-z0-9_-]+)"\)/g)) refs.add(m[1]);
let missing = 0;
for (const id of refs) {
  if (!htmlIds.has(id) && !dynamicIds.has(id)) {
    fail(`element id "${id}" referenced in JS but not in index.html`); missing++;
  }
}
if (!missing) console.log(`✓ all ${refs.size} referenced element ids exist in index.html`);

if (failures) { console.error(`\n${failures} smoke check(s) failed.`); process.exit(1); }
console.log("\nDesktop UI smoke test passed.");
