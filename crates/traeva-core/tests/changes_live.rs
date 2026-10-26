//! A check of Memory's change analysis (#33) and search (#34) against the
//! real model. It never runs by itself:
//!   - it is `#[ignore]`d, and also needs `TRAEVA_MEMORY_LIVE=1` (without
//!     it, a run of the ignored tests skips this one and sends nothing);
//!   - `TRAEVA_APP_DATA_DIR` must name a separate test folder (#65), whose
//!     own Keychain account (`openai@test-…`) already holds a key the
//!     student saved there. The normal folder and the normal key are never
//!     used, and this never handles a key;
//!   - `TRAEVA_MEMORY_LIVE_ROOT` names an empty synthetic folder outside
//!     the test folder, which this authorizes and writes its essay into.
//!
//! The essay is synthetic. At most `BUDGET` requests (or fewer, see below) go out; the run stops
//! before the next one would go past it.
//!
//! ```text
//! TRAEVA_MEMORY_LIVE=1 TRAEVA_APP_DATA_DIR=… TRAEVA_MEMORY_LIVE_ROOT=… \
//!   cargo test -p traeva-core --test changes_live -- --ignored --nocapture
//! ```

use std::fs;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use traeva_core::{
    AppDataSource, CandidateChoice, CandidateDecision, CandidateDecisionRequest,
    CandidateSwitchRequest, CheckOutcome, ModelSettingsInput, OrganizeService, PairState, PassStep,
    SearchState, StorageStatus, VersionPairView,
};

/// Requests this check may send, all in all (the student's limit for one
/// milestone is 20). `TRAEVA_MEMORY_LIVE_BUDGET` lowers it to what earlier
/// runs left.
const BUDGET: usize = 20;

/// Five saved versions of a synthetic essay: a citation fixed, a
/// conclusion added, another citation fixed, then a reworded opening.
const VERSIONS: [&str; 5] = [
    "Carbon pricing essay\n\nSmith 2019 argues that carbon taxes cut emissions.\nJones 2020 finds the effect is small.\nPrices alone may not be enough.\n",
    "Carbon pricing essay\n\nSmith (2019, p. 4) argues that carbon taxes cut emissions.\nJones 2020 finds the effect is small.\nPrices alone may not be enough.\n",
    "Carbon pricing essay\n\nSmith (2019, p. 4) argues that carbon taxes cut emissions.\nJones 2020 finds the effect is small.\nPrices alone may not be enough.\n\nIn conclusion, a carbon tax works best with rebates.\n",
    "Carbon pricing essay\n\nSmith (2019, p. 4) argues that carbon taxes cut emissions.\nJones (2020, p. 17) finds the effect is small.\nPrices alone may not be enough.\n\nIn conclusion, a carbon tax works best with rebates.\n",
    "Carbon pricing essay\n\nThis essay asks whether carbon taxes work.\nSmith (2019, p. 4) argues that carbon taxes cut emissions.\nJones (2020, p. 17) finds the effect is small.\nPrices alone may not be enough.\n\nIn conclusion, a carbon tax works best with rebates.\n",
];

/// Searches written while building #34, not the held-out ones.
const SEARCHES: [&str; 3] = [
    "the version before I added the conclusion",
    "the draft right after I fixed the Jones citation",
    "before I fixed a citation",
];

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set"))
}

