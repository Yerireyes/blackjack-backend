//! Wrappers `#[wasm_bindgen]` sobre el motor, sin lógica propia.
//! `computeMs` se mide aquí, porque el motor no usa reloj.

use engine::{AnalyzeDealerRequest, AnalyzeHandRequest, AnalyzeRoundRequest};
use serde::Serialize;
use wasm_bindgen::prelude::*;

fn to_js<T: Serialize>(value: &T) -> Result<JsValue, JsError> {
    // Structs como objetos planos (no Map) para que coincidan con los tipos TS.
    let ser = serde_wasm_bindgen::Serializer::json_compatible();
    Ok(value.serialize(&ser)?)
}

fn err(e: engine::EngineError) -> JsError {
    JsError::new(&e.to_string())
}

#[wasm_bindgen(js_name = analyzeHand, unchecked_return_type = "AnalyzeHandResponse")]
pub fn analyze_hand(
    #[wasm_bindgen(unchecked_param_type = "AnalyzeHandRequest")] req: JsValue,
) -> Result<JsValue, JsError> {
    let req: AnalyzeHandRequest = serde_wasm_bindgen::from_value(req)?;
    let t0 = js_sys::Date::now();
    let mut res = engine::analyze_hand(req).map_err(err)?;
    res.compute_ms = js_sys::Date::now() - t0;
    to_js(&res)
}

#[wasm_bindgen(js_name = analyzeDealer, unchecked_return_type = "AnalyzeDealerResponse")]
pub fn analyze_dealer(
    #[wasm_bindgen(unchecked_param_type = "AnalyzeDealerRequest")] req: JsValue,
) -> Result<JsValue, JsError> {
    let req: AnalyzeDealerRequest = serde_wasm_bindgen::from_value(req)?;
    let t0 = js_sys::Date::now();
    let mut res = engine::analyze_dealer(req).map_err(err)?;
    res.compute_ms = js_sys::Date::now() - t0;
    to_js(&res)
}

#[wasm_bindgen(js_name = analyzeRound, unchecked_return_type = "AnalyzeRoundResponse")]
pub fn analyze_round(
    #[wasm_bindgen(unchecked_param_type = "AnalyzeRoundRequest")] req: JsValue,
) -> Result<JsValue, JsError> {
    let req: AnalyzeRoundRequest = serde_wasm_bindgen::from_value(req)?;
    let t0 = js_sys::Date::now();
    let mut res = engine::analyze_round(req).map_err(err)?;
    let ms = js_sys::Date::now() - t0;
    res.compute_ms = ms;
    for h in &mut res.hands {
        h.compute_ms = ms;
    }
    to_js(&res)
}
