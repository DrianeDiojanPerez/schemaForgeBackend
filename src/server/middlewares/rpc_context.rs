use std::task::{Context, Poll};

use axum::http::{HeaderName, HeaderValue, Request, Response};
use tower::{Layer, Service};
use tracing::Instrument;
use uuid::Uuid;

const REQUEST_ID: HeaderName = HeaderName::from_static("x-request-id");

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
                let mut response = inner.call(request).await?;

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