#[test]
#[ignore = "sends to the real model; see the module docs"]
fn changes_and_searches_against_the_real_model() {
    // `cargo test -- --ignored` also runs the disk-image tests: without the
    // second switch this sends nothing and passes.
    if std::env::var("TRAEVA_MEMORY_LIVE").as_deref() != Ok("1") {
        println!("skipped: TRAEVA_MEMORY_LIVE is not 1, so nothing was sent");
        return;
    }
    env(traeva_core::APP_DATA_DIR_VAR);
    let budget = std::env::var("TRAEVA_MEMORY_LIVE_BUDGET")
        .map(|left| left.parse::<usize>().expect("a number of requests"))
        .unwrap_or(BUDGET)
        .min(BUDGET);
    println!("budget: {budget} requests");
    let root = PathBuf::from(env("TRAEVA_MEMORY_LIVE_ROOT"));
    assert!(
        fs::read_dir(&root).unwrap().next().is_none(),
        "{} must be empty",
        root.display()
    );

    let service = OrganizeService::start_for_app();
    let storage = service.get_app_state().unwrap().storage;
    assert!(
        matches!(
            &storage,
            StorageStatus::Ready(info) if info.app_data_source == AppDataSource::Test
        ),
        "not a separate test folder: {storage:?}"
    );

    // The essay, saved five times while Memory is on.
    let essay = root.join("essay.md");
    fs::write(&essay, VERSIONS[0]).unwrap();
    let root_id = service.authorize_root(&root).unwrap().root_id;
    let file_id = service
        .register_file(&root_id, &essay)
        .unwrap()
        .file
        .file_id;
    service.start_memory(&file_id).unwrap();
    for text in &VERSIONS[1..] {
        fs::write(&essay, text).unwrap();
        wait_until_saved(&service);
    }
    assert_eq!(
        service.list_snapshots(&file_id).unwrap().len(),
        VERSIONS.len()
    );

    // Provider and model (the key stays the one in the Keychain), then
    // Memory's agreement for this file only.
    let settings = service
        .save_model_settings(&ModelSettingsInput {
            provider: traeva_core::PROVIDER_OPENAI.to_string(),
            model: traeva_core::DEFAULT_MODEL.to_string(),
            api_key: None,
        })
        .unwrap();
    println!("model: {:?} {:?}", settings.provider, settings.model);
    // A key that an earlier run had refused holds every request until the
    // settings change (changes.rs `setup_on_hold`), and a key saved again
    // with `security` is not a settings change Traeva can see. Withdrawing
    // Organize's agreement to send, which this folder never uses, is one:
    // like saving the settings again, it ends the hold.
    service.set_analysis_consent(false).unwrap();
    println!("settings saved again: an earlier refused key no longer holds requests");
    // Only this run's file may be sent: an earlier run stopped half way
    // may have left its file agreed to.
    let mut withdrawn = 0;
    for watch in service.memory_watches().unwrap() {
        if watch.file_id != file_id && service.memory_consent(&watch.file_id).unwrap().agreed {
            service.set_memory_consent(&watch.file_id, false).unwrap();
            withdrawn += 1;
        }
    }
    println!("agreements of other files withdrawn: {withdrawn}");
    service.set_memory_consent(&file_id, true).unwrap();

    let mut sent = 0;
    let mut held = None;
    let started = Instant::now();
    // One pair per pass; a failed one is tried again a minute later. Stops
    // when every pair is described, when a failure holds every request
    // until the settings change, or after ten minutes.
    while sent < budget && started.elapsed() < Duration::from_secs(600) {
        let steps = service.analyze_memory().unwrap();
        let pairs = service.memory_changes(&file_id).unwrap();
        for step in &steps {
            if let PassStep::Described {
                pair_id,
                status,
                reason,
            } = step
            {
                sent += 1;
                let code = pairs
                    .iter()
                    .find(|pair| pair.pair_id.as_deref() == Some(pair_id.as_str()))
                    .and_then(|pair| match &pair.state {
                        PairState::Failed { error_code, .. } => Some(format!("{error_code:?}")),
                        _ => None,
                    })
                    .unwrap_or_default();
                println!(
                    "request {sent}: {status} {code} {}",
                    reason.as_deref().unwrap_or("")
                );
            }
        }
        if all_described(&pairs) {
            break;
        }
        held = pairs.iter().find_map(|pair| match &pair.state {
            PairState::Failed {
                error_code,
                will_retry: false,
            } => Some(*error_code),
            _ => None,
        });
        if let Some(code) = held {
            println!("stopped: {code:?} holds every request until the settings change");
            break;
        }
        if steps.is_empty() {
            thread::sleep(Duration::from_secs(5));
        }
    }
    for pair in service.memory_changes(&file_id).unwrap() {
        println!(
            "V{} → V{}: {:?}",
            pair.before.sequence, pair.after.sequence, pair.state
        );
        for change in &pair.changes {
            println!(
                "  {:?} [{}] {}",
                change.action, change.topic, change.description
            );
        }
    }

    for question in SEARCHES {
        // The same setup would refuse every search too.
        if let Some(code) = held {
            println!("searches not sent: {code:?}");
            break;
        }
        if sent >= budget {
            println!("budget used up before: {question}");
            break;
        }
        let search = service.start_memory_search(&file_id, question).unwrap();
        let mut view = search;
        while view.state == SearchState::Working && sent < budget {
            sent += 1;
            view = service.run_memory_search(&view.search_id).unwrap();
        }
        println!("search {question:?}: {:?}", view.state);
        service.cancel_memory_search(&view.search_id).ok();
        if let SearchState::Failed {
            error_code,
            retryable: false,
        } = view.state
        {
            held = Some(error_code);
        }
    }
    println!("requests sent: {sent} of {budget}");
    service.set_memory_consent(&file_id, false).unwrap();
}

