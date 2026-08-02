import { spawnSync } from "node:child_process";
import { mkdirSync, existsSync, rmSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const root = join(__dirname, "..", "..");
const outDir = join(__dirname, "..", "pkg");

function run(cmd, args, opts = {}) {
  const r = spawnSync(cmd, args, { stdio: "inherit", shell: true, ...opts });
  if (r.status !== 0) process.exit(r.status ?? 1);
}

function cargoTargetDir() {
  const r = spawnSync(
    "cargo",
    ["metadata", "--format-version", "1", "--no-deps"],
    { cwd: root, encoding: "utf8", shell: true },
  );
  if (r.status !== 0) {
    console.error(r.stderr);
    process.exit(r.status ?? 1);
  }
  return JSON.parse(r.stdout).target_directory;
}

run(
  "cargo",
  [
    "build",
    "-p",
    "emulator-web",
    "--target",
    "wasm32-unknown-unknown",
    "--release",
  ],
  { cwd: root },
);

const wasmArtifact = join(
  cargoTargetDir(),
  "wasm32-unknown-unknown",
  "release",
  "emulator_web.wasm",
);

if (!existsSync(wasmArtifact)) {
  console.error("missing wasm artifact", wasmArtifact);
  process.exit(1);
}

if (existsSync(outDir)) rmSync(outDir, { recursive: true });
mkdirSync(outDir, { recursive: true });

run("wasm-bindgen", [
  wasmArtifact,
  "--out-dir",
  outDir,
  "--target",
  "web",
  "--no-typescript",
]);

console.log("wasm pkg written to", outDir);
