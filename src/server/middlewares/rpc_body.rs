//! Logs the message going in and the message coming out of every call, at
//! debug level, so the request lines carry the payload and not only the
//! metadata. Protobuf is binary on the wire, so each message is decoded with
//! the same descriptor set the reflection service serves and written as JSON.
//! Credentials and tokens are blanked before the line is written.

use std::pin::Pin;
use std::sync::Arc;
use std::task::{ready, Context, Poll};

use axum::http::{Request, Response};
use bytes::{Buf, Bytes, BytesMut};
use http_body::{Body as HttpBody, Frame, SizeHint};
use prost_reflect::{
    DescriptorError, DescriptorPool, DynamicMessage, Kind, MessageDescriptor, MethodDescriptor,
    ReflectMessage, Value,
};
use tonic::body::Body;
use tower::{Layer, Service};
use tracing::Span;

const SENSITIVE: [&str; 3] = ["password", "token", "secret"];
const REDACTED: &str = "[redacted]";
const FRAME_PREFIX: usize = 5;

#[derive(Clone)]
pub struct BodyDumpLayer {
    pool: Arc<DescriptorPool>,
}

impl BodyDumpLayer {
    pub fn new(descriptors: &[u8]) -> Result<Self, DescriptorError> {
        Ok(Self {
            pool: Arc::new(DescriptorPool::decode(descriptors)?),
        })
    }
}

impl<S> Layer<S> for BodyDumpLayer {
    type Service = BodyDump<S>;

    fn layer(&self, inner: S) -> Self::Service {
        BodyDump {
            inner,
            pool: self.pool.clone(),
        }
    }
}

#[derive(Clone)]
pub struct BodyDump<S> {
    inner: S,
    pool: Arc<DescriptorPool>,
}

impl<S> Service<Request<Body>> for BodyDump<S>
where
    S: Service<Request<Body>, Response = Response<Body>> + Clone + Send + 'static,
    S::Future: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future =
        Pin<Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: Request<Body>) -> Self::Future {
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);

        let method = tracing::enabled!(tracing::Level::DEBUG)
            .then(|| method_for(&self.pool, request.uri().path()))
            .flatten();

        let Some(method) = method else {
            return Box::pin(inner.call(request));
        };

        let span = Span::current();
        let (parts, body) = request.into_parts();
        let body = Dump::wrap(body, method.input(), "rpc request", span.clone());
        let request = Request::from_parts(parts, body);
        let output = method.output();

        Box::pin(async move {
            let response = inner.call(request).await?;
            let (parts, body) = response.into_parts();
            let body = Dump::wrap(body, output, "rpc response", span);

            Ok(Response::from_parts(parts, body))
        })
    }
}

/// The path is `/package.Service/Method`, which is enough to find the
/// descriptor that says what the two messages look like.
fn method_for(pool: &DescriptorPool, path: &str) -> Option<MethodDescriptor> {
    let (service, method) = path.strip_prefix('/')?.split_once('/')?;

    pool.get_service_by_name(service)?
        .methods()
        .find(|candidate| candidate.name() == method)
}

/// Copies every data frame as it passes and writes one log line once the
/// stream is over, so the caller and the handler see the body exactly as
/// they would have without the layer.
struct Dump {
    inner: Body,
    buffer: BytesMut,
    descriptor: MessageDescriptor,
    message: &'static str,
    span: Span,
    logged: bool,
}

impl Dump {
    fn wrap(inner: Body, descriptor: MessageDescriptor, message: &'static str, span: Span) -> Body {
        Body::new(Self {
            inner,
            buffer: BytesMut::new(),
            descriptor,
            message,
            span,
            logged: false,
        })
    }

    fn log(&mut self) {
        if self.logged {
            return;
        }

        self.logged = true;

        let _guard = self.span.enter();
        let bytes = self.buffer.len();

        match render(&self.descriptor, &self.buffer) {
            Ok(body) => tracing::debug!(bytes, body = %body, "{}", self.message),
            Err(reason) => tracing::debug!(bytes, reason = %reason, "{}", self.message),
        }
    }
}

impl HttpBody for Dump {
    type Data = Bytes;
    type Error = <Body as HttpBody>::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let this = self.get_mut();
        let frame = ready!(Pin::new(&mut this.inner).poll_frame(cx));

        if let Some(Ok(frame)) = &frame {
            if let Some(data) = frame.data_ref() {
                this.buffer.extend_from_slice(data);
            }
        }

        if frame.is_none() || this.inner.is_end_stream() {
            this.log();
        }

