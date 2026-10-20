// Prueba de humo del paquete WASM desde Node: carga el módulo y llama las tres funciones.
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import assert from "node:assert/strict";

const pkg = join(dirname(fileURLToPath(import.meta.url)), "..", "engine-wasm", "pkg");
const mod = await import(pathToFileURL(join(pkg, "engine_wasm.js")).href);
mod.initSync({ module: readFileSync(join(pkg, "engine_wasm_bg.wasm")) });

const rules = {
  decks: 6,
  dealerHitsSoft17: false,
  doubleAfterSplit: true,
  doubleOn: "any",
  maxSplitHands: 4,
  resplitAces: false,
  hitSplitAces: false,
  surrender: "late",
  dealerPeeks: true,
  blackjackPays: 1.5,
  penetration: 0.75,
};
// 6 mazos sin las cartas visibles: mano 10,6 y carta visible del dealer 10.
const shoe = [24, 24, 24, 24, 24, 23, 24, 24, 24, 94];

const hand = mod.analyzeHand({ version: 3, rules, shoe, hand: [9, 5], dealerUp: 9 });
assert.equal(hand.version, 3);
assert.deepEqual(hand.actions.map((a) => a.action), ["stand", "hit", "double", "surrender"]);
for (const a of hand.actions) assert.ok(Math.abs(a.pWin + a.pPush + a.pLose - 1) < 1e-9);
assert.equal(hand.dealer.pBlackjack, 0);
assert.ok(hand.computeMs >= 0);
console.log("analyzeHand 10,6 vs 10:", hand.best, hand.actions.map((a) => `${a.action}=${a.ev.toFixed(5)}`).join(" "), `(${hand.computeMs} ms)`);

const dealer = mod.analyzeDealer({ version: 1, rules, shoe: [24, 24, 24, 24, 24, 23, 24, 24, 24, 96], up: 5 });
assert.ok(Math.abs(dealer.dealer.pBust - 0.422842) < 1e-6);
console.log("analyzeDealer 6D S17 visible 6: pBust =", dealer.dealer.pBust.toFixed(6));

const round = mod.analyzeRound({
  version: 1,
  rules,
  shoe: [23, 24, 24, 24, 24, 23, 24, 23, 24, 92],
  up: 9,
  hands: [[9, 5], [0, 7], [9, 9]],
});
assert.equal(round.hands.length, 3);
console.log("analyzeRound houseEdgeRound =", round.houseEdgeRound.toFixed(5));

assert.throws(() => mod.analyzeHand({ version: 1, rules, shoe: [0, 0, 0, 0, 0, 0, 0, 0, 0, 3], hand: [9, 5], dealerUp: 9 }), /zapato/);
console.log("Errores de validación llegan a JS como excepción.");
console.log("Prueba de humo WASM: OK");
