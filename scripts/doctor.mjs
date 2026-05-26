import { execFileSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

const repoRoot = process.cwd();
const checks = [];

function pass(name, detail) {
  checks.push({ ok: true, name, detail });
}

function fail(name, detail) {
  checks.push({ ok: false, name, detail });
}

function hasCommand(command) {
  const result = spawnSync(command, ["--version"], { stdio: "ignore" });
  return result.status === 0;
}

function run(command, args, options = {}) {
  return execFileSync(command, args, {
    cwd: repoRoot,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
    ...options,
  }).trim();
}

function fileExists(relativePath) {
  return fs.existsSync(path.join(repoRoot, relativePath));
}

function checkCommand(command, label = command) {
  if (hasCommand(command)) {
    pass(`${label} installed`, run(command, ["--version"]).split("\n")[0]);
  } else {
    fail(`${label} installed`, `${command} was not found on PATH`);
  }
}

checkCommand("node", "Node.js");
checkCommand("npm");
checkCommand("cargo", "Cargo");

if (fileExists("package-lock.json")) {
  pass("npm lockfile", "package-lock.json present");
} else {
  fail("npm lockfile", "package-lock.json missing");
}

if (fileExists("thymos/Cargo.lock")) {
  pass("cargo lockfile", "thymos/Cargo.lock present");
} else {
  fail("cargo lockfile", "thymos/Cargo.lock missing");
}

for (const required of [
  "README.md",
  "docs/index.md",
  "docs/getting-started.md",
  "src/app/runs/page.tsx",
  "thymos/Cargo.toml",
]) {
  if (fileExists(required)) {
    pass(`required file ${required}`, "present");
  } else {
    fail(`required file ${required}`, "missing");
  }
}

try {
  run("npm", ["run", "docs:check"]);
  pass("documentation links", "docs:check passed");
} catch (error) {
  fail("documentation links", error.stderr?.toString() || error.message);
}

try {
  const staleDomainPattern = `thymos\\${"."}ai|${"PAGES_CUSTOM"}_DOMAIN`;
  const hits = run("git", [
    "grep",
    "--untracked",
    "-n",
    "-I",
    "-E",
    staleDomainPattern,
    "--",
    ".",
  ]);
  fail("custom domain scrub", hits);
} catch {
  pass("custom domain scrub", "no stale custom-domain strings in repo files");
}

try {
  const cnameFiles = run("git", ["ls-files", "--cached", "--others", "--exclude-standard"])
    .split("\n")
    .filter((filePath) => path.basename(filePath) === "CNAME");
  if (cnameFiles.length > 0) {
    fail("CNAME files", cnameFiles.join("\n"));
  } else {
    pass("CNAME files", "none present");
  }
} catch (error) {
  fail("CNAME files", error.stderr?.toString() || error.message);
}

if (hasCommand("gh")) {
  try {
    const pages = JSON.parse(
      run("gh", [
        "api",
        "repos/gryszzz/OpenThymos/pages",
        "--jq",
        "{html_url, custom_domain, cname, source, build_type, status}",
      ]),
    );
    const customDomainClean = pages.custom_domain == null && pages.cname == null;
    if (customDomainClean) {
      pass("GitHub Pages custom domain", "no custom domain configured");
    } else {
      fail(
        "GitHub Pages custom domain",
        `custom_domain=${pages.custom_domain} cname=${pages.cname}`,
      );
    }

    if (pages.html_url === "https://gryszzz.github.io/OpenThymos/") {
      pass("GitHub Pages URL", pages.html_url);
    } else {
      fail("GitHub Pages URL", pages.html_url);
    }

    if (pages.source?.branch === "main" && pages.source?.path === "/docs") {
      pass("GitHub Pages source", "main /docs");
    } else {
      fail("GitHub Pages source", JSON.stringify(pages.source));
    }
  } catch (error) {
    fail("GitHub Pages API", error.stderr?.toString() || error.message);
  }
} else {
  pass("GitHub CLI optional", "gh not installed, skipped remote Pages checks");
}

const failed = checks.filter((check) => !check.ok);

console.log("\nOpenThymos Doctor\n");
for (const check of checks) {
  const marker = check.ok ? "OK " : "ERR";
  console.log(`${marker}  ${check.name}`);
  if (check.detail) {
    console.log(`     ${String(check.detail).split("\n").slice(0, 3).join("\n     ")}`);
  }
}

if (failed.length > 0) {
  console.error(`\n${failed.length} check(s) failed.`);
  process.exit(1);
}

console.log("\nAll checks passed.");