        Poll::Ready(frame)
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

impl Drop for Dump {
    fn drop(&mut self) {
        if !self.buffer.is_empty() {
            self.log();
        }
    }
}

/// A body is a run of frames, each a flag byte, a big endian length and the
/// message. Every call here is unary, so one frame is the norm, but a run is
/// rendered as a list rather than silently truncated.
fn render(descriptor: &MessageDescriptor, mut body: &[u8]) -> Result<String, String> {
    let mut messages = Vec::new();

    while !body.is_empty() {
        if body.len() < FRAME_PREFIX {
            return Err("truncated frame".to_owned());
        }

        let compressed = body.get_u8() != 0;
        let length = body.get_u32() as usize;

        if body.len() < length {
            return Err("truncated message".to_owned());
        }

        let (payload, rest) = body.split_at(length);
        body = rest;

        if compressed {
            return Err("compressed message".to_owned());
        }

        let mut message =
            DynamicMessage::decode(descriptor.clone(), payload).map_err(|err| err.to_string())?;
        redact(&mut message);
        messages.push(message);
    }

    let json = match messages.as_slice() {
        [single] => serde_json::to_string(single),
        many => serde_json::to_string(many),
    };

    json.map_err(|err| err.to_string())
}

fn is_sensitive(name: &str) -> bool {
    SENSITIVE.iter().any(|word| name.contains(word))
}

fn redact(message: &mut DynamicMessage) {
    let fields: Vec<_> = message.descriptor().fields().collect();

    for field in fields {
        if !message.has_field(&field) {
            continue;
        }

        if is_sensitive(field.name()) {
            let blank = Value::String(REDACTED.to_owned());

            if message.try_set_field(&field, blank).is_err() {
                message.clear_field(&field);
            }

            continue;
        }

        if !matches!(field.kind(), Kind::Message(_)) {
            continue;
        }

        match message.get_field_mut(&field) {
            Value::Message(inner) => redact(inner),
            Value::List(items) => {
                for item in items {
                    if let Some(inner) = item.as_message_mut() {
                        redact(inner);
                    }
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use http_body_util::{BodyExt, Full};
    use prost::Message;

    use crate::rpc::v1;

    fn pool() -> DescriptorPool {
        DescriptorPool::decode(v1::FILE_DESCRIPTOR_SET).expect("the descriptors should decode")
    }

    fn framed(message: &impl Message) -> Vec<u8> {
        let payload = message.encode_to_vec();
        let mut frame = vec![0];
        frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        frame.extend_from_slice(&payload);
        frame
    }

    #[test]
    fn the_path_names_the_method() {
        let method = method_for(&pool(), "/schemaforge.v1.AuthService/Login")
            .expect("login should be known");

        assert_eq!(method.input().full_name(), "schemaforge.v1.LoginRequest");
        assert_eq!(method.output().full_name(), "schemaforge.v1.LoginResponse");
    }

    #[test]
    fn a_path_off_the_contract_has_no_method() {
        assert!(method_for(&pool(), "/schemaforge.v1.AuthService/Nope").is_none());
        assert!(method_for(&pool(), "/grpc.reflection.v1.ServerReflection/Info").is_none());
        assert!(method_for(&pool(), "nonsense").is_none());
    }

    #[test]
    fn a_message_renders_as_json() {
        let pool = pool();
        let descriptor = pool
            .get_message_by_name("schemaforge.v1.CheckResponse")
            .expect("the check response should be known");
        let body = framed(&v1::CheckResponse {
            status: "OK".to_owned(),
            version: "0.1.0".to_owned(),
        });

        let json = render(&descriptor, &body).expect("the body should render");

        assert_eq!(json, r#"{"status":"OK","version":"0.1.0"}"#);
    }

    #[test]
    fn credentials_never_reach_the_log() {
        let pool = pool();
        let descriptor = pool
            .get_message_by_name("schemaforge.v1.LoginRequest")
            .expect("the login request should be known");
        let body = framed(&v1::LoginRequest {
            email: "sam@example.com".to_owned(),
            password: "hunter2".to_owned(),
        });

        let json = render(&descriptor, &body).expect("the body should render");

        assert!(json.contains("sam@example.com"));
        assert!(json.contains(REDACTED));
        assert!(!json.contains("hunter2"));
    }

    #[test]
    fn a_half_frame_is_reported_not_guessed() {
        let pool = pool();
        let descriptor = pool
            .get_message_by_name("schemaforge.v1.CheckRequest")
            .expect("the check request should be known");

        assert_eq!(
            render(&descriptor, &[0, 0]),
            Err("truncated frame".to_owned())
        );
        assert_eq!(
            render(&descriptor, &[0, 0, 0, 0, 9, 1]),
            Err("truncated message".to_owned())
        );
    }

    #[tokio::test]
    async fn the_body_passes_through_untouched() {
        let pool = pool();
        let descriptor = pool
            .get_message_by_name("schemaforge.v1.CheckResponse")
            .expect("the check response should be known");
        let bytes = framed(&v1::CheckResponse {
            status: "OK".to_owned(),
            version: "0.1.0".to_owned(),
        });
        let inner = Body::new(Full::new(Bytes::from(bytes.clone())));

        let body = Dump::wrap(inner, descriptor, "rpc response", Span::none());
        let collected = body.collect().await.expect("the body should collect");

        assert_eq!(collected.to_bytes().as_ref(), bytes.as_slice());
    }
}
