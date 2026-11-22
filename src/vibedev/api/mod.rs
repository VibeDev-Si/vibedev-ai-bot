//! The internal configuration API, which the VibeDev integration manager's backend uses to set up the AI assistant
//! of a room (see the README's "VibeDev changes" section for the endpoints).
//!
//! - Only meant for a private network (it's off unless `vibedev.internal_api.listen_address` is set).
//! - Every request needs `Authorization: Bearer <secret>` (compared in constant time).
//! - JSON only, with limits on header and body sizes, and timeouts for reading requests and for handling them
//!   (callers wait up to 30 seconds; model services get 20).
//! - API keys are never returned and never logged: requests and responses aren't logged, and keys are registered for
//!   redaction from all logs (see `crate::vibedev::redact`). Errors never pass on what model services answered.

pub mod assistant;
mod backend;
pub mod models;
pub mod types;
pub mod upstream;

#[cfg(test)]
mod tests;

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use http_body_util::{BodyExt, Full, LengthLimitError, Limited};
use hyper::body::{Body, Bytes};
use hyper::header::{self, HeaderMap, HeaderValue};
use hyper::{Method, Request, Response, StatusCode, Uri};
use mxlink::matrix_sdk::ruma::OwnedRoomId;
use serde::Serialize;
use serde::de::DeserializeOwned;
use subtle::ConstantTimeEq;

use crate::Bot;
use crate::vibedev::config::Secret;
use crate::vibedev::redact;

use assistant::Backend;
use types::{ApiError, DeleteAssistantRequest};

/// The path prefix of all endpoints.
pub const PATH_PREFIX: &str = "/_vibedev/v1/";

/// The largest request body accepted.
pub const MAX_BODY_BYTES: usize = 64 * 1024;

/// The most bytes of request line and headers (hyper's minimum is 8 KiB).
const MAX_HEADER_BYTES: usize = 16 * 1024;

/// How long a client may take to send the request line and headers.
const HEADER_READ_TIMEOUT: Duration = Duration::from_secs(10);

/// How long a client may take to send the body.
const BODY_READ_TIMEOUT: Duration = Duration::from_secs(5);

/// How long handling a request may take: callers wait up to 30 seconds, and a configuration change includes
/// a ping of the model service of up to 20 seconds.
const HANDLER_TIMEOUT: Duration = Duration::from_secs(29);

/// How long a connection may stay open (keep-alive included).
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(120);

/// The most connections handled at once.
const MAX_CONNECTIONS: usize = 64;

/// What requests are handled with.
pub struct ApiState<B> {
    backend: B,
    secret: Secret,
}

impl<B> ApiState<B> {
    pub fn new(backend: B, secret: Secret) -> Self {
        Self { backend, secret }
    }
}

/// Starts listening (failing if the address can't be bound) and serves requests in the background.
pub async fn start(bot: Bot, address: SocketAddr, secret: Secret) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .map_err(|err| {
            anyhow::anyhow!("Failed to listen on {address} for the VibeDev internal API: {err}")
        })?;

    tracing::info!(%address, "The VibeDev internal API is listening");

    let state = Arc::new(ApiState::new(backend::BotBackend::new(bot), secret));

    tokio::spawn(serve(listener, state));

    Ok(())
}

/// Accepts connections and serves HTTP/1.1 on them.
pub async fn serve<B: Backend>(listener: tokio::net::TcpListener, state: Arc<ApiState<B>>) {
    let connections = Arc::new(tokio::sync::Semaphore::new(MAX_CONNECTIONS));

    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(accepted) => accepted,
            Err(err) => {
                tracing::warn!(?err, "Failed to accept a VibeDev internal API connection");
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };

        let Ok(permit) = connections.clone().try_acquire_owned() else {
            tracing::warn!(%peer, "Too many VibeDev internal API connections - closing a new one");
            continue;
        };

        let state = state.clone();

        tokio::spawn(async move {
            let _permit = permit;

            let service = hyper::service::service_fn(move |request| {
                let state = state.clone();
                async move { Ok::<_, Infallible>(handle(&state, request).await) }
            });

            let connection = hyper::server::conn::http1::Builder::new()
                .timer(hyper_util::rt::TokioTimer::new())
                .header_read_timeout(HEADER_READ_TIMEOUT)
                .max_buf_size(MAX_HEADER_BYTES)
                .serve_connection(hyper_util::rt::TokioIo::new(stream), service);

            match tokio::time::timeout(CONNECTION_TIMEOUT, connection).await {
                Ok(Ok(())) => {}
                Ok(Err(err)) => {
                    tracing::debug!(%peer, ?err, "VibeDev internal API connection error");
                }
                Err(_) => {
                    tracing::debug!(%peer, "Closing a VibeDev internal API connection which stayed open too long");
                }
            }
        });
    }
}

