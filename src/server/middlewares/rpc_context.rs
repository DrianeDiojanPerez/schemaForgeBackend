use std::task::{Context, Poll};
use std::time::Instant;

use axum::http::{HeaderName, HeaderValue, Request, Response};
use tower::{Layer, Service};
use tracing::Instrument;
use uuid::Uuid;

const REQUEST_ID: HeaderName = HeaderName::from_static("x-request-id");
const GRPC_STATUS: HeaderName = HeaderName::from_static("grpc-status");

/// Only the codes that mean the server broke deserve an error line. A rejected
/// payload is the caller's mistake, and logging it at error turns every typo in
/// a request into something that looks like an outage.
fn is_server_fault(code: i32) -> bool {
    matches!(code, 2 | 13 | 14 | 15)
}

/// Stamps every call with a request id and opens a tracing span around it, so
/// a line in the log file can be traced back to the call that produced it.
/// An id the caller supplied is kept rather than replaced, which is what lets
/// a frontend server function correlate its own log line with this one.
#[derive(Clone, Default)]
pub struct RequestContextLayer;

impl<S> Layer<S> for RequestContextLayer {
    type Service = RequestContext<S>;

    fn layer(&self, inner: S) -> Self::Service {
        RequestContext { inner }
    }
}

#[derive(Clone)]
pub struct RequestContext<S> {
    inner: S,
}

impl<S, ReqBody, ResBody> Service<Request<ReqBody>> for RequestContext<S>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>> + Clone + Send + 'static,
    S::Future: Send + 'static,
    ReqBody: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>,
    >;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut request: Request<ReqBody>) -> Self::Future {
        let request_id = request
            .headers()
            .get(&REQUEST_ID)
            .and_then(|value| value.to_str().ok())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| Uuid::new_v4().to_string());

        if let Ok(value) = HeaderValue::from_str(&request_id) {
            request.headers_mut().insert(REQUEST_ID, value.clone());
        }

        let span = tracing::info_span!(
            "rpc",
            request_id = %request_id,
            path = %request.uri().path(),
        );

        // The clone dance is the standard tower workaround: `self.inner` is the
        // instance that was polled ready, so the ready one is what gets called.
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);

        Box::pin(
            async move {
                tracing::debug!(metadata = ?request.headers(), "rpc started");

                let started = Instant::now();
                let mut response = inner.call(request).await?;
                let latency_ms = started.elapsed().as_secs_f64() * 1_000.0;

                // A failed unary call comes back as a trailers-only response,
                // which puts the status in the headers. A successful one keeps
                // it in the trailers, which a layer this far out never sees.
                let code = response
                    .headers()
                    .get(&GRPC_STATUS)
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.parse::<i32>().ok());

                match code {
                    Some(code) if is_server_fault(code) => {
                        tracing::error!(grpc_status = code, latency_ms, "rpc failed");
                    }
                    Some(code) if code != 0 => {
                        tracing::warn!(grpc_status = code, latency_ms, "rpc rejected");
                    }
                    _ => tracing::debug!(latency_ms, "rpc completed"),
                }

                if let Ok(value) = HeaderValue::from_str(&request_id) {
                    response.headers_mut().insert(REQUEST_ID, value);
                }

                Ok(response)
            }
            // `enter()` would not survive the await points; `instrument` is
            // what keeps the span attached across them.
            .instrument(span),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_servers_own_failures_are_errors() {
        // Internal, Unavailable, Unknown and DataLoss.
        assert!(is_server_fault(13));
        assert!(is_server_fault(14));
        assert!(is_server_fault(2));
        assert!(is_server_fault(15));

        // InvalidArgument, NotFound, AlreadyExists, PermissionDenied and the
        // two milestones that answer Unimplemented on purpose.
        assert!(!is_server_fault(3));
        assert!(!is_server_fault(5));
        assert!(!is_server_fault(6));
        assert!(!is_server_fault(7));
        assert!(!is_server_fault(12));
    }
}
