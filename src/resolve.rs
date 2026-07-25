use std::process::Command;

/// Where a parameter count came from, plus the human trail.
#[derive(Debug, Clone)]
pub struct Resolution {
    pub source: &'static str,
    pub resolved_name: String,
    pub params_b: Option<f64>,
    pub note_key: &'static str,
    pub note_en: String,
}

/// Accept only names that are safe to embed in URLs and JSON payloads.
pub fn sanitize_model_name(raw: &str) -> Option<String> {
    let t = raw.trim();
    if t.is_empty() || t.len() > 128 {
        return None;
    }
    let ok = t
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/' | ':' | ' '));
    ok.then(|| t.to_string())
}

/// Raw sizes like "12b", "7.5B", "12gb", "14".
pub fn parse_params_b(input: &str) -> Option<f64> {
    let normalized = input.trim().to_lowercase().replace("gb", "").replace('b', "");
    let v = normalized.trim().parse::<f64>().ok()?;
    (v.is_finite() && v > 0.0 && v <= 5000.0).then_some(v)
}

/// Pull a "14b" / "135m" style size out of a model name or repo id.
pub fn extract_param_hint(text: &str) -> Option<f64> {
    let lower = text.to_lowercase();
    let cleaned: String = lower
        .chars()
        .map(|c| {
            if matches!(c, '-' | '_' | ':' | '/' | ',' | '(' | ')' | '"') {
                ' '
            } else {
                c
            }
        })
        .collect();
    for token in cleaned.split_whitespace() {
        let (num, scale) = if let Some(v) = token.strip_suffix('b') {
            (v, 1.0)
        } else if let Some(v) = token.strip_suffix('m') {
            (v, 0.001)
        } else {
            continue;
        };
        if let Ok(n) = num.parse::<f64>() {
            let p = n * scale;
            if (0.05..=5000.0).contains(&p) {
                return Some(p);
            }
        }
    }
    None
}

/// "14.8B" / "335M" labels as reported by Ollama.
fn parse_size_label(s: &str) -> Option<f64> {
    let t = s.trim().to_lowercase();
    let (num, scale) = if let Some(v) = t.strip_suffix('b') {
        (v, 1.0)
    } else if let Some(v) = t.strip_suffix('m') {
        (v, 0.001)
    } else {
        (t.as_str(), 1.0)
    };
    let n: f64 = num.trim().parse().ok()?;
    let p = n * scale;
    (p > 0.0 && p <= 5000.0).then_some(p)
}

fn curl_get(url: &str) -> Option<String> {
    let out = Command::new("curl")
        .args(["-sS", "-f", "--max-time", "6", "--url", url])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}

fn curl_post_json(url: &str, body: &str) -> Option<String> {
    let out = Command::new("curl")
        .args([
            "-sS",
            "-f",
            "--max-time",
            "6",
            "-H",
            "Content-Type: application/json",
            "-d",
            body,
            "--url",
            url,
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}

fn url_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' | b':' => {
                (b as char).to_string()
            }
            _ => format!("%{:02X}", b),
        })
        .collect()
}

fn ollama_show(name: &str) -> Option<Resolution> {
    let payload = serde_json::json!({ "model": name }).to_string();
    let body = curl_post_json("http://127.0.0.1:11434/api/show", &payload)
        .or_else(|| curl_post_json("http://localhost:11434/api/show", &payload))?;
    let v: serde_json::Value = serde_json::from_str(&body).ok()?;
    if v.get("error").is_some() {
        return None;
    }
    let params = v
        .pointer("/model_info/general.parameter_count")
        .and_then(|x| x.as_f64())
        .map(|x| x / 1e9)
        .or_else(|| {
            v.pointer("/details/parameter_size")
                .and_then(|x| x.as_str())
                .and_then(parse_size_label)
        })
        .or_else(|| extract_param_hint(name));
    Some(Resolution {
        source: "local_ollama",
        resolved_name: name.to_string(),
        params_b: params,
        note_key: "note_ollama",
        note_en: "Resolved from the local Ollama API (/api/show).".into(),
    })
}

fn ollama_tags(name: &str) -> Option<Resolution> {
    let body = curl_get("http://127.0.0.1:11434/api/tags")
        .or_else(|| curl_get("http://localhost:11434/api/tags"))?;
    let v: serde_json::Value = serde_json::from_str(&body).ok()?;
    let target = name.to_lowercase();
    for m in v.get("models")?.as_array()? {
        let m_name = m
            .get("name")
            .or_else(|| m.get("model"))
            .and_then(|x| x.as_str())
            .unwrap_or("");
        if !m_name.is_empty() && m_name.to_lowercase().contains(&target) {
            let params = m
                .pointer("/details/parameter_size")
                .and_then(|x| x.as_str())
                .and_then(parse_size_label)
                .or_else(|| extract_param_hint(m_name));
            return Some(Resolution {
                source: "local_ollama",
                resolved_name: m_name.to_string(),
                params_b: params,
                note_key: "note_ollama_tags",
                note_en: "Matched against the local Ollama model list (/api/tags).".into(),
            });
        }
    }
    None
}