/// Handles a request, logging only its method, route, status and duration.
pub async fn handle<B, RequestBody>(
    state: &ApiState<B>,
    request: Request<RequestBody>,
) -> Response<Full<Bytes>>
where
    B: Backend,
    RequestBody: Body<Data = Bytes>,
    RequestBody::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    let started = Instant::now();
    let method = request.method().clone();
    let route = Route::parse(request.uri().path());
    let route_name = route.as_ref().map_or("unknown", Route::name);

    let response =
        match tokio::time::timeout(HANDLER_TIMEOUT, dispatch(state, request, route)).await {
            Ok(Ok(response)) => response,
            Ok(Err(err)) => error_response(err),
            Err(_) => error_response(ApiError::timeout()),
        };

    tracing::info!(
        %method,
        route = route_name,
        status = response.status().as_u16(),
        duration_ms = started.elapsed().as_millis() as u64,
        "VibeDev internal API request"
    );

    response
}

/// The endpoints.
#[derive(Debug, PartialEq)]
enum Route {
    Assistant(OwnedRoomId),
    Models,
}

impl Route {
    /// Parses a request path (unknown paths and invalid room ids are errors).
    fn parse(path: &str) -> Result<Self, ApiError> {
        let not_found =
            || ApiError::invalid_request_with_status(StatusCode::NOT_FOUND, "No such endpoint");

        let rest = path.strip_prefix(PATH_PREFIX).ok_or_else(not_found)?;

        if rest == "models" {
            return Ok(Self::Models);
        }

        let room_id = rest
            .strip_prefix("rooms/")
            .and_then(|rest| rest.strip_suffix("/assistant"))
            .filter(|room_id| !room_id.is_empty() && !room_id.contains('/'))
            .ok_or_else(not_found)?;

        let room_id = percent_decode(room_id).ok_or_else(|| {
            ApiError::invalid_request("The room id in the path is not properly percent-encoded")
        })?;

        Ok(Self::Assistant(assistant::parse_room_id(&room_id)?))
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Assistant(_) => "rooms/{room_id}/assistant",
            Self::Models => "models",
        }
    }
}

async fn dispatch<B, RequestBody>(
    state: &ApiState<B>,
    request: Request<RequestBody>,
    route: Result<Route, ApiError>,
) -> Result<Response<Full<Bytes>>, ApiError>
where
    B: Backend,
    RequestBody: Body<Data = Bytes>,
    RequestBody::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    // Before anything else, so that unauthenticated callers learn nothing (not even which paths exist).
    if !is_authorized(request.headers(), &state.secret) {
        return Err(ApiError::unauthorized());
    }

    let route = route?;
    let method = request.method().clone();
    let backend = &state.backend;

    match (&route, &method) {
        (Route::Assistant(room_id), &Method::GET) => {
            let acting_user = query_param(request.uri(), "acting_user")?.ok_or_else(|| {
                ApiError::invalid_request("The acting_user query parameter is required")
            })?;

            json(
                StatusCode::OK,
                &assistant::get(backend, room_id, &acting_user).await?,
            )
        }
        (Route::Assistant(room_id), &Method::PUT) => {
            let body = read_json(request).await?;
            json(
                StatusCode::OK,
                &assistant::put(backend, room_id, body).await?,
            )
        }
        (Route::Assistant(room_id), &Method::DELETE) => {
            let from_query = query_param(request.uri(), "acting_user")?;
            let from_body = read_optional_json::<DeleteAssistantRequest, _>(request)
                .await?
                .unwrap_or_default()
                .acting_user;

            let acting_user = match (from_body, from_query) {
                (Some(from_body), Some(from_query)) if from_body.trim() != from_query.trim() => {
                    return Err(ApiError::invalid_request(
                        "acting_user differs between the body and the query string",
                    ));
                }
                (Some(acting_user), _) | (None, Some(acting_user)) => acting_user,
                (None, None) => {
                    return Err(ApiError::invalid_request(
                        "acting_user is required (in the body or the query string)",
                    ));
                }
            };

            json(
                StatusCode::OK,
                &assistant::delete(backend, room_id, &acting_user).await?,
            )
        }
        (Route::Models, &Method::POST) => {
            let body = read_json(request).await?;
            json(StatusCode::OK, &assistant::models(backend, body).await?)
        }
        (Route::Assistant(_), _) => Err(method_not_allowed("GET, PUT, DELETE")),
        (Route::Models, _) => Err(method_not_allowed("POST")),
    }
}

fn method_not_allowed(allowed: &str) -> ApiError {
    ApiError::invalid_request_with_status(
        StatusCode::METHOD_NOT_ALLOWED,
        format!("Allowed methods: {allowed}"),
    )
}

