//! Replays every call rust-anything recorded from Sofa 1.x's `src/vdf.js` (tests/cases.jsonl)
//! against the Rust port: same result, same JS key order, same error message.

use serde_json::Value;

#[test]
fn every_recorded_case_matches_the_original() {
    let cases = include_str!("cases.jsonl");
    let (mut total, mut failed) = (0, Vec::new());
    for (n, line) in cases.lines().filter(|l| !l.trim().is_empty()).enumerate() {
        let case: Value = serde_json::from_str(line).expect("a recorded case is JSON");
        let args = case["args"].as_array().cloned().unwrap_or_default();
        let arg = |i: usize| args.get(i).cloned().unwrap_or_else(vdf::js::undefined);
        let got = match case["fn"].as_str() {
            Some("parseVdf") => vdf::port::parse_vdf(arg(0)),
            Some("get") => vdf::port::get(arg(0), arg(1)),
            other => panic!("unknown function {other:?}"),
        };
        total += 1;
        let ok = match (&got, case.get("ok"), case.get("error")) {
            // Compared as text so key order counts, as it does in JS.
            (Ok(v), Some(expected), _) => serde_json::to_string(v).ok() == serde_json::to_string(expected).ok(),
            (Err(e), _, Some(Value::String(expected))) => e == expected,
            _ => false,
        };
        if !ok {
            failed.push(format!("case {}: {line}\n  got: {got:?}", n + 1));
        }
    }
    assert!(total >= 172, "expected the full recording, found {total} cases");
    assert!(failed.is_empty(), "{} of {total} cases differ:\n{}", failed.len(), failed.join("\n"));
}

#[test]
fn reads_steam_shaped_files() {
    let manifest = "\"AppState\"\n{\n\t\"appid\"\t\t\"1174180\"\n\t\"name\"\t\t\"Red Dead Redemption 2\"\n}\n";
    let st = vdf::field(&vdf::parse(manifest), "appstate").expect("AppState, any case");
    assert_eq!(vdf::field(&st, "AppID").and_then(|v| v.as_str().map(str::to_owned)).as_deref(), Some("1174180"));
    assert!(vdf::field(&st, "missing").is_none());
}
