//! HTTPS on Linux (Windows has WinHTTP): one request through ureq with rustls and Mozilla's
//! root certificates built in, so nothing on the system is needed.

/// One HTTPS request (`headers`: lines ending in CRLF): (status, body).
#[cfg(feature = "https")]
pub fn request(method: &str, host: &str, path: &str, headers: &str, body: &[u8]) -> Result<(u32, Vec<u8>), String> {
    use std::time::Duration;
    let offline = |_| format!("No connection to {host}");
    let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(Duration::from_secs(20))).user_agent("Sofa").build().into();
    let mut req = ureq::http::Request::builder().method(method).uri(format!("https://{host}{path}"));
    for line in headers.split("\r\n").filter(|l| !l.is_empty()) {
        if let Some((k, v)) = line.split_once(':') {
            req = req.header(k.trim(), v.trim());
        }
    }
    let req = req.body(body.to_vec()).map_err(|_| format!("No connection to {host}"))?;
    let mut resp = agent.run(req).map_err(offline)?;
    let status = u32::from(resp.status().as_u16());
    let data = resp.body_mut().with_config().limit(8_000_000).read_to_vec().map_err(offline)?;
    Ok((status, data))
}

/// Built without the `https` feature (type-checking the Linux build on Windows only).
#[cfg(not(feature = "https"))]
pub fn request(_method: &str, host: &str, _path: &str, _headers: &str, _body: &[u8]) -> Result<(u32, Vec<u8>), String> {
    Err(format!("No connection to {host}: this Sofa was built without HTTPS"))
}