/// Checks the `Authorization: Bearer <secret>` header, comparing the secret in constant time.
fn is_authorized(headers: &HeaderMap, secret: &Secret) -> bool {
    let mut values = headers.get_all(header::AUTHORIZATION).iter();

    let (Some(value), None) = (values.next(), values.next()) else {
        return false;
    };

    let value = value.as_bytes();

    // The scheme is case-insensitive.
    let Some(token) = value
        .get(..7)
        .filter(|scheme| scheme.eq_ignore_ascii_case(b"bearer "))
        .and_then(|_| value.get(7..))
    else {
        return false;
    };

    let expected = secret.expose().as_bytes();

    !expected.is_empty() && bool::from(token.ct_eq(expected))
}

/// Reads a query string parameter (form-encoded). Repeating it is an error.
fn query_param(uri: &Uri, name: &str) -> Result<Option<String>, ApiError> {
    let Some(query) = uri.query() else {
        return Ok(None);
    };

    let mut values = url::form_urlencoded::parse(query.as_bytes())
        .filter(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned());

    let value = values.next();

    if values.next().is_some() {
        return Err(ApiError::invalid_request(format!(
            "The {name} query parameter is repeated"
        )));
    }

    Ok(value)
}

/// Reads a required JSON body into the request type.
async fn read_json<T, RequestBody>(request: Request<RequestBody>) -> Result<T, ApiError>
where
    T: DeserializeOwned,
    RequestBody: Body<Data = Bytes>,
    RequestBody::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    read_optional_json(request)
        .await?
        .ok_or_else(|| ApiError::invalid_request("A JSON body is required"))
}

/// Reads a JSON body (up to [`MAX_BODY_BYTES`], within [`BODY_READ_TIMEOUT`]) into the request type,
/// or `None` if there's no body.
///
/// Parsing errors don't repeat the body: the value of `api_key` (if any) is redacted from them.
async fn read_optional_json<T, RequestBody>(
    request: Request<RequestBody>,
) -> Result<Option<T>, ApiError>
where
    T: DeserializeOwned,
    RequestBody: Body<Data = Bytes>,
    RequestBody::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    let headers = request.headers();

    let is_json = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("application/json"));

    let too_large = || {
        ApiError::invalid_request_with_status(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!("The body must be at most {MAX_BODY_BYTES} bytes"),
        )
    };

    let declared_length = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());

    if declared_length.is_some_and(|length| length > MAX_BODY_BYTES as u64) {
        return Err(too_large());
    }

    let body = Limited::new(request.into_body(), MAX_BODY_BYTES);

    let bytes = match tokio::time::timeout(BODY_READ_TIMEOUT, body.collect()).await {
        Ok(Ok(collected)) => collected.to_bytes(),
        Ok(Err(err)) if err.downcast_ref::<LengthLimitError>().is_some() => {
            return Err(too_large());
        }
        Ok(Err(_)) => return Err(ApiError::invalid_request("Failed to read the body")),
        Err(_) => {
            return Err(ApiError::invalid_request_with_status(
                StatusCode::REQUEST_TIMEOUT,
                "The body took too long to arrive",
            ));
        }
    };

    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Ok(None);
    }

    if !is_json {
        return Err(ApiError::invalid_request_with_status(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "The body must be JSON (Content-Type: application/json)",
        ));
    }

    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|err| {
        ApiError::invalid_request(format!(
            "The body is not valid JSON (line {}, column {})",
            err.line(),
            err.column()
        ))
    })?;

    let api_key = value
        .get("api_key")
        .and_then(|api_key| api_key.as_str())
        .unwrap_or_default()
        .to_owned();

    serde_json::from_value(value).map(Some).map_err(|err| {
        ApiError::invalid_request(redact::sanitize_message(
            &format!("Invalid body: {err}"),
            &[api_key.as_str(), api_key.trim()],
            300,
        ))
    })
}

/// Serializes a JSON response, with headers keeping it out of caches.
fn json(status: StatusCode, body: &impl Serialize) -> Result<Response<Full<Bytes>>, ApiError> {
    let body = serde_json::to_vec(body)
        .map_err(|_| ApiError::internal("Failed to serialize the response"))?;

    let mut response = Response::new(Full::new(Bytes::from(body)));
    *response.status_mut() = status;

    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );

    Ok(response)
}

fn error_response(err: ApiError) -> Response<Full<Bytes>> {
    let status = err.status;

    let mut response = json(status, &err).unwrap_or_else(|_| {
        let mut response = Response::new(Full::new(Bytes::from_static(b"{}")));
        *response.status_mut() = status;
        response
    });

    if status == StatusCode::UNAUTHORIZED {
        response
            .headers_mut()
            .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
    }

    response
}

/// Decodes `%XX` escapes (a room id in a path may be percent-encoded, e.g. `%21abc%3Aexample.com`).
fn percent_decode(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = bytes.get(index + 1..index + 3)?;
            if !hex.iter().all(u8::is_ascii_hexdigit) {
                return None;
            }
            decoded.push(u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }

    String::from_utf8(decoded).ok()
}
