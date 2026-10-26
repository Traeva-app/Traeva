//! Real-editor check for candidate capture and decisions (#31). The person
//! saves a synthetic file in TextEdit (which saves by replacing the file);
//! this runs the app's own watcher pass and the confirmation steps, and
//! prints what Memory saved and decided. Since the student's 10/14 decision
//! such a save is linked as the next version by itself (auto_link.rs);
//! candidates and the confirmation page only come up when it cannot be.
//!
//!   cargo run -p traeva-core --example memory_family_check -- \
//!       <folder> <file name> <app-data folder> <step> [seconds, default 120]
//!       (for `relocate`: <new name>; for `associate`, `revoke`, `move`:
//!       <other name>, in place of seconds)
//!
//! Steps (run them one after another with the same app-data folder):
//!   watch            turn Memory on for the file (or reuse its record), then
//!                    run the watcher pass every 2 s for [seconds]
//!   candidates-on    show the notice's facts, then turn candidate capture on
//!                    with exactly the token and notice version shown
//!   pending          show the confirmation page
//!   confirm-all      continue every waiting object with the page's own
//!                    parents (refuses when a relation skips objects)
//!   independent-now  judge the object at the place unrelated
//!   relocate         after the file was moved or renamed (in Finder, say),
//!                    show the preview for <new name> (relative to <folder>),
//!                    then confirm exactly that place; later steps use the
//!                    new name
//!   pairs            show the usable version pairs of the work
//!   family           show the work and every gap
//!   hint             show the same-content hints of the file's record (a
//!                    file saved as a copy of a recorded version, then
//!                    turned on with `watch`)
//!   associate        add <other name> (a file with its own record) to this
//!                    file's work as a branch from the version it has the
//!                    same bytes as (or the newest earlier version)
//!   revoke           revoke the relation into <other name>'s first version
//!   move             move <other name> out of this file's work, to a new
//!                    work
//!
//! - <folder> is authorized as a root and must hold only synthetic files.
//! - <app-data folder> must be outside <folder>, and never the real app data.
//!   Use a new one for every run of the check.
//!
//! Nothing is sent anywhere. The file's text is never printed: only hashes,
//! sizes and inode numbers.

use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use traeva_core::{
    AppError, AssociateChoice, AssociationRequest, CandidateChoice, CandidateDecision,
    CandidateDecisionRequest, CandidateSwitchRequest, CheckOutcome, CurrentObjectChoice,
    MEMORY_CHECK_INTERVAL, MemoryRecordView, MoveQuery, MoveRequest, MoveTarget, OrganizeService,
    RecordState, RelationAction, RelationRequest, RelocationRequest,
};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 4 {
        eprintln!(
            "usage: memory_family_check <folder> <file name> <app-data folder> <step> \
             [seconds | new name]"
        );
        return ExitCode::from(2);
    }
    let seconds = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(120);
    match run(
        Path::new(&args[0]),
        &args[1],
        Path::new(&args[2]),
        &args[3],
        Duration::from_secs(seconds),
        args.get(4).map(String::as_str),
    ) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("FAIL: {message}");
            ExitCode::FAILURE
        }
    }
}

fn show(error: AppError) -> String {
    format!("{:?}: {}", error.code, error.message)
}

fn short(sha256: &str) -> String {
    sha256.chars().take(12).collect()
}

