//! HTTP transport for component plugins with explicit URL and resource limits.

use std::time::{Duration, Instant};

use plugin_system::{PluginHostGrants, PluginManifest, PluginRuntimeError};
use wasmtime::component::{Linker, Val};

use super::WasmStoreState;

const DEFAULT_FINISH_RESERVE_MS: u64 = 30;

#[derive(Clone)]
pub(crate) struct HttpClient {
    agent: ureq::Agent,
    allowed_urls: Vec<String>,
    timeout: Duration,
    finish_reserve: Duration,
    request_max_bytes: usize,
    response_max_bytes: u64,
    call_deadline: Option<Instant>,
}

impl HttpClient {
    pub(crate) fn load(
        manifest: &PluginManifest,
        grants: &PluginHostGrants,
    ) -> Result<Option<Self>, PluginRuntimeError> {
        if !grants.can_request_http() {
            return Ok(None);
        }
        let config = manifest
            .host
            .http
            .as_ref()
            .ok_or_else(|| Self::load_error("http.request requires [host.http]"))?;
        if config.allowed_urls.is_empty() {
            return Err(Self::load_error("HTTP allowed_urls must not be empty"));
        }
        for url in &config.allowed_urls {
            let uri = url
                .parse::<ureq::http::Uri>()
                .map_err(|error| Self::load_error(format!("invalid HTTP URL: {error}")))?;
            if !matches!(uri.scheme_str(), Some("http" | "https"))
                || uri.host().is_none()
                || uri
                    .authority()
                    .is_some_and(|authority| authority.as_str().contains('@'))
                || url.contains('#')
            {
                return Err(Self::load_error(
                    "HTTP allowed_urls require absolute HTTP or HTTPS URLs without userinfo or fragments",
                ));
            }
        }
        if config.timeout_ms == 0 || config.request_max_bytes == 0 || config.response_max_bytes == 0
        {
            return Err(Self::load_error(
                "HTTP timeout and byte limits must be positive",
            ));
        }
        let timeout = Duration::from_millis(config.timeout_ms);
        if Instant::now().checked_add(timeout).is_none() {
            return Err(Self::load_error(
                "HTTP timeout overflows the monotonic clock",
            ));
        }
        let agent = ureq::Agent::config_builder()
            .proxy(None)
            .max_redirects(0)
            .max_redirects_will_error(false)
            .http_status_as_error(false)
            .timeout_global(Some(timeout))
            .build()
            .into();
        Ok(Some(Self {
            agent,
            allowed_urls: config.allowed_urls.clone(),
            timeout,
            finish_reserve: Duration::from_millis(
                config
                    .finish_reserve_ms
                    .unwrap_or(DEFAULT_FINISH_RESERVE_MS),
            ),
            request_max_bytes: config.request_max_bytes as usize,
            response_max_bytes: u64::from(config.response_max_bytes),
            call_deadline: None,
        }))
    }

    pub(crate) fn begin_call(&mut self, started_at: Instant, timeout_ms: Option<u64>) {
        self.call_deadline =
            timeout_ms.and_then(|timeout| started_at.checked_add(Duration::from_millis(timeout)));
    }

    pub(crate) fn end_call(&mut self) {
        self.call_deadline = None;
    }

