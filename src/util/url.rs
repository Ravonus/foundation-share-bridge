//! URL and IPFS reference parsing — all ways to get from a string to a CID.

use url::Url;

pub const PUBLIC_UTILITY_GATEWAY_BASE_URL: &str = "https://dweb.link";

pub fn trim_trailing_slash(value: &str) -> String {
    value.trim().trim_end_matches('/').to_string()
}

pub fn build_gateway_url(base: &str, cid: &str) -> String {
    format!("{}/ipfs/{}", trim_trailing_slash(base), cid.trim())
}

pub fn build_public_utility_gateway_url(cid: &str) -> String {
    build_gateway_url(PUBLIC_UTILITY_GATEWAY_BASE_URL, cid)
}

pub fn build_direct_ip_gateway_base_url(ip: &str) -> String {
    format!("http://{}:8080", ip.trim())
}

#[allow(clippy::uninlined_format_args)]
pub fn encode_query_component(value: &str) -> String {
    value
        .bytes()
        .flat_map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                vec![byte as char]
            }
            b' ' => vec!['+'],
            _ => format!("%{:02X}", byte).chars().collect(),
        })
        .collect()
}

pub fn parse_ipfs_path(raw: &str) -> Option<(String, String)> {
    let trimmed = raw.trim().trim_matches('/');
    if trimmed.is_empty() {
        return None;
    }

    let normalized = trimmed.strip_prefix("ipfs/").unwrap_or(trimmed);
    let mut parts = normalized.splitn(2, '/');
    let cid = parts.next()?.trim();
    if cid.is_empty() {
        return None;
    }

    let relative_path = parts.next().unwrap_or("").trim_matches('/').to_string();
    Some((cid.to_string(), relative_path))
}

pub fn parse_ipfs_reference(raw: &str) -> Option<(String, String)> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Some(rest) = trimmed.strip_prefix("ipfs://") {
        return parse_ipfs_path(rest);
    }

    if let Some(rest) = trimmed.strip_prefix("/ipfs/") {
        return parse_ipfs_path(rest);
    }

    let url = Url::parse(trimmed).ok()?;
    if let Some(host) = url.host_str()
        && let Some((cid, _)) = host.split_once(".ipfs.")
    {
        return Some((cid.to_string(), url.path().trim_matches('/').to_string()));
    }

    let path = url.path();
    let index = path.find("/ipfs/")?;
    parse_ipfs_path(&path[(index + "/ipfs/".len())..])
}
