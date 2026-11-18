//! Credential filtering is independent of any service's response semantics.
use aho_corasick::{AhoCorasick, MatchKind};
use anyhow::{Result, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use reqwest::header::HeaderMap;
use serde_json::Value;
use std::collections::BTreeSet;

pub(super) struct CredentialFilter {
    matcher: Option<AhoCorasick>,
    patterns: usize,
}

impl CredentialFilter {
    pub(super) fn from_headers(headers: &HeaderMap, additional: &[String]) -> Result<Self> {
        let mut values = BTreeSet::new();
        for (key, value) in headers {
            // These carry protocol/content metadata by default. A service may
            // explicitly mark even these headers sensitive using configuration.
            if matches!(
                key.as_str(),
                "accept"
                    | "accept-encoding"
                    | "accept-language"
                    | "content-type"
                    | "content-length"
                    | "user-agent"
                    | "host"
                    | "connection"
            ) && !additional
                .iter()
                .any(|s| s.eq_ignore_ascii_case(key.as_str()))
            {
                continue;
            }
            let raw = value
                .to_str()
                .map_err(|_| anyhow::anyhow!("无法建立请求头凭据过滤器"))?;
            if raw.is_empty() {
                continue;
            }
            values.insert(raw.to_owned());
            if (key == "authorization" || key == "proxy-authorization")
                && let Some((scheme, token)) = raw.split_once(' ')
            {
                let token = token.trim();
                if !token.is_empty() {
                    values.insert(token.to_owned());
                }
                if scheme.eq_ignore_ascii_case("basic")
                    && let Ok(decoded) = STANDARD.decode(token)
                    && let Ok(decoded) = String::from_utf8(decoded)
                {
                    if let Some((_, password)) = decoded.split_once(':')
                        && !password.is_empty()
                    {
                        values.insert(password.to_owned());
                    }
                    values.insert(decoded);
                }
            }
            if key == "cookie" {
                for cookie in raw.split(';') {
                    if let Some((_, value)) = cookie.trim().split_once('=')
                        && !value.is_empty()
                    {
                        values.insert(value.to_owned());
                    }
                }
            }
        }
        ensure!(
            values.iter().map(String::len).sum::<usize>() <= 512 * 1024,
            "请求头凭据超过过滤器容量"
        );
        let originals: Vec<_> = values.iter().cloned().collect();
        for value in originals {
            let encoded: String = url::form_urlencoded::byte_serialize(value.as_bytes()).collect();
            values.insert(encoded.replace('+', "%20"));
            let mut lower = encoded.as_bytes().to_vec();
            for index in 0..lower.len().saturating_sub(2) {
                if lower[index] == b'%' {
                    lower[index + 1].make_ascii_lowercase();
                    lower[index + 2].make_ascii_lowercase();
                }
            }
            let lower = String::from_utf8(lower).expect("percent encoding is ASCII");
            values.insert(lower.replace('+', "%20"));
            values.insert(lower);
            values.insert(encoded);
        }
        let variants: Vec<_> = values.iter().cloned().collect();
        for value in variants {
            let escaped = serde_json::to_string(&value)?;
            values.insert(escaped[1..escaped.len() - 1].to_owned());
        }
        let patterns = values.len();
        let matcher = if patterns == 0 {
            None
        } else {
            Some(
                AhoCorasick::builder()
                    .match_kind(MatchKind::LeftmostLongest)
                    .build(&values)
                    .map_err(|_| anyhow::anyhow!("无法建立凭据过滤器"))?,
            )
        };
        Ok(Self { matcher, patterns })
    }

    pub(super) fn redact_text(&self, value: &str) -> String {
        match &self.matcher {
            Some(matcher) => matcher.replace_all(value, &vec!["[redacted]"; self.patterns]),
            None => value.to_owned(),
        }
    }

    pub(super) fn redact_value(&self, value: &mut Value) {
        match value {
            Value::String(text) => *text = self.redact_text(text),
            Value::Array(items) => {
                for item in items {
                    self.redact_value(item);
                }
            }
            Value::Object(object) => {
                let original = std::mem::take(object);
                for (key, mut value) in original {
                    self.redact_value(&mut value);
                    object.insert(self.redact_text(&key), value);
                }
            }
            Value::Number(_) | Value::Bool(_) => {
                let encoded = value.to_string();
                if self.matcher.as_ref().is_some_and(|matcher| {
                    matcher
                        .find(&encoded)
                        .is_some_and(|found| found.start() == 0 && found.end() == encoded.len())
                }) {
                    *value = Value::String("[redacted]".into());
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::HeaderValue;
    use serde_json::json;

    #[test]
    fn credentials_are_filtered_in_nested_data_and_encoded_echoes() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "authorization",
            HeaderValue::from_static("Bearer fixture-secret/with+chars"),
        );
        headers.insert("x-client-id", HeaderValue::from_static("fixture-client-id"));
        headers.insert("x-pin", HeaderValue::from_static("1234567"));
        headers.insert("content-type", HeaderValue::from_static("application/json"));
        let filter = CredentialFilter::from_headers(&headers, &[]).unwrap();
        let mut value = json!({"fixture-client-id":["Bearer fixture-secret/with+chars",
            "fixture-secret%2Fwith%2Bchars", "fixture-secret%2fwith%2bchars"],"pin":1234567,"type":"application/json","value":7});
        filter.redact_value(&mut value);
        assert_eq!(
            value,
            json!({"[redacted]":["[redacted]","[redacted]","[redacted]"],"pin":"[redacted]","type":"application/json","value":7})
        );
    }

    #[test]
    fn overlapping_short_and_mask_shaped_credentials_do_not_reprocess_replacements() {
        let mut headers = HeaderMap::new();
        headers.insert("x-one", HeaderValue::from_static("redacted"));
        headers.insert("x-two", HeaderValue::from_static("redacted-long"));
        let filter = CredentialFilter::from_headers(&headers, &[]).unwrap();
        assert_eq!(
            filter.redact_text("redacted-long redacted"),
            "[redacted] [redacted]"
        );
    }

    #[test]
    fn basic_password_cookie_values_and_explicit_metadata_headers_are_filtered() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "authorization",
            HeaderValue::from_str(&format!(
                "Basic {}",
                STANDARD.encode("user:synthetic-password")
            ))
            .unwrap(),
        );
        headers.insert(
            "cookie",
            HeaderValue::from_static("session=synthetic-cookie; nonce=short"),
        );
        headers.insert(
            "user-agent",
            HeaderValue::from_static("synthetic-agent-secret"),
        );
        let filter = CredentialFilter::from_headers(&headers, &["User-Agent".into()]).unwrap();
        assert_eq!(
            filter.redact_text("synthetic-password synthetic-cookie short synthetic-agent-secret"),
            "[redacted] [redacted] [redacted] [redacted]"
        );
    }
}