/// Waits for the watcher to save the file's new version (it saves a version
/// once it has been the same for a while).
fn wait_until_saved(service: &OrganizeService) {
    for _ in 0..60 {
        let checks = service.check_memory().unwrap();
        if checks
            .iter()
            .any(|check| matches!(check.outcome, CheckOutcome::Saved { .. }))
        {
            return;
        }
        thread::sleep(Duration::from_millis(500));
    }
    panic!("the new version was not saved");
}

fn all_described(pairs: &[VersionPairView]) -> bool {
    pairs.iter().all(|pair| {
        matches!(
            pair.state,
            PairState::Analyzed { .. } | PairState::NoTextChange | PairState::Unreadable { .. }
        ) || matches!(
            pair.state,
            PairState::Failed {
                will_retry: false,
                ..
            }
        ) || matches!(
            pair.state,
            PairState::InvalidOutput {
                will_retry: false,
                ..
            }
        )
    })
}

// ---------------------------------------------------------------------------
// #33 final check: versions saved by a real editor (VS Code) while Memory
// records, then described by the real model (student's limit: 12 requests).
// ---------------------------------------------------------------------------

/// Requests the editor check may send (#26 issuecomment-6104680722).
const EDITOR_BUDGET: usize = 12;

/// The synthetic essay the student opens in the editor.
const EDITOR_START: &str = "# Urban heat and street trees\n\nCities can be several degrees warmer than the countryside around them.\nOke 1982 explains this as the urban heat island.\nStreet trees cool the air through shade and evaporation.\nBowler 2010 reports that parks are about one degree cooler than nearby streets.\n";

/// The edits, one save after each: two citation fixes of the same kind,
/// a conclusion added and then changed, a new citation, a new opening.
const EDITS: [&str; 7] = [
    "Line 4: change \"Oke 1982\" to \"Oke (1982, p. 7)\"",
    "At the end, add the line: In conclusion, planting street trees is the cheapest way to cool a city.",
    "Line 6: change \"Bowler 2010\" to \"Bowler et al. (2010, p. 147)\"",
    "Last line: change \"the cheapest way to cool a city\" to \"the cheapest first step to cool a city\"",
    "After line 5, add the line: Shashua-Bar 2009 measured up to 2.5 degrees of cooling under trees.",
    "Line 3: change \"several degrees warmer\" to \"up to 4 degrees warmer at night\"",
    "The Shashua-Bar line: change \"Shashua-Bar 2009\" to \"Shashua-Bar and Hoffman (2009, p. 22)\"",
];

