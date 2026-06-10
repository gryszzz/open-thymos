import { execFileSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const desktopDir = path.resolve(scriptDir, "..");
const workspaceDir = path.resolve(desktopDir, "../..");
const host = execFileSync("rustc", ["-vV"], { encoding: "utf8" })
  .match(/^host:\s+(.+)$/m)?.[1];

if (!host) {
  throw new Error("Could not determine the Rust host target");
}

const extension = host.includes("windows") ? ".exe" : "";
const result = spawnSync(
  "cargo",
  ["build", "--release", "--target", host, "-p", "thymos-server"],
  { cwd: workspaceDir, stdio: "inherit" },
);

if (result.status !== 0) {
  process.exit(result.status ?? 1);
}

const source = path.join(
  workspaceDir,
  "target",
  host,
  "release",
  `thymos-server${extension}`,
);
const binariesDir = path.join(desktopDir, "src-tauri", "binaries");
const destination = path.join(
  binariesDir,
  `thymos-server-${host}${extension}`,
);

fs.mkdirSync(binariesDir, { recursive: true });
fs.copyFileSync(source, destination);
fs.chmodSync(destination, 0o755);
console.log(`Prepared desktop runtime sidecar: ${destination}`);