fn hf_direct(name: &str) -> Option<Resolution> {
    let body = curl_get(&format!(
        "https://huggingface.co/api/models/{}",
        url_encode(name)
    ))?;
    let v: serde_json::Value = serde_json::from_str(&body).ok()?;
    let id = v
        .get("id")
        .and_then(|x| x.as_str())
        .unwrap_or(name)
        .to_string();
    let params = v
        .pointer("/safetensors/total")
        .and_then(|x| x.as_f64())
        .map(|x| x / 1e9)
        .or_else(|| {
            v.pointer("/gguf/total")
                .and_then(|x| x.as_f64())
                .map(|x| x / 1e9)
        })
        .or_else(|| extract_param_hint(&id))
        .or_else(|| extract_param_hint(name));
    Some(Resolution {
        source: "hf_direct",
        resolved_name: id,
        params_b: params,
        note_key: "note_hf_direct",
        note_en: "Resolved from the Hugging Face model endpoint.".into(),
    })
}

fn hf_search(name: &str) -> Option<Resolution> {
    let body = curl_get(&format!(
        "https://huggingface.co/api/models?search={}&limit=3",
        url_encode(name)
    ))?;
    let v: serde_json::Value = serde_json::from_str(&body).ok()?;
    for item in v.as_array()? {
        if let Some(id) = item.get("id").and_then(|x| x.as_str()) {
            let params = item
                .pointer("/safetensors/total")
                .and_then(|x| x.as_f64())
                .map(|x| x / 1e9)
                .or_else(|| extract_param_hint(id));
            if params.is_some() {
                return Some(Resolution {
                    source: "hf_search",
                    resolved_name: id.to_string(),
                    params_b: params,
                    note_key: "note_hf_search",
                    note_en: "Resolved via Hugging Face search; closest match picked.".into(),
                });
            }
        }
    }
    None
}

/// Lookup order: raw size, local Ollama (show, tags), Hugging Face (direct,
/// search), then size parsed from the name itself.
pub fn resolve(input: &str) -> Resolution {
    if let Some(v) = parse_params_b(input) {
        return Resolution {
            source: "direct",
            resolved_name: input.trim().to_string(),
            params_b: Some(v),
            note_key: "note_direct",
            note_en: "Parameter count supplied directly.".into(),
        };
    }

    let steps: [fn(&str) -> Option<Resolution>; 4] =
        [ollama_show, ollama_tags, hf_direct, hf_search];
    for step in steps {
        if let Some(r) = step(input) {
            if r.params_b.is_some() {
                return r;
            }
        }
    }

    if let Some(p) = extract_param_hint(input) {
        return Resolution {
            source: "name_only",
            resolved_name: input.to_string(),
            params_b: Some(p),
            note_key: "note_name_only",
            note_en: "No local or remote metadata; size read from the name itself.".into(),
        };
    }

    Resolution {
        source: "unresolved",
        resolved_name: input.to_string(),
        params_b: None,
        note_key: "note_unresolved",
        note_en: "Could not resolve this model from Ollama, Hugging Face, or the name.".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_sizes() {
        assert_eq!(parse_params_b("12b"), Some(12.0));
        assert_eq!(parse_params_b(" 7.5B "), Some(7.5));
        assert_eq!(parse_params_b("12gb"), Some(12.0));
        assert_eq!(parse_params_b("14"), Some(14.0));
        assert_eq!(parse_params_b("qwen3"), None);
        assert_eq!(parse_params_b("0"), None);
        assert_eq!(parse_params_b("-4b"), None);
    }

    #[test]
    fn name_hints() {
        assert_eq!(extract_param_hint("qwen3:14b"), Some(14.0));
        assert_eq!(extract_param_hint("unsloth/Qwen3-14B-GGUF"), Some(14.0));
        assert_eq!(extract_param_hint("smollm:135m"), Some(0.135));
        assert_eq!(extract_param_hint("llama3.1-8b-instruct-q4_k_m"), Some(8.0));
        assert_eq!(extract_param_hint("mixtral-8x7b"), None); // MoE names stay unresolved
        assert_eq!(extract_param_hint("gemma"), None);
    }

    #[test]
    fn size_labels() {
        assert_eq!(parse_size_label("14.8B"), Some(14.8));
        assert_eq!(parse_size_label("335M"), Some(0.335));
        assert_eq!(parse_size_label(""), None);
    }

    #[test]
    fn sanitize() {
        assert!(sanitize_model_name("qwen3:14b").is_some());
        assert!(sanitize_model_name("unsloth/Qwen3-14B-GGUF").is_some());
        assert!(sanitize_model_name("a$(rm -rf /)").is_none());
        assert!(sanitize_model_name("").is_none());
        assert!(sanitize_model_name(&"x".repeat(200)).is_none());
    }
}
