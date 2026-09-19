//! Hand-rolled JSON emission for `--json` — no `serde` dependency, matching
//! the rest of this workspace's "no external crates for a small, fixed
//! shape" pattern (see `engine/src/pdf.rs`). Consumed by the VS Code
//! extension (`vscode-extension/`) to place diagnostics without having to
//! parse the human-readable report text.

use tpt_verify_engine::FunctionReport;

/// Renders one file's analysis result as a JSON object:
/// `{"path": ..., "parse_error": ..., "functions": [...]}`.
pub fn file_to_json(path: &str, result: &Result<Vec<FunctionReport>, String>) -> String {
    match result {
        Ok(reports) => {
            let functions: Vec<String> = reports.iter().map(function_to_json).collect();
            format!(
                "{{\"path\":{},\"parse_error\":null,\"functions\":[{}]}}",
                json_string(path),
                functions.join(",")
            )
        }
        Err(e) => format!("{{\"path\":{},\"parse_error\":{},\"functions\":[]}}", json_string(path), json_string(e)),
    }
}

fn function_to_json(r: &FunctionReport) -> String {
    let params: Vec<String> = r.params.iter().map(|p| json_string(p)).collect();
    match (&r.error, &r.result) {
        (Some(err), _) => format!(
            "{{\"name\":{},\"params\":[{}],\"error\":{},\"clean\":false,\"paths_explored\":0,\"findings\":[]}}",
            json_string(&r.name),
            params.join(","),
            json_string(err)
        ),
        (None, Some(result)) => {
            let findings: Vec<String> = result
                .findings
                .iter()
                .map(|f| {
                    format!(
                        "{{\"kind\":{},\"summary\":{},\"detail\":{},\"explanation\":{}}}",
                        json_string(&format!("{:?}", f.kind)),
                        json_string(&f.summary),
                        json_string(&f.detail),
                        json_string(&f.explanation)
                    )
                })
                .collect();
            format!(
                "{{\"name\":{},\"params\":[{}],\"error\":null,\"clean\":{},\"paths_explored\":{},\"findings\":[{}]}}",
                json_string(&r.name),
                params.join(","),
                result.is_clean(),
                result.paths_explored,
                findings.join(",")
            )
        }
        (None, None) => format!(
            "{{\"name\":{},\"params\":[{}],\"error\":null,\"clean\":false,\"paths_explored\":0,\"findings\":[]}}",
            json_string(&r.name),
            params.join(",")
        ),
    }
}

/// A JSON string literal, escaped per RFC 8259 (the characters that
/// actually show up in Rust source/identifiers/messages — control
/// characters, quotes, backslashes).
fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_quotes_and_backslashes() {
        assert_eq!(json_string("a\"b\\c"), "\"a\\\"b\\\\c\"");
    }

    #[test]
    fn escapes_newlines() {
        assert_eq!(json_string("a\nb"), "\"a\\nb\"");
    }

    #[test]
    fn file_to_json_is_valid_shape_for_a_parse_error() {
        let out = file_to_json("f.rs", &Err("bad syntax".to_string()));
        assert!(out.contains("\"parse_error\":\"bad syntax\""));
        assert!(out.contains("\"functions\":[]"));
    }
}
