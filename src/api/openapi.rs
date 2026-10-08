//! Generated REST contract. Route registration and Rust wire types are authoritative.
use super::cache::CachePolicy;
use crate::{Error, ErrorBody, ErrorCode, model::Envelope};
use aide::{
    OperationInput, OperationOutput,
    generate::GenContext,
    openapi::{OpenApi, Operation, Response as ApiDocResponse, StatusCode},
    operation::{ParamLocation, parameters_from_schema},
};
use axum::{
    Json,
    extract::{FromRequestParts, Query, rejection::QueryRejection},
    http::{header::CACHE_CONTROL, request::Parts},
    response::{IntoResponse, Response},
};
use rmcp::schemars::{JsonSchema, generate::SchemaSettings};
use serde::{Serialize, de::DeserializeOwned};

/// Keep malformed queries in the shared error envelope rather than Axum's text rejection.
pub(super) struct ApiQuery<T>(pub Result<Query<T>, QueryRejection>);
impl<S: Send + Sync, T: DeserializeOwned> FromRequestParts<S> for ApiQuery<T> {
    type Rejection = std::convert::Infallible;
    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        Ok(Self(Query::<T>::from_request_parts(parts, state).await))
    }
}
impl<T: JsonSchema> OperationInput for ApiQuery<T> {
    fn operation_input(ctx: &mut GenContext, operation: &mut Operation) {
        // Requests use deserialization semantics; responses use serialization semantics.
        let schema = SchemaSettings::draft2020_12()
            .with(|s| s.inline_subschemas = true)
            .into_generator()
            .into_root_schema_for::<T>();
        operation.parameters.extend(
            parameters_from_schema(ctx, schema, ParamLocation::Query)
                .into_iter()
                .map(aide::openapi::ReferenceOr::Item),
        );
    }
}
/// Carries the concrete payload type through runtime rendering and operation inference.
pub(super) struct ApiResponse<T> {
    pub result: Result<T, Error>,
    pub cache: CachePolicy,
}
impl<T: Serialize> IntoResponse for ApiResponse<T> {
    fn into_response(self) -> Response {
        match self.result {
            Ok(data) => (
                [(CACHE_CONTROL, self.cache.header_value())],
                Json(crate::envelope(data)),
            )
                .into_response(),
            Err(error) => ([(CACHE_CONTROL, self.cache.header_value())], error).into_response(),
        }
    }
}
impl<T: JsonSchema> OperationOutput for ApiResponse<T> {
    type Inner = Envelope<T>;
    fn operation_response(ctx: &mut GenContext, op: &mut Operation) -> Option<ApiDocResponse> {
        Json::<Envelope<T>>::operation_response(ctx, op)
    }
    fn inferred_responses(
        ctx: &mut GenContext,
        op: &mut Operation,
    ) -> Vec<(Option<StatusCode>, ApiDocResponse)> {
        Json::<Envelope<T>>::inferred_responses(ctx, op)
    }
}
impl OperationOutput for Error {
    type Inner = ErrorBody;
    fn operation_response(ctx: &mut GenContext, op: &mut Operation) -> Option<ApiDocResponse> {
        Json::<ErrorBody>::operation_response(ctx, op)
    }
}
pub(super) fn success_description(op: &mut Operation, description: &str) {
    let response = op
        .responses
        .as_mut()
        .expect("typed responses")
        .responses
        .get_mut(&StatusCode::Code(200))
        .expect("typed success");
    if let aide::openapi::ReferenceOr::Item(response) = response {
        response.description = description.into();
    }
}
/// Explicit service/middleware statuses with the typed shared error body.
pub(super) fn errors(op: &mut Operation, codes: &[ErrorCode]) {
    let too_large = format!(
        "Request body exceeds {} (REQUEST_TOO_LARGE)",
        super::max_request_body_text()
    );
    aide::generate::in_context(|ctx| {
        for code in codes {
            let mut response = Error::operation_response(ctx, op).expect("JSON error schema");
            response.description = match code {
                ErrorCode::InvalidLocation => "Invalid query, location or request body (INVALID_LOCATION)",
                ErrorCode::Forbidden => "Request did not come through the public address (FORBIDDEN). Only when the server is configured with an origin-verify value.",
                ErrorCode::RequestTooLarge => too_large.as_str(),
                ErrorCode::CityNotFound => "City not found (CITY_NOT_FOUND); suggestions in choices when available",
                ErrorCode::AmbiguousCity => "Ambiguous city (AMBIGUOUS_CITY); select one of choices",
                ErrorCode::OutsideCoverage => "Outside NWS coverage (OUTSIDE_COVERAGE)",
                ErrorCode::UpstreamUnavailable => "NWS unavailable (UPSTREAM_UNAVAILABLE)",
                ErrorCode::Busy => "Too many in-flight requests (BUSY)",
                ErrorCode::UpstreamTimeout => "Weather lookup or HTTP request deadline exceeded (UPSTREAM_TIMEOUT)",
            }.into();
            op.responses
                .get_or_insert_with(Default::default)
                .responses
                .insert(
                    StatusCode::Code(code.http_status()),
                    aide::openapi::ReferenceOr::Item(response),
                );
        }
    });
}
/// Generate without Weather initialization, network requests, or a listening socket.
pub fn openapi() -> OpenApi {
    let (_, spec) = super::routes::rest_router();
    spec
}
/// Stable sorted JSON artifact, including a final newline for checked-in files.
pub fn openapi_json() -> String {
    let value = serde_json::to_value(openapi()).expect("OpenAPI serializes");
    format!(
        "{}\n",
        serde_json::to_string_pretty(&value).expect("OpenAPI serializes")
    )
}
pub(super) fn initialize() {
    aide::generate::reset_context();
    aide::generate::on_error(|error| panic!("OpenAPI generation failed: {error}"));
    aide::generate::in_context(|ctx| {
        ctx.schema = SchemaSettings::draft2020_12()
            .for_serialize()
            .with(|s| {
                s.definitions_path = "#/components/schemas/".into();
            })
            .into_generator();
    });
}
