// Extrae los tipos del contrato (generados por tsify) del .d.ts del paquete WASM
// y los escribe en contract/contract.ts.
//
//   node scripts/gen-contract.mjs            escribe contract/contract.ts
//   node scripts/gen-contract.mjs --check    falla si el archivo del repo difiere (CI)
//   node scripts/gen-contract.mjs --out <ruta>   copia extra
//
// Si existe ../Frontend/package.json, también escribe ../Frontend/src/engine/contract.ts.
import { existsSync, readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const dts = join(root, "engine-wasm", "pkg", "engine_wasm.d.ts");
const target = join(root, "contract", "contract.ts");
const args = process.argv.slice(2);

if (!existsSync(dts)) {
  console.error("No existe engine-wasm/pkg/engine_wasm.d.ts. Ejecuta primero: npm run build:wasm");
  process.exit(1);
}

const src = readFileSync(dts, "utf8").replace(/\r\n/g, "\n");
const end = src.indexOf("\nexport function ");
const types = src
  .slice(0, end === -1 ? src.length : end)
  .replace(/^\/\* tslint:disable \*\/\n\/\* eslint-disable \*\/\n/, "")
  .trim();
const contract =
  "// ARCHIVO GENERADO desde los structs de Rust (engine/src/dto.rs) con tsify.\n" +
  "// No editar a mano: npm run gen:contract\n\n" +
  types +
  "\n";

if (args.includes("--check")) {
  const current = existsSync(target) ? readFileSync(target, "utf8").replace(/\r\n/g, "\n") : "";
  if (current !== contract) {
    console.error("contract/contract.ts no coincide con los structs de Rust. Ejecuta: npm run gen:contract");
    process.exit(1);
  }
  console.log("contract/contract.ts está al día.");
  process.exit(0);
}

const outputs = [target];
const frontend = join(root, "..", "Frontend");
if (existsSync(join(frontend, "package.json"))) outputs.push(join(frontend, "src", "engine", "contract.ts"));
const outIdx = args.indexOf("--out");
if (outIdx !== -1 && args[outIdx + 1]) outputs.push(resolve(process.cwd(), args[outIdx + 1]));
for (const out of outputs) {
  mkdirSync(dirname(out), { recursive: true });
  writeFileSync(out, contract);
  console.log(`Contrato escrito en ${out}`);
}