    fn request(&self, input: HttpRequest<'_>) -> Result<Val, String> {
        if input.method.eq_ignore_ascii_case("CONNECT") {
            return Err("HTTP CONNECT tunnels are not supported".into());
        }
        if !self.allowed_urls.iter().any(|url| url == input.url) {
            return Err("HTTP URL is not allowed".into());
        }
        if input.body.len() > self.request_max_bytes {
            return Err("HTTP request body exceeds configured byte limit".into());
        }
        let mut builder = ureq::http::Request::builder()
            .method(input.method)
            .uri(input.url);
        for header in input.headers {
            let Val::Tuple(fields) = header else {
                return Err("invalid HTTP header".into());
            };
            let [Val::String(name), Val::String(value)] = fields.as_slice() else {
                return Err("invalid HTTP header".into());
            };
            // The authorized URL controls the destination, including its virtual host.
            if ["host", "content-length", "transfer-encoding"]
                .iter()
                .any(|controlled| name.eq_ignore_ascii_case(controlled))
            {
                return Err("HTTP Host, Content-Length and Transfer-Encoding headers are managed by the transport".into());
            }
            builder = builder.header(name.as_str(), value.as_str());
        }
        let body = input
            .body
            .iter()
            .map(|byte| match byte {
                Val::U8(byte) => Ok(*byte),
                _ => Err("invalid HTTP body byte".to_string()),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let request = builder
            .body(body)
            .map_err(|error| format!("invalid HTTP request: {error}"))?;
        // Leave time for the guest to handle the HTTP error and finish its call.
        let remaining = match self.call_deadline {
            Some(deadline) => deadline
                .checked_duration_since(Instant::now())
                .and_then(|remaining| remaining.checked_sub(self.finish_reserve))
                .filter(|value| !value.is_zero())
                .ok_or("plugin call deadline leaves no time for HTTP after finish reserve")?
                .min(self.timeout),
            None => self.timeout,
        };
        let request = self
            .agent
            .configure_request(request)
            .timeout_global(Some(remaining))
            .build();
        // The transport deadline covers body reads, where Wasmtime epochs cannot interrupt IO.
        let mut response = self
            .agent
            .run(request)
            .map_err(|error| format!("HTTP request failed: {error}"))?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .map(|(name, value)| {
                let value = value
                    .to_str()
                    .map_err(|error| format!("HTTP response header is not text: {error}"))?;
                Ok(Val::Tuple(vec![
                    Val::String(name.to_string()),
                    Val::String(value.to_string()),
                ]))
            })
            .collect::<Result<Vec<_>, String>>()?;
        let bytes = response
            .body_mut()
            .with_config()
            .limit(self.response_max_bytes)
            .read_to_vec()
            .map_err(|error| format!("HTTP response failed: {error}"))?;
        Ok(Val::Record(vec![
            ("status".into(), Val::U16(status)),
            ("headers".into(), Val::List(headers)),
            (
                "body".into(),
                Val::List(bytes.into_iter().map(Val::U8).collect()),
            ),
        ]))
    }

    pub(crate) fn add_to(linker: &mut Linker<WasmStoreState>) -> Result<(), PluginRuntimeError> {
        linker
            .instance("actrail:plugin/http-host@0.4.0")
            .map_err(|error| Self::load_error(error.to_string()))?
            .func_new("request", |store, _ty, params, results| {
                let result = store
                    .data()
                    .http
                    .as_ref()
                    .ok_or_else(|| "http.request is not granted".to_string())
                    .and_then(|client| {
                        HttpRequest::decode(params).and_then(|request| client.request(request))
                    });
                results[0] = Val::Result(match result {
                    Ok(value) => Ok(Some(Box::new(value))),
                    Err(error) => Err(Some(Box::new(Val::String(error)))),
                });
                Ok(())
            })
            .map_err(|error| Self::load_error(error.to_string()))?;
        Ok(())
    }

    fn load_error(message: impl Into<String>) -> PluginRuntimeError {
        PluginRuntimeError::new("wasm_runtime", message)
    }
}

struct HttpRequest<'a> {
    method: &'a str,
    url: &'a str,
    headers: &'a [Val],
    body: &'a [Val],
}

impl<'a> HttpRequest<'a> {
    fn decode(params: &'a [Val]) -> Result<Self, String> {
        let [Val::Record(fields)] = params else {
            return Err("HTTP request requires a record".into());
        };
        let get = |name: &str| {
            fields
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value)
                .ok_or_else(|| format!("HTTP request missing {name}"))
        };
        let (Val::String(method), Val::String(url), Val::List(headers), Val::List(body)) =
            (get("method")?, get("url")?, get("headers")?, get("body")?)
        else {
            return Err("invalid HTTP request fields".into());
        };
        Ok(Self {
            method,
            url,
            headers,
            body,
        })
    }
}
