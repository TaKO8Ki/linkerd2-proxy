use deflate::write::GzEncoder;
use deflate::CompressionOptions;
use futures::future;
use http::{self, header, StatusCode};
use hyper::{service::Service, Body, Request, Response};
use std::error::Error;
use std::fmt;
use std::io::{self, Write};
use std::task::{Context, Poll};
use tracing::{error, trace};

use super::FmtMetrics;

/// Serve Prometheues metrics.
#[derive(Debug, Clone)]
pub struct Serve<M: FmtMetrics> {
    metrics: M,
}

#[derive(Debug)]
enum ServeError {
    Http(http::Error),
    Io(io::Error),
}

// ===== impl Serve =====

impl<M: FmtMetrics> Serve<M> {
    pub fn new(metrics: M) -> Self {
        Self { metrics }
    }

    fn is_gzip<B>(req: &Request<B>) -> bool {
        req.headers()
            .get_all(header::ACCEPT_ENCODING)
            .iter()
            .any(|value| {
                value
                    .to_str()
                    .ok()
                    .map(|value| value.contains("gzip"))
                    .unwrap_or(false)
            })
    }
}

impl<M: FmtMetrics> Service<Request<Body>> for Serve<M> {
    type Response = Response<Body>;
    type Error = io::Error;
    type Future = future::Ready<Result<Response<Body>, Self::Error>>;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        if req.uri().path() != "/metrics" {
            let rsp = Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::empty())
                .expect("builder with known status code should not fail");
            return future::ok(rsp);
        }

        let resp = if Self::is_gzip(&req) {
            trace!("gzipping metrics");
            let mut writer = GzEncoder::new(Vec::<u8>::new(), CompressionOptions::fast());
            write!(&mut writer, "{}", self.metrics.as_display())
                .and_then(|_| writer.finish())
                .map_err(ServeError::from)
                .and_then(|body| {
                    Response::builder()
                        .header(header::CONTENT_ENCODING, "gzip")
                        .header(header::CONTENT_TYPE, "text/plain")
                        .body(Body::from(body))
                        .map_err(ServeError::from)
                })
        } else {
            let mut writer = Vec::<u8>::new();
            write!(&mut writer, "{}", self.metrics.as_display())
                .map_err(ServeError::from)
                .and_then(|_| {
                    Response::builder()
                        .header(header::CONTENT_TYPE, "text/plain")
                        .body(Body::from(writer))
                        .map_err(ServeError::from)
                })
        };

        let resp = resp.unwrap_or_else(|e| {
            error!("{}", e);
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::empty())
                .expect("builder with known status code should not fail")
        });
        future::ok(resp)
    }
}

// ===== impl ServeError =====

impl From<http::Error> for ServeError {
    fn from(err: http::Error) -> Self {
        ServeError::Http(err)
    }
}

impl From<io::Error> for ServeError {
    fn from(err: io::Error) -> Self {
        ServeError::Io(err)
    }
}

impl fmt::Display for ServeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            ServeError::Http(_) => "error constructing HTTP response".fmt(f),
            ServeError::Io(_) => "error writing metrics".fmt(f),
        }
    }
}

impl Error for ServeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match *self {
            ServeError::Http(ref source) => Some(source),
            ServeError::Io(ref source) => Some(source),
        }
    }
}
