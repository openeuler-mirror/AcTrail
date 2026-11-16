use super::{
    Request, Response, STATUS_BAD_REQUEST, STATUS_CONFLICT, STATUS_INTERNAL_ERROR,
    STATUS_METHOD_NOT_ALLOWED,
};
use crate::configuration::{ConfigEditor, ConfigSaveError};

const STATUS_FORBIDDEN: &str = "403 Forbidden";
const STATUS_UNSUPPORTED_MEDIA_TYPE: &str = "415 Unsupported Media Type";

#[derive(Default)]
pub(super) struct ConfigRequestHeaders {
    host: Option<String>,
    origin: Option<String>,
    fetch_site: Option<String>,
    content_type: Option<String>,
    duplicate: bool,
}

impl ConfigRequestHeaders {
    pub(super) fn capture(&mut self, name: &str, value: &str) {
        let target = if name.eq_ignore_ascii_case("host") {
            &mut self.host
        } else if name.eq_ignore_ascii_case("origin") {
            &mut self.origin
        } else if name.eq_ignore_ascii_case("sec-fetch-site") {
            &mut self.fetch_site
        } else if name.eq_ignore_ascii_case("content-type") {
            &mut self.content_type
        } else {
            return;
        };
        if target.replace(value.to_owned()).is_some() {
            self.duplicate = true;
        }
    }

    fn validate(&self, request: &Request) -> Result<(), Response> {
        let deny = |message| Response::text(STATUS_FORBIDDEN, message);
        if !request.local_addr.ip().is_loopback() || !request.peer_addr.ip().is_loopback() {
            return Err(deny("configuration editing requires a loopback connection"));
        }
        let Some(host) = self.host.as_deref() else {
            return Err(deny("configuration editing requires a Host header"));
        };
        let address_matches = host.parse::<std::net::SocketAddr>().ok() == Some(request.local_addr);
        let localhost_matches =
            host.eq_ignore_ascii_case(&format!("localhost:{}", request.local_addr.port()));
        let default_port_matches = request.local_addr.port() == 80
            && (host.eq_ignore_ascii_case("localhost")
                || host.parse::<std::net::IpAddr>().ok() == Some(request.local_addr.ip())
                || host == format!("[{}]", request.local_addr.ip()));
        if self.duplicate || !(address_matches || localhost_matches || default_port_matches) {
            return Err(deny("configuration editing requires the local server Host"));
        }
        if let Some(origin) = self.origin.as_deref()
            && !origin.eq_ignore_ascii_case(&format!("http://{host}"))
        {
            return Err(deny("configuration editing requires a same-origin request"));
        }
        if let Some(site) = self.fetch_site.as_deref()
            && !matches!(site, "same-origin" | "none")
        {
            return Err(deny("configuration editing rejects cross-site requests"));
        }
        if request.method == "POST"
            && !self.content_type.as_deref().is_some_and(|value| {
                value
                    .split(';')
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .eq_ignore_ascii_case("application/json")
            })
        {
            return Err(Response::text(
                STATUS_UNSUPPORTED_MEDIA_TYPE,
                "configuration editing requires Content-Type: application/json",
            ));
        }
        Ok(())
    }
}

pub(super) struct ConfigHttp<'a> {
    editor: Option<&'a ConfigEditor>,
}

impl<'a> ConfigHttp<'a> {
    pub(super) fn new(editor: Option<&'a ConfigEditor>) -> Self {
        Self { editor }
    }

    pub(super) fn route(&self, request: &Request) -> Result<Response, String> {
        if !matches!(request.method.as_str(), "GET" | "POST") {
            return Ok(Response::text(
                STATUS_METHOD_NOT_ALLOWED,
                "GET or POST required for /api/config/editable",
            ));
        }
        if let Err(response) = request.config_headers.validate(request) {
            return Ok(response);
        }
        let Some(editor) = self.editor else {
            return Ok(if request.method == "GET" {
                Response::json(serde_json::json!({
                    "available": false,
                    "reason": "operator config was not loaded; restart Web with a conf file to edit configuration",
                }).to_string())
            } else {
                Response::text(
                    STATUS_BAD_REQUEST,
                    "configuration editing requires a conf file",
                )
            });
        };
        let result = if request.method == "GET" {
            editor.read()
        } else {
            editor.save(&request.body)
        };
        Ok(match result {
            Ok(response) => Response::json(response),
            Err(ConfigSaveError::Invalid(error)) => Response::text(STATUS_BAD_REQUEST, error),
            Err(ConfigSaveError::ExistingInvalid(error)) => Response::text(STATUS_CONFLICT, error),
            Err(ConfigSaveError::Storage(error)) => Response::text(STATUS_INTERNAL_ERROR, error),
        })
    }
}