/// Saves made in a real editor, then one request per pair: a pair whose
/// request fails is not sent again (no retry), and the run stops at the
/// limit. Needs the student at the Mac: the editor, and the Keychain's
/// "Allow" when the key is read.
///
/// ```text
/// TRAEVA_MEMORY_LIVE=1 TRAEVA_APP_DATA_DIR=… TRAEVA_MEMORY_LIVE_ROOT=… \
///   TRAEVA_MEMORY_LIVE_BUDGET=12 TRAEVA_MEMORY_LIVE_VERSIONS=6 \
///   cargo test -p traeva-core --test changes_live editor -- --ignored --nocapture
/// ```
#[test]
#[ignore = "sends to the real model and waits for an editor; see above"]
fn changes_from_editor_saves_against_the_real_model() {
    if std::env::var("TRAEVA_MEMORY_LIVE").as_deref() != Ok("1") {
        println!("skipped: TRAEVA_MEMORY_LIVE is not 1, so nothing was sent");
        return;
    }
    env(traeva_core::APP_DATA_DIR_VAR);
    let budget = std::env::var("TRAEVA_MEMORY_LIVE_BUDGET")
        .map(|left| left.parse::<usize>().expect("a number of requests"))
        .unwrap_or(EDITOR_BUDGET)
        .min(EDITOR_BUDGET);
    let wanted = std::env::var("TRAEVA_MEMORY_LIVE_VERSIONS")
        .map(|n| n.parse::<usize>().expect("a number of versions"))
        .unwrap_or(6)
        .clamp(5, EDITS.len() + 1);
    println!("budget: {budget} requests; versions wanted: {wanted}");
    let root = PathBuf::from(env("TRAEVA_MEMORY_LIVE_ROOT"));
    assert!(
        fs::read_dir(&root).unwrap().next().is_none(),
        "{} must be empty",
        root.display()
    );
    let service = OrganizeService::start_for_app();
    let storage = service.get_app_state().unwrap().storage;
    assert!(
        matches!(
            &storage,
            StorageStatus::Ready(info) if info.app_data_source == AppDataSource::Test
        ),
        "not a separate test folder: {storage:?}"
    );

    let essay = root.join("heat.md");
    fs::write(&essay, EDITOR_START).unwrap();
    let root_id = service.authorize_root(&root).unwrap().root_id;
    let file_id = service
        .register_file(&root_id, &essay)
        .unwrap()
        .file
        .file_id;
    let record = service.start_memory_for_file(&file_id).unwrap();
    let chain_id = record.chain_id.clone();
    // An editor may save by writing a new file over the old one: with
    // candidate capture on, that new file is saved as a candidate, and the
    // run confirms it continues the essay (the student saves only this
    // essay, following the list below).
    let info = service.candidate_capture(&chain_id).unwrap();
    service
        .set_candidate_capture(&CandidateSwitchRequest {
            chain_id: chain_id.clone(),
            on: true,
            notice_version: Some(info.notice_version),
            occupant_token: None,
            place_token: Some(info.place_token.clone()),
        })
        .unwrap();

    println!(
        "\nOpen {} in VS Code. Make these edits one at a time;",
        essay.display()
    );
    println!("after each, press Cmd+S and wait for \"version N saved\":");
    for (n, edit) in EDITS.iter().take(wanted - 1).enumerate() {
        println!("  {}. {edit}", n + 1);
    }
    let started = Instant::now();
    let mut versions: i64 = 1;
    while versions < wanted as i64 && started.elapsed() < Duration::from_secs(30 * 60) {
        for check in service.check_memory().unwrap() {
            match check.outcome {
                CheckOutcome::Saved { .. } => {}
                CheckOutcome::CandidateSaved { .. } => {
                    let page = service.pending_candidates(&chain_id).unwrap();
                    let items = page
                        .items
                        .iter()
                        .map(|item| CandidateDecision {
                            candidate_id: item.candidate_id.clone(),
                            choice: CandidateChoice::Continue {
                                parent_snapshot_id: item
                                    .continue_from
                                    .as_ref()
                                    .expect("a version to continue")
                                    .snapshot_id
                                    .clone(),
                            },
                        })
                        .collect();
                    service
                        .decide_pending_candidates(&CandidateDecisionRequest {
                            chain_id: chain_id.clone(),
                            page_token: page.page_token.clone(),
                            items,
                            current_object: None,
                        })
                        .unwrap();
                    println!("(the editor wrote a new file: confirmed as continuing the essay)");
                }
                CheckOutcome::SameBytes { .. } => {
                    println!("saved with no change: not a new version")
                }
                _ => continue,
            }
            let now = service.memory_record(&chain_id).unwrap().line_versions;
            if now > versions {
                versions = now;
                println!("version {versions} saved");
            }
        }
        thread::sleep(Duration::from_millis(500));
    }
    if versions < 2 {
        println!("no version was saved after the first: nothing sent");
        return;
    }

    let settings = service
        .save_model_settings(&ModelSettingsInput {
            provider: traeva_core::PROVIDER_OPENAI.to_string(),
            model: traeva_core::DEFAULT_MODEL.to_string(),
            api_key: None,
        })
        .unwrap();
    println!("model: {:?} {:?}", settings.provider, settings.model);
    service.set_analysis_consent(false).unwrap();
    for watch in service.memory_watches().unwrap() {
        if watch.file_id != file_id && service.memory_consent(&watch.file_id).unwrap().agreed {
            service.set_memory_consent(&watch.file_id, false).unwrap();
        }
    }
    service.set_memory_consent(&file_id, true).unwrap();

    // One request per pair, no retry. Traeva tries a failed pair again by
    // itself a minute after it failed, before pairs still waiting; a pass
    // picks its pair when it starts, so passes started within 50 seconds of
    // the first failure never pick it. After that the run stops, even if
    // some pairs were not sent.
    let mut sent = 0;
    let mut described: Vec<String> = Vec::new();
    let mut first_failure: Option<Instant> = None;
    let started = Instant::now();
    while sent < budget && started.elapsed() < Duration::from_secs(600) {
        let pairs = service.memory_changes(&file_id).unwrap();
        let waiting = pairs
            .iter()
            .filter(|pair| {
                matches!(
                    pair.state,
                    PairState::NotCompared | PairState::Queued | PairState::Analyzing
                )
            })
            .count();
        if waiting == 0 {
            break;
        }
        if first_failure.is_some_and(|at| at.elapsed() >= Duration::from_secs(50)) {
            println!("stopped so a failed pair is not sent again: {waiting} pairs not sent");
            break;
        }
        let steps = service.analyze_memory().unwrap();
        for step in &steps {
            if let PassStep::Described {
                pair_id,
                status,
                reason,
            } = step
            {
                assert!(!described.contains(pair_id), "a pair was sent twice");
                described.push(pair_id.clone());
                sent += 1;
                if status != "analyzed" && first_failure.is_none() {
                    first_failure = Some(Instant::now());
                }
                println!(
                    "request {sent}: {status} {}",
                    reason.as_deref().unwrap_or("")
                );
            }
        }
        if steps.is_empty() {
            thread::sleep(Duration::from_secs(2));
        }
    }
    let pairs = service.memory_changes(&file_id).unwrap();
    for pair in &pairs {
        println!(
            "\n{} v{} → {} v{} (gap: {}): {:?}",
            pair.before.relative_path,
            pair.before.sequence,
            pair.after.relative_path,
            pair.after.sequence,
            pair.gap,
            pair.state
        );
        for change in &pair.changes {
            println!(
                "  {:?} [{}] {}",
                change.action, change.topic, change.description
            );
        }
    }
    println!("\nrequests sent: {sent} of {budget}");
    service.set_memory_consent(&file_id, false).unwrap();
    service.stop_memory_record(&chain_id).unwrap();
    println!("agreement withdrawn and recording stopped");
}
