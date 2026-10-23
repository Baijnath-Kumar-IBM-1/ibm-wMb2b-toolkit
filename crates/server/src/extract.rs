//! Extractors that fail with the API error shape and never echo input.
//!
//! Stock axum rejections include serde messages, which can repeat submitted
//! values (for example a secret sent with the wrong type). These do not.

use std::str::FromStr;

use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{FromRequest, FromRequestParts, Path, Query, Request};
use axum::http::StatusCode;
use axum::http::request::Parts;
use serde::de::DeserializeOwned;
use trace_core::Error as CoreError;

use crate::error::{AppError, core_err};

pub struct ApiJson<T>(pub T);

impl<S, T> FromRequest<S> for ApiJson<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = AppError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        match axum::Json::<T>::from_request(req, state).await {
            Ok(axum::Json(value)) => Ok(Self(value)),
            Err(rejection) => Err(json_rejection(&rejection)),
        }
    }
}

fn json_rejection(rejection: &JsonRejection) -> AppError {
    let status = rejection.status();
    let message = match status {
        StatusCode::UNSUPPORTED_MEDIA_TYPE => "expected a JSON body",
        StatusCode::PAYLOAD_TOO_LARGE => "request body is too large",
        _ => "invalid request body",
    };
    AppError::new(status, message)
}

pub struct ApiQuery<T>(pub T);

impl<S, T> FromRequestParts<S> for ApiQuery<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        match Query::<T>::from_request_parts(parts, state).await {
            Ok(Query(value)) => Ok(Self(value)),
            Err(rejection) => Err(query_rejection(&rejection)),
        }
    }
}

fn query_rejection(_: &QueryRejection) -> AppError {
    AppError::new(StatusCode::BAD_REQUEST, "invalid query parameters")
}

/// A single typed id taken from the path (for example an `EnvironmentId`).
pub struct PathId<T>(pub T);

impl<S, T> FromRequestParts<S> for PathId<T>
where
    S: Send + Sync,
    T: FromStr<Err = CoreError>,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Path(raw) = Path::<String>::from_request_parts(parts, state)
            .await
            .map_err(|_| AppError::new(StatusCode::BAD_REQUEST, "invalid path"))?;
        T::from_str(&raw).map(Self).map_err(core_err)
    }
}
