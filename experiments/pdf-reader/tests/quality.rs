//! The quality report against the rules frozen in
//! fixtures/quality/expected.json (written before the report existed), on
//! every fixture of every set. validate.py checks the same rules on the
//! release build and compares the text with the reader without the quality
//! report. Run the generators first: fixtures/generate.py,
//! fixtures/defects/generate.py, fixtures/defects2/generate.py,
//! fixtures/defects3/generate.py, fixtures/layout/generate.py and
//! fixtures/quality/generate.py. A case marked unverified (g25-g27) is
//! checked only against the rules that need no frozen data.

use std::path::{Path, PathBuf};

use serde_json::Value;
use traeva_pdf_experiment::{Level, PDF_READER, TextStatus, read_pdf};

fn here() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn json(path: &str) -> Value {
    serde_json::from_str(&std::fs::read_to_string(here().join(path)).unwrap()).unwrap()
}

/// (name, path, unverified) of every fixture: the sets with frozen
/// readings, and the layout inputs.
fn fixtures() -> Vec<(String, PathBuf, bool)> {
    let mut all = Vec::new();
    for set in [
        "fixtures/expected.json",
        "fixtures/defects/expected.json",
        "fixtures/defects2/expected.json",
        "fixtures/defects3/expected.json",
        "fixtures/quality/expected.json",
    ] {
        for (name, case) in json(set)["fixtures"].as_object().unwrap() {
            let path = here().join("fixtures").join(case["path"].as_str().unwrap());
            all.push((name.clone(), path, case.get("unverified").is_some()));
        }
    }
    for name in json("fixtures/layout/manifest.json")["files"]
        .as_object()
        .unwrap()
        .keys()
    {
        all.push((
            name.clone(),
            here().join("fixtures/layout/generated").join(name),
            false,
        ));
    }
    for name in json("fixtures/layout/exported/manifest.json")["files"]
        .as_object()
        .unwrap()
        .keys()
        .filter(|name| name.ends_with(".pdf"))
    {
        all.push((
            name.clone(),
            here().join("fixtures/layout/exported").join(name),
            false,
        ));
    }
    all
}

#[test]
fn every_fixture_meets_the_frozen_quality_rules() {
    let rules = json("fixtures/quality/expected.json");
    let mut problems = Vec::new();
    for (name, path, unverified) in fixtures() {
        let bytes = std::fs::read(&path).unwrap_or_else(|e| {
            panic!("{}: {e}; run the fixture generators first", path.display())
        });
        let reading = read_pdf(&bytes);
        let summary = &reading.extraction.summary;
        let quality = &reading.quality;
        let mut check = |ok: bool, what: String| {
            if !ok {
                problems.push(format!("{name}: {what}"));
            }
        };
        check(quality.reader == PDF_READER, "reader".into());
        check(
            serde_json::to_value(quality.reading_order).unwrap() == rules["reading_order"],
            "reading_order".into(),
        );
        let has_text = matches!(summary.status, TextStatus::Complete | TextStatus::Partial);
        check(quality.text_available == has_text, "text_available".into());

        let codes = |level: Level| -> Vec<(String, Vec<usize>)> {
            let mut found: Vec<_> = quality
                .diagnostics
                .iter()
                .filter(|d| d.level == level)
                .map(|d| {
                    let code = serde_json::to_value(d.code).unwrap();
                    (code.as_str().unwrap().to_string(), d.pages.clone())
                })
                .collect();
            found.sort();
            found
        };
        let mut risks: Vec<(String, Vec<usize>)> = rules["risks"]
            .get(&name)
            .and_then(Value::as_object)
            .map(|risks| {
                risks
                    .iter()
                    .map(|(code, pages)| {
                        let pages = pages.as_array().unwrap();
                        let pages = pages.iter().map(|p| p.as_u64().unwrap() as usize);
                        (code.clone(), pages.collect())
                    })
                    .collect()
            })
            .unwrap_or_default();
        risks.sort();
        let suspected = codes(Level::SuspectedRisk);
        if unverified {
            risks = suspected.clone();
        }
        check(
            suspected == risks,
            format!("suspected {suspected:?} != frozen {risks:?}"),
        );
        let confirmed = codes(Level::ConfirmedOmission);
        check(
            confirmed.len() == summary.left_out.len(),
            format!(
                "{} confirmed omissions for {} left_out entries",
                confirmed.len(),
                summary.left_out.len()
            ),
        );
        let stopped: Vec<String> = codes(Level::Stopped).into_iter().map(|(c, _)| c).collect();
        if has_text {
            check(stopped.is_empty(), format!("stopped {stopped:?} with text"));
        } else if !unverified {
            let frozen = rules["stop_codes"][&name]
                .as_str()
                .unwrap_or("(none frozen)");
            check(
                stopped == [frozen],
                format!("stopped {stopped:?} != {frozen}"),
            );
        }
        if summary.status == TextStatus::Complete && risks.is_empty() {
            check(
                quality.diagnostics.is_empty(),
                "a complete reading without risks has diagnostics".into(),
            );
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
