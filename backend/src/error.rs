use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use shared::ApiError;

pub struct ApiErr {
    pub status: StatusCode,
    pub err: ApiError,
}

impl ApiErr {
    pub fn msg(status: StatusCode, msg: impl Into<String>) -> Self {
        Self { status, err: ApiError::new(msg) }
    }

    pub fn bad(msg: impl Into<String>) -> Self {
        Self::msg(StatusCode::BAD_REQUEST, msg)
    }

    pub fn not_found(msg: impl Into<String>) -> Self {
        Self::msg(StatusCode::NOT_FOUND, msg)
    }
}

impl IntoResponse for ApiErr {
    fn into_response(self) -> Response {
        (self.status, Json(self.err)).into_response()
    }
}

impl From<anyhow::Error> for ApiErr {
    fn from(e: anyhow::Error) -> Self {
        ApiErr::msg(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    }
}
