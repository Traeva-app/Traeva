//! Real check of finding files by subject (the student's decisions 10/16:
//! organize-spec v0.15, prompt classify-6; Find's related words, prompt
//! find-words-2). With OpenAI, in a separate test folder named by
//! `TRAEVA_APP_DATA_DIR` (#65) and that folder's own Keychain account, as
//! the app uses it: the normal app data and the normal Keychain account are
//! never used. Without the variable, or when the folder cannot be used,
//! it stops before looking for a key or sending anything.
//!
//!   1. Organize analyses two problem sets that never name their subject
//!      (one in English, one in Chinese). Their summaries should start with
//!      the subject. Nothing is moved; a question, if asked, is not answered.
//!   2. Five Find searches over the same synthetic folder. The problem sets
//!      should be found through the summaries or the related words, and the
//!      other files through theirs.
//!
//! At most 7 requests, no retry. `cargo test` never runs it; run it on
//! purpose:
//!
//!   TRAEVA_REAL_MODEL=1 TRAEVA_APP_DATA_DIR=<test folder> \
//!     cargo run -p traeva-core --example subject_check
//!
//! These files are prompt-debugging samples, never CAC demo documents or
//! #22 material. The files are in a temporary folder, deleted at the end;
//! the test folder keeps the model settings and the analyses, and the
//! temporary folder's authorization is withdrawn. It prints the summaries,
//! the words and the results; never the key.

use std::fs;
use std::process::ExitCode;

use traeva_core::{
    APP_DATA_DIR_VAR, AppDataSource, DEFAULT_MODEL, FindView, ModelSettingsInput, ModelStatus,
    OperationStage, OrganizeService, PROVIDER_OPENAI, StorageStatus,
};

const MAX_REQUESTS: usize = 7;

/// Chemistry problems that never say "chemistry".
const SET_4: &str = "Problem set 4\n\n1. How many moles of sodium chloride are in 11.7 g?\n\
                     2. Balance: Fe + O2 -> Fe2O3.\n3. What is the pH of a 0.01 M HCl \
                     solution?\n4. Which reactant is limiting when 2 g of hydrogen react with \
                     32 g of oxygen?\n";

/// The same kind of problems in Chinese, never saying 化学.
const LIANXI: &str = "练习四\n\n1. 计算 11.7 克氯化钠的物质的量。\n2. 配平：铁 + 氧气 → 氧化铁。\n\
                      3. 0.01 mol/L 盐酸的 pH 是多少？\n4. 2 克氢气和 32 克氧气反应，哪种是限量反应物？\n";

const FILES: [(&str, &str); 6] = [
    ("Inbox/set-4.txt", SET_4),
    ("Inbox/练习四.txt", LIANXI),
    (
        "School/English/vocab.txt",
        "Vocabulary, week 6\nabundant, candid, chemistry, diligent, eloquent\n",
    ),
    (
        "School/Physics/forces.txt",
        "1. A 2 kg cart has a net force of 6 N. Find its acceleration.\n\
         2. A ball is thrown up at 12 m/s. How high does it go?\n",
    ),
    (
        "Trips/kyoto-budget.txt",
        "Kyoto, May: guesthouse 3 nights 240 USD, rail pass 90 USD.\n",
    ),
    (
        "Finance/2025-return.txt",
        "Form 1040 for 2025\nW-2 wages: 48,200\nStandard deduction taken. Refund expected \
         in March.\n",
    ),
];

/// Where Organize could suggest the problem sets.
const FOLDERS: [&str; 3] = ["School/Chemistry", "School/History", "Finance"];

/// A question and the files that should be among its results (the first
/// one first, when `first` is set).
struct Search {
    question: &'static str,
    expect: &'static [&'static str],
    first: bool,
}

const SEARCHES: [Search; 5] = [
    Search {
        question: "chemistry homework",
        expect: &["set-4.txt", "练习四.txt"],
        first: false,
    },
    Search {
        question: "化学练习",
        expect: &["练习四.txt", "set-4.txt"],
        first: false,
    },
    Search {
        question: "physics problems",
        expect: &["forces.txt"],
        first: false,
    },
    Search {
        question: "my tax documents",
        expect: &["2025-return.txt"],
        first: false,
    },
    Search {
        question: "kyoto budget",
        expect: &["kyoto-budget.txt"],
        first: true,
    },
];

fn main() -> ExitCode {
    if std::env::var("TRAEVA_REAL_MODEL").as_deref() != Ok("1") {
        eprintln!("This sends real requests to OpenAI. Run it with TRAEVA_REAL_MODEL=1.");
        return ExitCode::from(2);
    }
    match check() {
        Ok(0) => {
            println!("PASS");
            ExitCode::SUCCESS
        }
        Ok(failed) => {
            eprintln!("FAIL: {failed} check(s) above");
            ExitCode::FAILURE
        }
        Err(message) => {
            eprintln!("FAIL: {message}");
            ExitCode::FAILURE
        }
    }
}

