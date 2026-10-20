//! Tolerant lookups in server JSON (field names vary between platforms).

use serde_json::Value;

/// First value under any of `keys`, searched breadth-first.
pub fn find<'a>(v: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    let mut queue = vec![v];
    while !queue.is_empty() {
        let mut next = Vec::new();
        for node in queue {
            match node {
                Value::Object(m) => {
                    for k in keys {
                        if let Some(hit) = m.get(*k) {
                            if !hit.is_null() {
                                return Some(hit);
                            }
                        }
                    }
                    next.extend(m.values());
                }
                Value::Array(a) => next.extend(a.iter()),
                _ => {}
            }
        }
        queue = next;
    }
    None
}

pub fn as_number(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

pub fn find_number(v: &Value, keys: &[&str]) -> Option<f64> {
    find(v, keys).and_then(as_number)
}

pub fn find_string(v: &Value, keys: &[&str]) -> Option<String> {
    match find(v, keys)? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

const MEDIA_EXT: &[&str] = &[
    ".png", ".jpg", ".jpeg", ".webp", ".gif", ".mp4", ".mov", ".webm",
];

/// Every https URL whose path ends in a media extension, deduplicated, full-size
/// files first (thumbnail/preview URLs last), otherwise in document order.
pub fn media_urls(v: &Value) -> Vec<String> {
    fn walk(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::String(s) => {
                let path = s
                    .split(['?', '#'])
                    .next()
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if s.starts_with("https://")
                    && MEDIA_EXT.iter().any(|e| path.ends_with(e))
                    && !out.contains(s)
                {
                    out.push(s.clone());
                }
            }
            Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
            Value::Object(m) => m.values().for_each(|x| walk(x, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(v, &mut out);
    out.sort_by_key(|u| {
        let l = u.to_ascii_lowercase();
        ["thumbnail", "/thumb", "preview", "poster"]
            .iter()
            .any(|k| l.contains(k))
    });
    out
}

/// The first array of objects anywhere in `v` (catalog lists).
pub fn first_object_array(v: &Value) -> Option<&Vec<Value>> {
    match v {
        Value::Array(a) if a.iter().any(Value::is_object) => Some(a),
        Value::Array(a) => a.iter().find_map(first_object_array),
        Value::Object(m) => m.values().find_map(first_object_array),
        _ => None,
    }
}

/// Lowercase alphanumerics only, for tolerant model-name matching.
pub fn norm(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn full_size_before_thumbnail() {
        let v = serde_json::json!({"thumb": "https://c/x/thumbnail/a.webp", "image": "https://c/x/a.png"});
        assert_eq!(super::media_urls(&v)[0], "https://c/x/a.png");
    }
}