fn run(
    folder: &Path,
    name: &str,
    app_data: &Path,
    step: &str,
    run_for: Duration,
    new_name: Option<&str>,
) -> Result<(), String> {
    let folder = folder.canonicalize().map_err(|e| e.to_string())?;
    let file_path: PathBuf = folder.join(name);
    let apart = |app_data: &Path| !app_data.starts_with(&folder) && !folder.starts_with(app_data);
    let overlap = "the app-data folder and the watched folder must not contain each other";
    let absolute = std::env::current_dir()
        .map_err(|e| e.to_string())?
        .join(app_data);
    if !apart(&absolute) {
        return Err(overlap.into());
    }
    std::fs::create_dir_all(&absolute).map_err(|e| e.to_string())?;
    let app_data = absolute.canonicalize().map_err(|e| e.to_string())?;
    if !apart(&app_data) {
        return Err(overlap.into());
    }
    if let Some(home) = std::env::var_os("HOME") {
        let real = Path::new(&home).join("Library/Application Support/Traeva");
        if real.canonicalize().is_ok_and(|real| real == app_data) {
            return Err("this is Traeva's real app-data folder; use a separate one".into());
        }
    }
    let service = OrganizeService::start(app_data);
    let inode = || {
        std::fs::symlink_metadata(&file_path)
            .map(|m| m.ino().to_string())
            .unwrap_or_else(|_| "-".to_string())
    };
    let record = find_record(&service, name)?;
    match (step, record) {
        ("watch", record) => {
            let record = match record {
                Some(record) => {
                    println!("restart: record {} is {:?}", record.chain_id, record.state);
                    record
                }
                None => {
                    let root = service.authorize_root(&folder).map_err(show)?;
                    let record = service
                        .start_memory_at(&root.root_id, &file_path)
                        .map_err(show)?;
                    println!("Memory on: record {}, first version saved", record.chain_id);
                    record
                }
            };
            let started = Instant::now();
            while started.elapsed() < run_for {
                for check in service.check_memory().map_err(show)? {
                    let at = format!("[{:6.1}s]", started.elapsed().as_secs_f64());
                    match check.outcome {
                        CheckOutcome::Unchanged => {}
                        CheckOutcome::Saved { snapshot } => println!(
                            "{at} saved v{} sha {} (inode {})",
                            snapshot.sequence,
                            short(&snapshot.sha256),
                            inode()
                        ),
                        CheckOutcome::SameBytes { .. } => {
                            println!("{at} same bytes again (inode {})", inode())
                        }
                        CheckOutcome::CandidateSaved { snapshot, .. } => println!(
                            "{at} candidate saved: sha {} {} bytes (inode {})",
                            short(&snapshot.sha256),
                            snapshot.byte_length,
                            inode()
                        ),
                        CheckOutcome::CandidatePaused => println!("{at} candidate capture paused"),
                        CheckOutcome::LinkedAutomatically { snapshot } => println!(
                            "{at} linked automatically: next version sha {} (inode {})",
                            short(&snapshot.sha256),
                            inode()
                        ),
                        CheckOutcome::Problem { error } => {
                            println!("{at} not saved: {:?} (inode {})", error.code, inode())
                        }
                        CheckOutcome::Stopped => println!("{at} Memory was turned off"),
                    }
                }
                std::thread::sleep(MEMORY_CHECK_INTERVAL);
            }
            print_record(&service.memory_record(&record.chain_id).map_err(show)?);
        }
        (_, None) => return Err("no record for this file yet: run the watch step first".into()),
        ("candidates-on", Some(record)) => {
            let info = service.candidate_capture(&record.chain_id).map_err(show)?;
            println!(
                "notice v{}: place {}; holds {:?}; saves the object there now: {}; quota {}/{} \
                 objects, {}/{} versions",
                info.notice_version,
                info.place.display_path,
                info.place_holds,
                info.saves_occupant_now,
                info.pending.objects,
                info.pending.limit,
                info.pending.snapshots,
                info.pending.limit
            );
            let on = service
                .set_candidate_capture(&CandidateSwitchRequest {
                    chain_id: record.chain_id.clone(),
                    on: true,
                    notice_version: Some(info.notice_version),
                    occupant_token: info.unread_occupant.map(|o| o.occupant_token),
                    place_token: Some(info.place_token.clone()),
                })
                .map_err(show)?;
            println!("candidate capture: {:?}", on.candidate_capture);
        }
        ("pending", Some(record)) => {
            let page = service.pending_candidates(&record.chain_id).map_err(show)?;
            println!(
                "page {}: {} waiting, quota {}/{} objects, {}/{} versions",
                &page.page_token[..12],
                page.items.len(),
                page.pending.objects,
                page.pending.limit,
                page.pending.snapshots,
                page.pending.limit
            );
            for (n, item) in page.items.iter().enumerate() {
                let last = item.snapshots.last();
                println!(
                    "  waiting file {}: {} versions, last sha {}, at place {}, continue from {} \
                     (skips {}), needs {}, not sure earlier: {}",
                    n + 1,
                    item.snapshots.len(),
                    last.map_or("-".into(), |s| short(&s.sha256)),
                    item.at_place,
                    item.continue_from
                        .as_ref()
                        .map_or("-".into(), |s| short(&s.sha256)),
                    item.continue_skips,
                    item.continue_needs.is_some(),
                    item.deferred
                );
            }
            if let Some(unread) = &page.unread_occupant {
                println!(
                    "  unread object at the place: {} bytes, continue from {}",
                    unread.occupant.byte_length,
                    unread
                        .continue_from
                        .as_ref()
                        .map_or("-".into(), |s| short(&s.sha256))
                );
            }
        }
        ("confirm-all", Some(record)) => {
            let page = service.pending_candidates(&record.chain_id).map_err(show)?;
            let mut items = Vec::new();
            for item in &page.items {
                if item.continue_skips > 0 {
                    return Err("a relation skips objects: decide on the page instead".into());
                }
                let parent = item
                    .continue_from
                    .as_ref()
                    .ok_or("an object has nothing to continue from")?;
                items.push(CandidateDecision {
                    candidate_id: item.candidate_id.clone(),
                    choice: CandidateChoice::Continue {
                        parent_snapshot_id: parent.snapshot_id.clone(),
                    },
                });
            }
            let current_object = page.unread_occupant.as_ref().and_then(|unread| {
                unread
                    .continue_from
                    .as_ref()
                    .map(|parent| CurrentObjectChoice::Continue {
                        parent_snapshot_id: parent.snapshot_id.clone(),
                    })
            });
            let outcome = service
                .decide_pending_candidates(&CandidateDecisionRequest {
                    chain_id: record.chain_id.clone(),
                    page_token: page.page_token,
                    items,
                    current_object,
                })
                .map_err(show)?;
            println!(
                "decided: watch moved {}, history only {}",
                outcome.continued_watch.is_some(),
                outcome.history_only
            );
        }
        ("independent-now", Some(record)) => {
            let page = service.pending_candidates(&record.chain_id).map_err(show)?;
            let at_place = page.items.iter().find(|item| item.at_place);
            let request = CandidateDecisionRequest {
                chain_id: record.chain_id.clone(),
                page_token: page.page_token.clone(),
                items: at_place
                    .map(|item| CandidateDecision {
                        candidate_id: item.candidate_id.clone(),
                        choice: CandidateChoice::Independent {},
                    })
                    .into_iter()
                    .collect(),
                current_object: (at_place.is_none() && page.unread_occupant.is_some())
                    .then_some(CurrentObjectChoice::Independent {}),
            };
            let outcome = service.decide_pending_candidates(&request).map_err(show)?;
            println!("record is now {:?}", outcome.record.state);
        }
        ("relocate", Some(record)) => {
            let new_name = new_name.ok_or("relocate needs the file's new name")?;
            let shown = service
                .relocation_preview(
                    &record.chain_id,
                    &record.place.root_id,
                    &folder.join(new_name),
                )
                .map_err(show)?;
            println!(
                "preview: the old place holds {:?}; new place {}; {} waiting candidates stay; \
                 candidate capture {:?}",
                shown.old_place_holds,
                shown.new_place.relative_path,
                shown.waiting_candidates,
                shown.candidate_capture
            );
            let outcome = service
                .relocate_record(&RelocationRequest {
                    chain_id: record.chain_id.clone(),
                    root_id: shown.new_place.root_id.clone(),
                    relative_path: shown.new_place.relative_path.clone(),
                    page_token: shown.page_token.clone(),
                })
                .map_err(show)?;
            let new_inode = std::fs::symlink_metadata(folder.join(new_name))
                .map(|m| m.ino().to_string())
                .unwrap_or_else(|_| "-".to_string());
            println!(
                "relocated: version sha {} saved at the new place (inode {new_inode})",
                short(&outcome.snapshot.sha256)
            );
            print_record(&outcome.record);
        }
        ("pairs", Some(record)) => {
            for pair in service
                .usable_pairs_of_family(&record.family_id)
                .map_err(show)?
            {
                println!(
                    "{} -> {}: {:?}/{:?}, gap {:?}, unplaced {}, permission files {}",
                    short(&pair.before.sha256),
                    short(&pair.after.sha256),
                    pair.basis,
                    pair.evidence,
                    pair.gap_kinds,
                    pair.unplaced,
                    pair.permission_file_ids.len()
                );
            }
        }
        ("family", Some(record)) => {
            let family = service.memory_family(&record.family_id).map_err(show)?;
            println!(
                "work {}: {} files, {} relations",
                family.family_id,
                family.members.len(),
                family.relations.len()
            );
            for member in &family.members {
                println!(
                    "  file {}: {:?} ({:?}), placed {}, {} versions",
                    member.file_id,
                    member.status,
                    member.basis,
                    member.placed,
                    member.snapshots.len()
                );
            }
            for gap in &family.gaps {
                println!(
                    "  gap {:?}{}: {} to {}",
                    gap.kind,
                    gap.problem.map_or(String::new(), |p| format!(" ({p:?})")),
                    gap.started_at_ms,
                    gap.ended_at_ms.map_or("now".into(), |at| at.to_string())
                );
            }
        }
        ("hint", Some(record)) => {
            if record.same_content.is_empty() {
                println!("no hint: no earlier version of another work has these bytes");
            }
            for hint in &record.same_content {
                println!(
                    "hint: own v{} sha {} has the bytes of {} v{} in work {} (only a hint, no \
                     relation)",
                    hint.own_version.sequence,
                    short(&hint.own_version.sha256),
                    hint.same_bytes_as.relative_path,
                    hint.same_bytes_as.sequence,
                    hint.family_id
                );
            }
        }
        ("associate", Some(record)) => {
            let other_name = new_name.ok_or("associate needs <other name>")?;
            let other = find_record(&service, other_name)?
                .ok_or("the other file has no record on: run `watch` for it first")?;
            let shown = service
                .association_preview_for_file(&record.family_id, &other.watch.file_id)
                .map_err(show)?;
            println!(
                "preview: standing {:?}, choices {:?}, same bytes as {} version(s), {} fork \
                 points, merges work {:?}",
                shown.standing,
                shown.choices,
                shown.evidence.same_bytes_as.len(),
                shown.fork_points.len(),
                shown.other_family_id
            );
            let parent = shown
                .evidence
                .same_bytes_as
                .iter()
                .rev()
                .find(|s| shown.fork_points.contains(s))
                .or(shown.fork_points.last())
                .ok_or("no earlier version to branch from")?;
            let outcome = service
                .associate_file(&AssociationRequest {
                    family_id: shown.family_id.clone(),
                    subject: shown.request_subject.clone(),
                    choice: AssociateChoice::Branch {
                        parent_snapshot_id: parent.snapshot_id.clone(),
                    },
                    page_token: shown.page_token.clone(),
                })
                .map_err(show)?;
            println!(
                "associated as a branch from {} v{}: the work now has {} files, merged {:?}",
                parent.relative_path,
                parent.sequence,
                outcome.family.members.len(),
                outcome.family.merged_family_ids
            );
        }
        ("revoke", Some(record)) => {
            let other_name = new_name.ok_or("revoke needs <other name>")?;
            let family = service.memory_family(&record.family_id).map_err(show)?;
            let relation = family
                .relations
                .iter()
                .find(|r| r.child.relative_path == other_name && r.parent.is_some())
                .ok_or("no relation into that file's first version")?;
            let shown = service.relation_preview(&relation.edge_id).map_err(show)?;
            println!(
                "preview: {:?} {:?}, revoking pauses {} record(s)",
                shown.relation.kind,
                shown.relation.status,
                shown.records_held.len()
            );
            let outcome = service
                .correct_relation(&RelationRequest {
                    edge_id: relation.edge_id.clone(),
                    action: RelationAction::Revoke,
                    page_token: shown.page_token,
                })
                .map_err(show)?;
            println!("revoked: {} record(s) paused", outcome.held_records.len());
        }
        ("move", Some(record)) => {
            let other_name = new_name.ok_or("move needs <other name>")?;
            let family = service.memory_family(&record.family_id).map_err(show)?;
            let member = family
                .members
                .iter()
                .find(|m| m.display_path.ends_with(&format!("/{other_name}")))
                .ok_or("that file is not a member of this work")?;
            let query = MoveQuery {
                family_id: family.family_id.clone(),
                file_ids: vec![member.file_id.clone()],
                target: MoveTarget::NewWork {},
            };
            let shown = service.move_preview(&query).map_err(show)?;
            println!(
                "preview: {} record(s) go along, {} relation(s) kept, {} disputed, {} \
                 record(s) pause",
                shown.records_moving.len(),
                shown.relations_kept.len(),
                shown.relations_disputed.len(),
                shown.records_held.len()
            );
            let outcome = service
                .move_members(&MoveRequest {
                    family_id: query.family_id,
                    file_ids: query.file_ids,
                    target: query.target,
                    page_token: shown.page_token,
                })
                .map_err(show)?;
            println!(
                "moved to the new work {:?}: {} file(s) stay here",
                outcome.other_family_id,
                outcome.family.members.len()
            );
        }
        (other, Some(_)) => return Err(format!("unknown step {other}")),
    }
    Ok(())
}

/// The record whose place is <file name> (the newest one).
fn find_record(service: &OrganizeService, name: &str) -> Result<Option<MemoryRecordView>, String> {
    Ok(service
        .memory_records()
        .map_err(show)?
        .into_iter()
        .find(|record| record.place.relative_path == name && record.state != RecordState::Stopped))
}

fn print_record(record: &MemoryRecordView) {
    println!(
        "record {:?}: candidate capture {:?}, waiting {} objects / {} versions, open gaps {:?}",
        record.state,
        record.candidate_capture,
        record.pending.objects,
        record.pending.snapshots,
        record.open_gaps.iter().map(|g| g.kind).collect::<Vec<_>>()
    );
}