/// Runs both parts; returns how many checks failed.
fn check() -> Result<usize, String> {
    let model = std::env::var("TRAEVA_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string());
    if std::env::var_os(APP_DATA_DIR_VAR).is_none() {
        return Err(format!(
            "set {APP_DATA_DIR_VAR} to a separate test folder (#65): nothing was read or sent"
        ));
    }
    let service = OrganizeService::start_for_app();
    let show = |e: traeva_core::AppError| format!("{:?}: {}", e.code, e.message);
    match service.get_app_state().map_err(show)?.storage {
        StorageStatus::Ready(info) if info.app_data_source == AppDataSource::Test => {}
        StorageStatus::Ready(_) => return Err("not a separate test folder".to_string()),
        StorageStatus::Blocked { error } => {
            return Err(format!("the test folder cannot be used: {}", show(error)));
        }
    }
    let root = tempfile::Builder::new()
        .prefix("traeva-real-root-")
        .tempdir()
        .map_err(|e| e.to_string())?;
    for folder in FOLDERS {
        fs::create_dir_all(root.path().join(folder)).map_err(|e| e.to_string())?;
    }
    for (relative, text) in FILES {
        let path = root.path().join(relative);
        fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
        fs::write(path, text).map_err(|e| e.to_string())?;
    }

    let root_id = service.authorize_root(root.path()).map_err(show)?.root_id;
    let settings = service
        .save_model_settings(&ModelSettingsInput {
            provider: PROVIDER_OPENAI.to_string(),
            model: model.clone(),
            api_key: None,
        })
        .map_err(show)?;
    if !settings.key_saved {
        return Err("no API key in the test folder's own Keychain account".to_string());
    }
    if service.set_analysis_consent(true).map_err(show)?.status != ModelStatus::Configured {
        return Err("the model setup is not complete".to_string());
    }
    service.set_find_consent(true).map_err(show)?;
    println!("model: {model}");

    let mut requests = 0;
    let mut failed = 0;

    println!("=== 1. Organize summaries");
    for (relative, subject) in [
        ("Inbox/set-4.txt", &["chemistry"][..]),
        ("Inbox/练习四.txt", &["化学", "chemistry"][..]),
    ] {
        let operation = service.start_operation(&root_id, &root_id).map_err(show)?;
        let id = operation.operation_id;
        service.run_operation(&id, &root.path().join(relative));
        if service.get_operation(&id).map_err(show)?.stage != OperationStage::ReadyForAnalysis {
            return Err(format!("{relative} was not ready for analysis"));
        }
        service.start_analysis(&id).map_err(show)?;
        service.run_analysis(&id);
        let view = service.get_operation(&id).map_err(show)?;
        requests += view.model_requests as usize;
        let Some(suggestion) = &view.suggestion else {
            // A question hides the summary from the view (it is still saved
            // for Find); the summary is not checked here.
            let asked = view.stage == OperationStage::NeedsClarification;
            println!(
                "{} {relative}: no suggestion (stage {:?})",
                if asked { "skip" } else { "MISS" },
                view.stage
            );
            if !asked {
                failed += 1;
            }
            service.cancel_operation(&id).map_err(show)?;
            continue;
        };
        let start: String = suggestion.summary.chars().take(60).collect::<String>();
        let named = subject
            .iter()
            .any(|word| start.to_lowercase().contains(word));
        println!(
            "{} {relative}: {}",
            if named { "ok  " } else { "MISS" },
            suggestion.summary
        );
        if !named {
            failed += 1;
        }
        service.cancel_operation(&id).map_err(show)?;
    }

    println!("=== 2. Find");
    for search in &SEARCHES {
        if requests + 1 > MAX_REQUESTS {
            return Err(format!("no request left for \"{}\"", search.question));
        }
        let started = service.start_find(search.question).map_err(show)?;
        let view = service.run_find(&started.find_id).map_err(show)?;
        requests += 1;
        print_find(search.question, &view);
        let names: Vec<&str> = view.results.iter().map(|r| r.name.as_str()).collect();
        for expected in search.expect {
            if !names.contains(expected) {
                println!("MISS \"{}\" did not list {expected}", search.question);
                failed += 1;
            }
        }
        if search.first && names.first() != search.expect.first() {
            println!(
                "MISS \"{}\" did not list {} first",
                search.question, search.expect[0]
            );
            failed += 1;
        }
    }
    println!("requests: {requests}");
    service.revoke_root(&root_id).map_err(show)?;
    Ok(failed)
}

fn print_find(question: &str, view: &FindView) {
    println!("--- \"{question}\" ({:?})", view.words_by);
    if let Some(words) = &view.words {
        println!("things: {:?}", words.things);
        println!("related: {:?}", words.related);
    }
    for result in &view.results {
        let matches: Vec<String> = result
            .matches
            .iter()
            .map(|m| {
                if m.related_words.is_empty() {
                    format!("{:?} \"{}\"", m.place, m.wording)
                } else {
                    format!("{:?} related {:?}", m.place, m.related_words)
                }
            })
            .collect();
        println!("  {} [{}]", result.relative_path, matches.join("; "));
    }
}
