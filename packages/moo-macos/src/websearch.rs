//! Search suggestions in the OpenSearch format Google, DuckDuckGo, Bing and Brave all answer
//! with: `["query", ["suggestion", …], …]`. Which engine and URL is decided in Tish.

use tishlang_core::{json_parse, Value};

/// Most suggestions kept from one answer.
pub const MAX: usize = 8;

/// The suggestions in an OpenSearch answer, without the query itself and duplicates.
pub fn parse(body: &str, query: &str) -> Vec<String> {
    let Ok(Value::Array(top)) = json_parse(body) else {
        return Vec::new();
    };
    let top = top.borrow();
    let Some(Value::Array(list)) = top.get(1) else {
        return Vec::new();
    };
    let mut out: Vec<String> = Vec::new();
    for v in list.borrow().iter() {
        if let Value::String(s) = v {
            let s = s.as_str().trim();
            if !s.is_empty() && !s.eq_ignore_ascii_case(query.trim()) && !out.iter().any(|o| o.eq_ignore_ascii_case(s)) {
                out.push(s.to_string());
            }
        }
        if out.len() == MAX {
            break;
        }
    }
    out
}

/// Fetches `url` and parses it. Blocks: call from a worker thread.
#[cfg(target_os = "macos")]
pub fn suggest(url: &str, query: &str) -> Result<Vec<String>, String> {
    let mut req = crate::http::Request::get(url).header("Accept", "application/json");
    req.timeout = 5.0;
    let (status, body) = crate::http::fetch(&req)?;
    if status != 200 {
        return Err(format!("{url} answered {status}"));
    }
    Ok(parse(&body, query))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_opensearch_suggestions() {
        let google = r#"["rust trai",["rust traits","rust trailer","Rust Traits","rust trai"],[],{"google:suggestsubtypes":[[512]]}]"#;
        assert_eq!(parse(google, "rust trai"), ["rust traits", "rust trailer"]);
        assert_eq!(parse(r#"["x",[]]"#, "x"), Vec::<String>::new());
        assert_eq!(parse("<html>blocked</html>", "x"), Vec::<String>::new());
        assert_eq!(parse(r#"["caf",["café \u00e9t\u00e9"]]"#, "caf"), ["café été"]);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn fetches_through_urlsession() {
        let base = crate::http::tests::serve(|line, _h, _b| {
            assert!(line.contains("q=rust%20trai"), "{line}");
            (200, "application/json", vec![r#"["rust trai",["rust traits","rust trailer"]]"#.into()])
        });
        let got = suggest(&format!("{base}/ac?q=rust%20trai"), "rust trai").unwrap();
        assert_eq!(got, ["rust traits", "rust trailer"]);
    }
}
