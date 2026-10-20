//! Extractores con errores en el formato de la API.

use axum::extract::{FromRequest, FromRequestParts};

use crate::error::ApiError;

/// Como `axum::Json`, pero un cuerpo inválido responde `{ code, message }`.
#[derive(FromRequest)]
#[from_request(via(axum::Json), rejection(ApiError))]
pub struct ApiJson<T>(pub T);

/// Como `axum::extract::Path`, con errores en el formato de la API.
#[derive(FromRequestParts)]
#[from_request(via(axum::extract::Path), rejection(ApiError))]
pub struct ApiPath<T>(pub T);
