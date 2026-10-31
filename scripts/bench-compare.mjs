// Compara los benchmarks de criterion contra la línea base guardada y falla si alguno
// empeoró más que el umbral. Lo usa la CI: primero corre los benchmarks de main con
// `--save-baseline main` y después los de la rama con `--baseline main`, en la misma
// máquina; criterion deja el cambio en target/criterion/<bench>/change/estimates.json.
//
//   node scripts/bench-compare.mjs [umbral]     (por defecto 0.20 = 20 %)
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";

const threshold = Number(process.argv[2] ?? 0.2);
const root = join(process.env.CARGO_TARGET_DIR ?? "target", "criterion");
if (!existsSync(root)) {
  console.error(`No hay resultados de criterion en ${root}`);
  process.exit(1);
}

const rows = [];
for (const bench of readdirSync(root)) {
  const file = join(root, bench, "change", "estimates.json");
  if (!existsSync(file)) continue;
  const change = JSON.parse(readFileSync(file, "utf8")).mean.point_estimate;
  rows.push({ bench, change });
}
if (rows.length === 0) {
  console.error("Ningún benchmark tiene comparación: ¿se corrió con --baseline main?");
  process.exit(1);
}

rows.sort((a, b) => b.change - a.change);
let worse = 0;
for (const { bench, change } of rows) {
  const pct = (change * 100).toFixed(1).padStart(7);
  const mark = change > threshold ? "  ← empeoró" : "";
  if (change > threshold) worse++;
  console.log(`${pct} %  ${bench}${mark}`);
}
if (worse > 0) {
  console.error(`${worse} benchmark(s) empeoraron más de ${(threshold * 100).toFixed(0)} %.`);
  process.exit(1);
}
console.log(`Ningún benchmark empeoró más de ${(threshold * 100).toFixed(0)} %.`);
