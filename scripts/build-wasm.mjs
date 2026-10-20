// Compila engine-wasm con wasm-pack (target web, release, optimizado con wasm-opt)
// y deja el paquete @bj/engine-wasm en engine-wasm/pkg.
//
// El front lo consume como dependencia local:
//   "@bj/engine-wasm": "file:../Backend/engine-wasm/pkg"
import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const pkgDir = join(root, "engine-wasm", "pkg");

execFileSync(
  "wasm-pack",
  ["build", "engine-wasm", "--target", "web", "--release", "--out-dir", "pkg", "--out-name", "engine_wasm"],
  { cwd: root, stdio: "inherit" },
);

// wasm-pack nombra el paquete como el crate; el front lo importa como @bj/engine-wasm.
const pkgJson = join(pkgDir, "package.json");
const pkg = JSON.parse(readFileSync(pkgJson, "utf8"));
pkg.name = "@bj/engine-wasm";
pkg.description = "Motor exacto de probabilidades de blackjack (WebAssembly)";
pkg.private = true;
writeFileSync(pkgJson, JSON.stringify(pkg, null, 2) + "\n");
console.log(`Paquete ${pkg.name} ${pkg.version} listo en ${pkgDir}`);
