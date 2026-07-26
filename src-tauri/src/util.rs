use std::path::Path;

use chrono::Utc;
use url::Url;
use uuid::Uuid;

pub fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

pub fn create_id() -> String {
    Uuid::new_v4().to_string()
}

pub fn normalize_canonical_url(raw_url: &str) -> Result<String, String> {
    let mut url = Url::parse(raw_url).map_err(|e| e.to_string())?;
    url.set_fragment(None);

    let host = url.host_str().unwrap_or("").to_lowercase();
    let _ = url.set_host(Some(&host));

    if (url.scheme() == "http" && url.port() == Some(80))
        || (url.scheme() == "https" && url.port() == Some(443))
    {
        let _ = url.set_port(None);
    }

    let drop_params = [
        "utm_source",
        "utm_medium",
        "utm_campaign",
        "utm_term",
        "utm_content",
        "gh_src",
        "ref",
    ];
    {
        let mut pairs: Vec<(String, String)> = url
            .query_pairs()
            .filter(|(k, _)| !drop_params.contains(&k.as_ref()))
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        if pairs.is_empty() {
            url.set_query(None);
        } else {
            url.query_pairs_mut().clear().extend_pairs(pairs.drain(..));
        }
    }

    let mut path = url.path().to_string();
    if path.len() > 1 && path.ends_with('/') {
        path.pop();
        url.set_path(&path);
    }

    Ok(url.to_string())
}

pub fn guess_title_from_url(raw_url: &str) -> String {
    match Url::parse(raw_url) {
        Ok(url) => {
            let parts: Vec<&str> = url.path().split('/').filter(|p| !p.is_empty()).collect();
            let last = parts
                .last()
                .copied()
                .unwrap_or_else(|| url.host_str().unwrap_or("Untitled role"));
            let decoded = urlencoding::decode(last)
                .unwrap_or_else(|_| last.into())
                .into_owned();
            let cleaned = decoded
                .replace(['-', '_'], " ")
                .trim()
                .to_string();
            let without_ext = if let Some(idx) = cleaned.rfind('.') {
                let ext = &cleaned[idx + 1..];
                if ext.chars().all(|c| c.is_ascii_alphanumeric()) && ext.len() <= 5 {
                    cleaned[..idx].to_string()
                } else {
                    cleaned
                }
            } else {
                cleaned
            };
            title_case(&without_ext)
        }
        Err(_) => "Untitled role".to_string(),
    }
}

fn title_case(s: &str) -> String {
    s.split_whitespace()
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().collect::<String>() + chars.as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn extension_for_mime(mime_type: &str, original_filename: &str) -> String {
    let from_name = Path::new(original_filename)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{}", e.to_lowercase()));
    if let Some(ext) = from_name {
        if !ext.is_empty() && ext != "." {
            return ext;
        }
    }
    match mime_type {
        "application/pdf" => ".pdf".into(),
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => ".docx".into(),
        "text/plain" => ".txt".into(),
        _ => String::new(),
    }
}

pub fn format_label(value: &str) -> String {
    value
        .split('_')
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().collect::<String>() + chars.as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_tracking_params_and_trailing_slash() {
        let got = normalize_canonical_url(
            "https://Jobs.Example.com/role/design/?utm_source=x&gh_src=y#frag",
        )
        .unwrap();
        assert_eq!(got, "https://jobs.example.com/role/design");
    }
}
