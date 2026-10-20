// ARCHIVO GENERADO desde los structs de Rust (engine/src/dto.rs) con tsify.
// No editar a mano: npm run gen:contract

/**
 * Qué acciones permite la situación de la mesa (el front puede restringir más).
 */
export interface Allowed {
    double: boolean;
    split: boolean;
    surrender: boolean;
}

/**
 * Rango de una carta: 0 = As, 1 = 2, ..., 8 = 9, 9 = cualquier carta de valor 10.
 * J, Q y K no existen para el motor.
 */
export type Rank = number;

export interface ActionResult {
    action: Action;
    ev: number;
    pWin: number;
    pPush: number;
    pLose: number;
    exact: boolean;
}

export interface AnalyzeDealerRequest {
    version: number;
    rules: Rules;
    /**
     * Cartas sin ver. Si `hole` viene informada, NO debe incluirla.
     */
    shoe: [number, number, number, number, number, number, number, number, number, number];
    up: Rank;
    hole?: Rank;
    /**
     * Cartas que el dealer ya sacó después de la visible y la oculta, en orden.
     * Requiere `hole`; el zapato tampoco las incluye.
     */
    drawn?: Rank[];
}

export interface AnalyzeDealerResponse {
    version: number;
    dealer: DealerDistribution;
    /**
     * Probabilidad de que el dealer se pase con la próxima carta. Solo viene cuando
     * se conocen todas sus cartas (`hole` informada) y las reglas lo obligan a pedir.
     */
    pBustNext?: number;
    computeMs: number;
}

export interface AnalyzeHandRequest {
    version: number;
    rules: Rules;
    /**
     * Cartas que quedan sin ver, por rango. Incluye la carta oculta del dealer
     * y excluye las cartas visibles (mano del jugador, carta visible del dealer...).
     */
    shoe: [number, number, number, number, number, number, number, number, number, number];
    hand: Rank[];
    dealerUp: Rank;
    /**
     * Cuántas veces se ha dividido ya en esta ronda (0 = la mano no viene de dividir).
     */
    splitHandsSoFar?: number;
    /**
     * La mano viene de dividir ases.
     */
    isSplitAces?: boolean;
    allowed?: Allowed;
}

export interface AnalyzeHandResponse {
    version: number;
    actions: ActionResult[];
    best: Action;
    pBustOnHit: number;
    dealer: DealerDistribution;
    /**
     * Lo mide el binding (WASM/HTTP); el motor no usa reloj y lo deja en 0.
     */
    computeMs: number;
}

export interface AnalyzeRoundRequest {
    version: number;
    rules: Rules;
    /**
     * Cartas sin ver: incluye la oculta y excluye todas las manos visibles.
     */
    shoe: [number, number, number, number, number, number, number, number, number, number];
    up: Rank;
    hands: Rank[][];
}

export interface AnalyzeRoundResponse {
    version: number;
    hands: AnalyzeHandResponse[];
    /**
     * −promedio del mejor EV de cada mano. Aproximación: ignora las cartas que
     * sacarán los otros jugadores después.
     */
    houseEdgeRound: number;
    computeMs: number;
}

export interface DealerDistribution {
    p17: number;
    p18: number;
    p19: number;
    p20: number;
    p21: number;
    pBlackjack: number;
    pBust: number;
}

export interface Rules {
    decks: number;
    dealerHitsSoft17: boolean;
    doubleAfterSplit: boolean;
    doubleOn: DoubleOn;
    maxSplitHands: number;
    resplitAces: boolean;
    hitSplitAces: boolean;
    surrender: Surrender;
    dealerPeeks: boolean;
    blackjackPays: number;
    /**
     * La usa el front para rebarajar; el motor la ignora.
     */
    penetration: number;
}

export type Action = "stand" | "hit" | "double" | "split" | "surrender";

export type DoubleOn = "any" | "nineToEleven" | "tenToEleven";

export type Surrender = "none" | "late";
