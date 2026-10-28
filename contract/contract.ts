// ARCHIVO GENERADO desde los structs de Rust (engine/src/dto.rs) con tsify.
// No editar a mano: npm run gen:contract

/**
 * Carta visible: figura y palo.
 */
export interface CardView {
    rank: Face;
    suit: Suit;
}

/**
 * El turno del dealer se resuelve dentro de la misma respuesta: no hay fase visible
 * para él.
 */
export type RoundPhase = "player" | "settled";

/**
 * Figura de la carta, como la escribe el front: "A", "2" … "10", "J", "Q", "K".
 */
export type Face = "A" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "10" | "J" | "Q" | "K";

/**
 * Lo único que el cliente ve de una ronda. Nunca incluye la oculta sin revelar ni
 * el orden del zapato.
 */
export interface RoundView {
    id: string;
    kind: RoundKind;
    /**
     * Id de la partida casual (o de la cuenta ranked) a la que pertenece.
     */
    contextId: string;
    bet: number;
    phase: RoundPhase;
    player: PlayerView;
    dealer: DealerView;
    allowed: AllowedActions;
    /**
     * El dealer tenía blackjack al revisar la oculta: la ronda terminó en el reparto.
     */
    peekBlackjack: boolean;
    wallet: WalletView;
    /**
     * Solo casual: composición restante por rango (0 = As … 9 = diez), con la
     * oculta contada como no vista.
     */
    shoeCounts?: [number, number, number, number, number, number, number, number, number, number];
    /**
     * Fracción del zapato ya repartida.
     */
    penetration: number;
    /**
     * Solo en `settled`.
     */
    result?: RoundResult;
}

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

/**
 * `GET /casual/leaderboard/weekly`.
 */
export interface WeeklyLeaderboard {
    /**
     * Lunes 00:00 UTC de la semana (RFC 3339).
     */
    weekStart: string;
    /**
     * Los 100 primeros.
     */
    top: LeaderboardRow[];
    /**
     * La fila del usuario, si tiene partidas esta semana (aunque esté en `top`).
     */
    me?: LeaderboardRow;
}

/**
 * `GET /me/records`.
 */
export interface MyRecords {
    casual: CasualRecords;
}

/**
 * `POST /casual/runs`: un preset o reglas personalizadas (exactamente uno).
 */
export interface CreateCasualRunRequest {
    preset?: Preset;
    rules?: Rules;
}

/**
 * `POST /play/rounds/{id}/actions`.
 */
export interface RoundActionRequest {
    action: Action;
    idempotencyKey: string;
}

/**
 * `POST /play/rounds`.
 */
export interface NewRoundRequest {
    kind: RoundKind;
    /**
     * Casual: id de la partida activa. Si falta, se usa la partida activa.
     */
    contextId?: string;
    bet: number;
    /**
     * UUID generado por el cliente; repetirlo devuelve la misma respuesta.
     */
    idempotencyKey: string;
}

export interface ActionResult {
    action: Action;
    ev: number;
    pWin: number;
    pPush: number;
    pLose: number;
    exact: boolean;
}

export interface AllowedActions {
    hit: boolean;
    stand: boolean;
    double: boolean;
    split: boolean;
    surrender: boolean;
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

export interface CasualRecords {
    runs: number;
    /**
     * Mejor pico de saldo en una partida.
     */
    bestPeak?: number;
    /**
     * Más manos jugadas en una partida.
     */
    mostHands?: number;
}

export interface CasualRunView {
    id: string;
    rules: Rules;
    preset?: Preset;
    status: CasualRunStatus;
    balance: number;
    /**
     * Saldo máximo alcanzado.
     */
    peak: number;
    /**
     * Manos (rondas) liquidadas.
     */
    hands: number;
    /**
     * Apuesta máxima para la próxima ronda (0 si la partida terminó).
     */
    maxBet: number;
    /**
     * Fracción del zapato ya repartida.
     */
    penetration: number;
    startedAt: string;
    endedAt?: string;
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

export interface DealerView {
    up: CardView;
    /**
     * `null` mientras no se revela.
     */
    hole: CardView | null;
    /**
     * Cartas que pidió el dealer, en orden, para animarlas una a una.
     */
    hits: CardView[];
}

export interface HandView {
    cards: CardView[];
    doubled: boolean;
    surrendered: boolean;
    fromSplit: boolean;
    /**
     * Solo en una ronda liquidada.
     */
    outcome?: HandOutcome;
    /**
     * Neto de esta mano en fichas (solo liquidada).
     */
    net?: number;
}

export interface LeaderboardRow {
    /**
     * Posición, desde 1. Empates comparten posición.
     */
    rank: number;
    displayName: string;
    /**
     * Casual semanal: el mejor pico de la semana.
     */
    value: number;
    isMe: boolean;
}

export interface PlayerView {
    hands: HandView[];
    /**
     * Índice de la mano que se está jugando.
     */
    active: number;
}

export interface ReviewItem {
    handIndex: number;
    chosen: Action;
    best: Action;
    evChosen: number;
    evBest: number;
    exact: boolean;
}

export interface RoundResult {
    /**
     * Neto de la ronda en fichas.
     */
    net: number;
    /**
     * Ranked: recarga aplicada en esta liquidación.
     */
    refilled?: number;
    review?: ReviewItem[];
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

export interface WalletView {
    balance: number;
    /**
     * Apuesta máxima para la próxima ronda (0 si el saldo no alcanza el mínimo).
     */
    maxBet: number;
    /**
     * Ranked: cuándo se puede volver a recargar.
     */
    refillAvailableAt?: string;
}

export type Action = "stand" | "hit" | "double" | "split" | "surrender";

export type CasualRunStatus = "active" | "retired" | "busted";

export type DoubleOn = "any" | "nineToEleven" | "tenToEleven";

export type HandOutcome = "blackjack" | "win" | "push" | "lose" | "bust" | "surrender";

export type Preset = "vegas" | "europe" | "sixToFive";

export type RoundKind = "casual" | "ranked";

export type Suit = "S" | "H" | "D" | "C";

export type Surrender = "none" | "late";
