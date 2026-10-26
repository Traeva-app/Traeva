# Organize v0.3 acceptance coverage list (#20)

> Historical v0.3 developer coverage record at `ddfb16d`. Scope, pending review and acceptance criteria below belong to that baseline. Later revisions, including additional formats and up to three clarification questions, require their own evidence and do not inherit these PASS results. Current behavior is in [Organize rules](../features/organize-spec.md); final acceptance is tracked in [#37](https://github.com/P30g53/Traeva/issues/37).

Maps each acceptance row of [organize-spec.md §10](../features/organize-spec.md#10-acceptance-ids-and-deferral-mapping) to tests and evidence. This is not a frozen document, and not a Gate conclusion: every PASS here is a result the **implementer** actually ran on the versions stated below; Gate 3 (#23) still needs independent re-verification by Codex-R.

- **Current scope, 33 rows**: O01–O14, O17, O18-R, O18-C, O19–O26, O29–O36.
- **DEFERRED, 4 rows**: O15 learning, O16 analysis cache, O27 watcher dedup, O28 Source references. Deferred with their features per §10; not recorded as PASS, and not counted as failures.
- **Product code version**: `main` `ddfb16d`. The #20 and #21 branches both only add tests and documents and do not change product code, so Gate 2's real-window evidence at `ddfb16d` is valid for the same product code.
- **Automated test version**: the `m20-coverage` branch (test commit `95cd8c0`), local evidence set `m20`, not included in this source package. O18-C is on `m21-crash-checkpoints` (#21), evidence in `traeva-evidence/m21/`.
- **Test environment**: macOS 27.0.1 arm64; the volume holding the temporary folders is APFS (case-insensitive, normalization-insensitive), so O09's case and NFD sub-cases really ran and did not take the "skip if the volume is case-sensitive" branch.

## How to read this

| Notation | Meaning |
|---|---|
| PASS | the listed tests actually ran on the versions above and passed |
| Not tested | this sub-item has not been run; not counted as PASS |
| BLOCKED | the required conditions are not available yet (for example sealed material, real cloud-drive placeholder files) |
| DEFERRED | explicitly deferred by spec §10 |

Test kinds: **U** = core unit tests (`crates/traeva-core/src/*_tests.rs`), **I** = core integration tests (`crates/traeva-core/tests/`), **IPC** = `src-tauri/tests/`, **FE** = frontend `npm test`, **W** = real packaged `.app` window, **M** = real OpenAI requests, **C** = real abnormal process termination. Items marked *(new)* are regressions added by #20.

Success criteria: execution tests all check the files on disk directly (byte hash, inode, directory tree snapshots) and the database rows (actions, action_events, path_history, files); a return value, process exit code or model answer is never taken as success.

## Summary table

| ID | Result to verify | Kind | Status |
|---|---|---|---|
| O01 | same object, same UUID; same-content copy, different UUID; external replacement/external move not linked | U I | PASS |
| O02 | clear recommendation of an existing folder, reasons shown, no root write before confirmation | U I W M | PASS |
| O03 | asks about only one missing dimension, answer locked, at most once, still unclear goes to manual pick | U FE W M | PASS |
| O04 | a high score without grounds or conflicting multiple candidates cannot become a confirmable guess | U FE | PASS |
| O05 | full preview of two levels of new folders; root snapshots at every stage before confirmation show no writes | U I W | PASS (the two-level new folder preview card in a real window not tested) |
| O06 | changing the destination gives a new plan and invalidates the old plan; cancelled/expired cannot run; revoking and re-authorizing does not revive | U I | PASS |
| O07 | a same-length content change with mtime restored is still judged stale by the full hash | U I | PASS |
| O08 | an external rename, same-path replacement or same-content copy invalidates the old plan | U I | PASS |
| O09 | late occupation is still no-replace, including case/Unicode equivalence | U I | PASS |
| O10 | fd level by level, not following symlinks; no success reported when the chain is replaced or moved out of the root | U I | PASS |
| O11 | revoked/permission/cross-volume/hard link/special file/placeholder file refused; app-data does not overlap in either direction | U I | PASS (real cloud-drive placeholder files not tested) |
| O12 | project/package/hidden config protection, name rules, no write probes | U I | PASS |
| O13 | invalid model JSON/field/ID/path/overwritten answer refused, with no move | U | PASS |
| O14 | offline/timeout/API failure can retry or manual pick, no automatic apply | U FE | PASS (offline controlled provider; real service failures not tested) |
| O15 | learning | — | DEFERRED |
| O16 | analysis cache | — | DEFERRED |
| O17 | no change if the journal is not on disk; mkdir/move failure states are true; no unaccounted move | U I | PASS |
| O18-R | recovery at four checkpoints by service rebuild/same-DB reopen | U | PASS (recovery-logic, not a process crash) |
| O18-C | recovery at four checkpoints after a real abnormal termination of a child process | C | PASS (#21, PR #81 awaiting merge) |
| O19 | after a move, a mismatch of either end/hash/physical evidence/chain → needs_user_decision | U I | PASS |
| O20 | at most one action per plan; retry after failure needs a new plan; a failed confirmation transaction leaves no root write | U I | PASS |
| O21 | normal move, restart, confirm the reverse plan for Undo; bytes/hash/UUID match | U I C W | PASS |
| O22 | at Undo, content changed, moved out, replaced, original name taken or parent folder replaced: all refused | U I | PASS |
| O23 | only cleans up the same folder that has a receipt and is still empty; .DS_Store kept; the warning does not block | U I | PASS |
| O24 | repeated Undo: no extra move or record | U I | PASS |
| O25 | injection in the content is only data; the model has no file execution ability | U FE | PASS (offline); real-model injection resistance belongs to #22, BLOCKED |
| O26 | truncated/unsupported/extraction failed stated truthfully; over 20 MiB blocked even for manual pick | U I W | PASS |
| O27 | watcher dedup | — | DEFERRED |
| O28 | Source references | — | DEFERRED |
| O29 | Chinese/emoji/BOM/CRLF/invalid-encoding boundaries; hash over the raw bytes | U I | PASS |
| O30 | 8,000-character/20 MiB boundaries and omission placement; no pretending to understand the full text | U | PASS |
| O31 | bounded wait for changes during reading; the request excerpt comes from the same source as the hash; no claim that the writer finished | U | PASS |
| O32 | IPC rejects forged IDs/paths/unknown fields/windows; the default DB/logs hold no content, raw prompt or secret; the agreed metadata is really saved | U IPC FE W | PASS (one item awaiting a decision, see below) |
| O33 | cancel, repeated answer, late response, refresh/restart do not apply twice; unconsumed plans are invalidated on restart | U I | PASS (window refresh has only static and core guarantees) |
| O34 | DB not writable/corrupted/higher schema/second instance stops new operations; no database deletion, no double write | I | PASS |
| O35 | same-location no-op has no action; a cleanup failure does not erase the restore; recovery does not roll back user files | U I | PASS |
| O36 | core independent of Tauri; thin commands; a real window and at least one closed loop in a packaged .app | I IPC W | PASS |

A production completion condition that sits alongside the 33 rows but is not one of them: the **independent 7-case semantic evaluation** (organize-spec §11, T16 in the plan = #22). The material is sealed and the implementer has not viewed or generated it; **BLOCKED**, to be run separately by the student after unsealing per AGENTS.md §3 (Gate 2 has PASSed and the prompt/schema are frozen, so the unsealing conditions are met).

## Evidence row by row

Path prefixes: `S/` = `crates/traeva-core/src/`, `T/` = `crates/traeva-core/tests/`, `P/` = `crates/traeva-core/src/platform/tests.rs`.

### O01 Identity
- Re-registering the same object gives the same UUID: `T/observe.rs::the_same_current_file_keeps_its_id`; editing in place keeps the same UUID and raises the revision by one: `::editing_in_place_keeps_the_id_and_raises_the_revision`.
- A same-content copy gets a different UUID: `T/observe.rs::a_copy_with_the_same_content_gets_its_own_id`; database constraint `T/schema.rs::a_location_holds_at_most_one_present_file`.
- External replacement is not linked: `T/observe.rs::a_replaced_file_gets_a_new_id_and_the_old_record_goes_missing`.
- An external move (same inode) is a new file at the new place, not linked by inode/hash: `T/observe.rs::a_file_moved_outside_traeva_gets_a_new_id_at_its_new_place` *(new)*.
- Only moves recorded by this app are recognized: `T/undo.rs::organize_then_undo_puts_the_same_file_back` (re-registering gives the same file_id, history intake/organize/undo).

### O02 Clear recommendation
- `S/analysis_tests.rs::a_clear_file_is_recommended_once_and_a_double_start_sends_nothing_more`: RecommendationReady, Travel, 1 request, snapshot unchanged; *(new)* asserts that the summary, reasons, cited E1 and score reach the SuggestionView the window uses.
- A recommendation becomes a plan only through build_plan, and the choice is recorded: `S/analysis_tests.rs::a_recommendation_becomes_a_plan_only_through_build_plan_with_its_choice_recorded`; `S/classify_tests.rs::a_valid_answer_is_checked_kept_and_still_only_a_suggestion`.
- Zero writes in the whole flow before confirmation: `S/analysis_tests.rs::the_whole_flow_writes_nothing_before_confirmation`.
- **W + M**: Gate 2 final acceptance (R, `main@ddfb16d`): the new sample circuit-lab.md was recommended School/Physics directly, the preview showed the model's reasons, and the whole root's metadata was unchanged during the recommendation and preview stages (#19 comment 6074407516). #18 demo (`85ed78d`): titration-lab.txt → School/Chemistry.

### O03 One clarification question
- Question shape and the single dimension: `S/analysis_tests.rs::the_one_question_is_answered_locked_and_sent_once_more`, `S/classify_tests.rs::the_question_has_a_fixed_shape`, `::the_fields_must_fit_together`.
- Answer locked, sent only once, as data: `S/analysis_tests.rs::answers_are_checked_locked_and_never_resent_or_changed`, `S/classify_tests.rs::the_second_request_carries_the_locked_answer_as_data`.
- At most one round; still unclear goes to manual pick: `S/analysis_tests.rs::still_unsure_after_the_answer_goes_to_a_manual_choice_never_a_second_question`, `::a_spare_question_is_asked_first_and_never_twice`; a retry keeps the locked answer: `::a_failed_step_can_be_retried_with_the_locked_answer_and_no_new_question`.
- Frontend answer shape: FE `flow.test.ts` “an answer is one option or the user's own words, never both”.
- **W + M**: in Gate 2 final acceptance, seedling-check.txt asked about the purpose only once, and after a fixed answer recommended Community Garden, with no second question; #18 frankenstein-notes.md, after asking only once → Book Club.

### O04 Guesses that cannot be confirmed
- `S/analysis_tests.rs::the_routing_table_holds_row_by_row` (0.80/0.79 boundary, null, no grounds, two candidates, abstain, uncertain dimension), `::without_a_question_unclear_results_go_to_manual_with_no_extra_request`, `::offered_candidates_in_a_manual_choice_are_recorded_as_manual`.
- The frontend does not preselect: FE “only a recommendation has a recommended candidate; offers have none”.

### O05 Two-level preview and zero writes
- Exact preview of two levels of new folders: `T/plans.rs::a_plan_previews_exactly_what_will_happen_and_writes_nothing` (destination, display_path, basename, folders_to_create, database columns).
- Two levels of new folders given by the model become a full preview and are not created before confirmation: `S/analysis_tests.rs::a_recommended_new_folder_path_is_previewed_in_full_and_not_created` *(new)*.
- Tree snapshots at every stage: authorization `T/roots.rs::authorizing_reads_only_and_saves_an_active_root`; refusal `T/roots.rs::protected_and_unsupported_folders_are_refused_without_writing`, `T/observe.rs::files_that_cannot_be_organized_are_refused_and_not_recorded`; reading `T/observe.rs::registering_reads_only_and_records_identity_location_and_hash`, `S/observe_tests.rs::stable_reads_write_nothing`; folder list `T/observe.rs::listing_folders_reads_only_and_returns_a_bounded_tree`; analysis, answer, preview `S/analysis_tests.rs::the_whole_flow_writes_nothing_before_confirmation`; pre-execution checks `S/plan_tests.rs::a_fresh_plan_passes_every_check_and_nothing_is_written`; Undo preview `T/undo.rs::organize_then_undo_puts_the_same_file_back`; failed confirmation transaction `T/execute.rs::a_failed_confirmation_changes_nothing_and_the_plan_stays_usable`. The snapshots compare path, type, bytes, mode, link target, xattr, mtime, ctime (not atime).
- **W**: in Gate 2 final acceptance, for both samples the whole root was unchanged from the start through the preview. **Not tested**: the "two levels of new folders" preview card in a real window (R's sample destinations were existing folders).

### O06 Plan lifecycle
- Changing the destination: `T/plans.rs::choosing_again_supersedes_the_old_plan_and_cancelling_is_final`, *(new)* confirming a superseded or cancelled plan returns StalePlan, 0 actions, tree unchanged; `T/schema.rs::plan_payload_cannot_change_but_lifecycle_can`.
- Expiry: `S/execute_tests.rs::an_expired_plan_creates_no_action_and_changes_nothing`, `S/plan_tests.rs::an_expired_plan_is_refused_and_its_lifecycle_is_not_changed`, `::a_clock_set_back_makes_a_plan_expire`.
- Revoking and re-authorizing does not revive: `S/plan_tests.rs::revoking_a_root_ends_its_plans_and_authorizing_again_does_not_revive_them`, *(new)* confirming the old plan after re-authorization gives StalePlan, 0 actions, tree unchanged.
- The same location ends the old plan: `S/plan_tests.rs::choosing_the_place_the_file_is_in_ends_the_earlier_plan` (Gate 2 #71).
- A used plan is final: `S/plan_tests.rs::a_used_plan_is_final`; after a cancel in the page flow only the new plan runs: `S/analysis_tests.rs::the_page_flow_runs_through_the_same_plan_preview_and_executor`.

### O07 Same-length edit with mtime restored
- After the preview: `S/plan_tests.rs::an_equal_length_edit_with_restored_mtime_invalidates_the_plan`; *(new)* at confirmation: `S/execute_tests.rs::a_file_changed_after_the_preview_is_refused_at_the_confirmation` (FileChanged, 0 actions, tree unchanged).
- After confirmation: `S/execute_tests.rs::an_edit_after_the_confirmation_stops_the_move`; after the move: `::a_change_right_after_the_move_needs_a_decision_and_nothing_moves_back`, `::an_edit_right_after_the_final_read_needs_a_decision`.
- Read primitive: `S/observe_tests.rs::an_equal_length_change_with_restored_mtime_is_still_caught`.

### O08 Rename, replacement, copy
- `S/plan_tests.rs::a_moved_or_replaced_file_invalidates_the_plan`; *(new)* rename and same-byte atomic replacement at confirmation: `S/execute_tests.rs::a_file_changed_after_the_preview_is_refused_at_the_confirmation`.
- A same-content copy takes the original place after confirmation: `S/execute_tests.rs::a_folder_replaced_after_the_confirmation_stops_the_move` (second part).
- Rename between the check and the syscall: `S/execute_tests.rs::a_failed_move_with_the_file_gone_from_its_old_name_needs_a_decision`.
- Registration and analysis side: `T/observe.rs::a_replaced_file_gets_a_new_id_and_the_old_record_goes_missing`, `S/analysis_tests.rs::a_file_changed_or_a_folder_revoked_while_waiting_drops_the_result`.

### O09 no-replace and late occupation
- Occupation by the same name, by case or by NFD after confirmation: `S/execute_tests.rs::a_file_put_at_the_target_just_before_the_move_is_never_replaced` (occupant's bytes unchanged, source hash unchanged, 1 action, confirming again returns the same action).
- Primitives: `P::move_no_replace_refuses_other_spellings_of_a_taken_name`, `P::move_no_replace_refuses_unicode_equivalent_target`, `P::move_no_replace_refuses_a_target_that_appears_late`, `P::move_no_replace_refuses_links_and_folders_at_the_target`.
- Occupation after/at the preview: `S/plan_tests.rs::a_target_taken_after_the_preview_invalidates_the_plan`, `T/plans.rs::taken_names_are_conflicts_never_overwrites`; late folders: `S/execute_tests.rs::a_folder_someone_else_creates_is_not_claimed`, `P::create_folder_never_claims_an_existing_name`, `P::create_folder_respects_unicode_equivalent_names`.

### O10 fd chain and symlinks
- Not following symlinks, accepting single names only: `P::walk_never_follows_symbolic_links`, `P::walk_takes_single_names_only`, `T/observe.rs::files_that_cannot_be_organized_are_refused_and_not_recorded` (outside-link), IPC `input_that_does_not_fit_the_contract_is_refused`.
- Source chain replaced by same-name folders after the preview (same file object): `S/plan_tests.rs::a_file_moved_into_look_alike_folders_ends_the_plan` *(new)*; destination chain: `S/plan_tests.rs::a_replaced_target_folder_invalidates_the_plan`.
- Destination folder replaced after confirmation by a link pointing outside the root: `S/execute_tests.rs::a_target_folder_turned_into_a_link_to_outside_stops_the_move` *(new)* (the folder outside the root stays empty).
- Destination folder moved out of the root between the last check and the move call: `S/execute_tests.rs::a_target_folder_moved_out_of_the_root_just_before_the_move_needs_a_decision` *(new)*. The file follows the handle to outside the root; this is exactly the race spec §2.1 acknowledges: the system detects it, records needs_user_decision, does not report success, does not move it back, and the record still points to the original location.
- Others: `S/execute_tests.rs::a_folder_replaced_after_the_confirmation_stops_the_move`, `::a_new_folder_swapped_right_after_mkdir_is_not_claimed`, `::a_folder_leaving_the_root_during_the_final_read_needs_a_decision`, `::a_root_moved_away_during_the_final_read_needs_a_decision`, `::a_folder_moved_during_the_move_needs_a_decision`.

### O11 Refusals and app-data isolation
- Revoked: `T/observe.rs::a_revoked_root_cannot_be_used`, `S/execute_tests.rs::revoking_a_root_during_a_run_is_noticed_before_and_after_the_move`, `S/undo_tests.rs::a_revoked_folder_blocks_the_undo`.
- Permission: `S/execute_tests.rs::a_move_the_system_refuses_is_failed_no_move_after_looking_at_both_names`; file becomes unreadable: `S/plan_tests.rs::a_file_that_cannot_be_read_any_more_is_not_moved` *(new)*.
- Cross-volume (opt-in, mounts a temporary disk image, `cargo test -p traeva-core -- --ignored`): `P::move_across_volumes_is_refused_and_moves_nothing`, `P::walk_does_not_cross_into_another_volume`, `P::roots_on_hfs_volumes_are_refused`, `T/roots.rs::only_local_apfs_volumes_can_hold_a_root`, `T/plans.rs::a_plan_to_another_volume_is_refused_and_nothing_is_written` *(new)*.
- Hard links and special files: `S/observe_tests.rs::refused_files_are_never_opened`, `P::only_plain_single_link_visible_files_are_movable`; hard link and FIFO at registration: `T/observe.rs::files_that_cannot_be_organized_are_refused_and_not_recorded` *(two new items)*; hard link added after the preview/after confirmation: `S/execute_tests.rs::a_file_that_gets_a_second_name_is_not_moved` *(new)*.
- Placeholder files: `P::cloud_placeholders_are_refused` only sets SF_DATALESS by hand on the pure check. **Not tested**: real iCloud/File Provider placeholder files (needs a cloud-drive environment).
- app-data overlap in either direction and cases that cannot be determined: `T/roots.rs::a_root_may_not_contain_or_be_inside_the_app_data_folder`, `T/startup.rs::a_data_folder_moved_into_an_authorized_folder_blocks_the_start_before_any_write`, `::a_data_folder_that_cannot_be_checked_blocks_the_start`; app-data moved into a root while running: `T/roots.rs::a_data_folder_moved_into_a_root_while_traeva_runs_stops_every_use_of_it` *(new)*.

### O12 Protection, names, no probes
- Every project marker actually placed on disk (`.git` file and folder, `.hg`, `.svn`, `package.json`, `Cargo.toml`, `pom.xml`, `build.gradle`, `CMakeLists.txt`, `.xcodeproj`, `.xcworkspace`): `P::every_project_marker_on_disk_makes_its_folder_a_project` *(new)*.
- Packages, hidden config: `P::walk_refuses_hidden_package_project_and_non_folders`, `P::package_names_are_recognized_by_extension`, `S/observe_tests.rs::the_catalog_is_breadth_first_and_skips_what_it_must`, `T/roots.rs::protected_and_unsupported_folders_are_refused_without_writing`, `T/roots.rs::every_use_checks_the_rules_again`.
- Destination becomes a project after the preview: `S/plan_tests.rs::a_target_that_became_a_project_folder_is_not_used` *(new)*.
- A folder whose contents cannot be listed cannot be a destination: `T/plans.rs::a_folder_that_cannot_be_listed_is_never_a_target` *(new)* (it does not appear in the new list; confirming the old plan is refused).
- Name rules: `P::new_folder_names_follow_the_spec`, `P::project_marker_names_cannot_be_new_folders`, `T/plans.rs::bad_folder_names_are_refused_not_renamed`, `S/plan_tests.rs::new_folder_names_are_checked_never_changed`, `S/classify_tests.rs::new_folder_names_follow_the_naming_rules_and_the_list`.
- No write probes: `P::walking_and_checking_write_nothing` and O05's snapshot tests.

### O13 Invalid model output
- `S/model_tests.rs::answers_without_one_valid_structured_output_are_invalid`; `S/classify_tests.rs::the_materials_invalid_examples_are_refused`, `::every_field_must_be_there_with_the_right_type`, `::the_fields_must_fit_together`, `::a_candidate_cites_at_most_five_text_pieces`, `::new_folder_names_follow_the_naming_rules_and_the_list`, `::limits_are_counted_in_characters_and_hold_exactly_at_the_edge`, `::a_valid_answer_too_large_to_keep_is_rejected`, `::long_copies_of_the_file_text_are_refused`, `::new_folder_names_never_carry_the_file_text` (Gate 2 #69), `::refusals_never_repeat_the_answers_text`.
- Folder IDs that are in the list but were not offered to the model (project folders, the 101st and later): `S/classify_tests.rs::a_listed_folder_that_was_not_offered_is_refused` *(new)*.
- After a refusal, no folder is chosen, no plan is built, the root is untouched, and manual pick is available: `S/classify_tests.rs::an_invalid_answer_is_rejected_and_nothing_is_chosen`.
- Overwriting the locked answer: `S/analysis_tests.rs::answers_are_checked_locked_and_never_resent_or_changed`, `::still_unsure_after_the_answer_goes_to_a_manual_choice_never_a_second_question`.

### O14 API failures
- `S/model_tests.rs::a_slow_answer_times_out_and_is_not_retried`, `::an_unreachable_provider_fails_and_is_not_retried`, `::http_errors_get_traevas_own_messages_and_are_not_retried`, `::a_redirect_is_not_followed`, `::an_answer_over_64_kib_is_dropped`, `::a_request_over_64_kib_is_not_sent`, `::the_app_talks_only_to_openai_over_https_with_the_spec_limits`, `::nothing_is_sent_without_settings_consent_and_a_key`.
- Operation layer: after 503, 429, unreachable and timeout, the error code the window sees matches the analyses record, Try again and Choose folder are offered, 0 plans, 0 actions, tree unchanged, and a manual pick works afterwards: `S/analysis_tests.rs::a_failed_request_offers_retry_or_a_manual_choice_and_changes_nothing` *(new)*.
- Retry and manual pick: `S/analysis_tests.rs::a_failed_step_can_be_retried_with_the_locked_answer_and_no_new_question`, `::an_invalid_answer_can_be_retried_but_a_rejected_key_cannot`; FE “a retry is offered only when the core's error says so”.
- These failures are all simulated by a local controlled provider; real OpenAI service failures are **not tested**.

### O17 Record first, then act
- Order: `S/execute_tests.rs::every_change_has_a_saved_intent_before_it_and_a_result_after_it`; a successful move call is not yet a success: `::a_successful_move_call_is_not_yet_a_success`.
- Both halves of the confirmation transaction may fail: `T/execute.rs::a_failed_confirmation_changes_nothing_and_the_plan_stays_usable` (actions insert fails), `::a_confirmation_whose_journal_entry_fails_changes_nothing` *(new)* (confirm journal entry fails).
- Nothing is done if the intent cannot be written: `T/execute.rs::no_folder_is_created_without_its_saved_intent`, `::no_move_happens_without_its_saved_intent`; not claimed if the receipt cannot be written: `::a_folder_whose_receipt_cannot_be_saved_is_not_claimed`; when even the failure cannot be recorded: `::when_even_the_failure_cannot_be_recorded_the_storage_error_is_returned`; final transaction fails: `::a_failed_final_record_needs_a_decision_and_the_move_never_repeats`.
- macOS refuses mkdir: `S/execute_tests.rs::a_new_folder_the_system_refuses_ends_the_run_without_a_move` *(new)*; refuses move: `::a_move_the_system_refuses_is_failed_no_move_after_looking_at_both_names`; EEXIST: `::a_folder_someone_else_creates_is_not_claimed`.
- Durability PRAGMAs read back: `T/schema.rs::ready_state_reports_the_pragmas_sqlite_actually_uses`.
- Branch covered only by code review: when the move call's rename succeeds but confirming it on disk fails (`ChangedButUnconfirmed`), the executor records needs_user_decision; the primitive layer has `P::confirm_moved_rejects_anything_but_the_moved_file`, but this executor mapping has no test entry point; per #21's "no fault framework" requirement, no injection point was added.

### O18-R Recovery logic (same-process service rebuild)
- After mkdir, before its receipt: `S/recover_tests.rs::stopped_after_mkdir_before_its_receipt_keeps_the_folder_and_the_file`
- After the move intent, before the move: `S/recover_tests.rs::stopped_after_the_move_intent_before_the_move_is_failed_no_move`
- After the move, before the final record: `S/recover_tests.rs::stopped_after_the_move_before_the_final_record_is_committed_once`
- After the Undo move, before the final record: `S/undo_tests.rs::an_undo_stopped_after_its_move_is_committed_once_at_the_next_start`, *(new)* added sha256/inode unchanged and exactly one undo history entry.
- These are recovery-logic tests: the same database is reopened after an unwind within the same process; not a process crash.

### O18-C Real abnormal termination (#21, PR #81)
- `S/crash_tests.rs` (PR #81, branch `m21-crash-checkpoints`): the test binary runs as a child process the way the app does and stops at the checkpoint, and the test kills it with SIGKILL; a new process starts and reconciles the records, and one more start proves nothing repeats. One test for each of the four checkpoints, plus two cases: "file changed after the kill" and "the first new process cannot write the journal".
- Evidence `traeva-evidence/m21/`: exit status signal 9, action state before and after the kill, journal, sha256/inode; all 9 recovery-logic mutations were caught; 20 parallel repetitions all passed.
- Not equivalent to a whole-machine power loss or a disk failure.

### O19 Mismatch after the move
- A file at the old name again: `S/execute_tests.rs::a_new_file_at_the_old_name_after_the_move_needs_a_decision`, `::a_new_file_at_the_old_name_during_the_final_read_needs_a_decision`, `S/recover_tests.rs::a_file_at_both_names_needs_a_decision`.
- At neither place: `S/recover_tests.rs::a_file_at_neither_name_needs_a_decision`.
- Same inode with different bytes, same bytes in a different object: `S/execute_tests.rs::a_change_right_after_the_move_needs_a_decision_and_nothing_moves_back`, `::the_same_bytes_in_another_file_after_the_move_need_a_decision`, `S/recover_tests.rs::the_same_inode_with_other_bytes_needs_a_decision`.
- Chain or root changes: see O10, and `S/recover_tests.rs::a_replaced_target_folder_needs_a_decision`, `::a_moved_root_needs_a_decision_until_it_is_back`.
- Root revoked while unsettled: `S/recover_tests.rs::a_root_revoked_before_the_check_needs_a_decision` *(new)*.
- Journal does not match the plan: `S/recover_tests.rs::a_moved_file_with_a_journal_that_does_not_match_is_not_committed`; Undo side: `S/undo_tests.rs::an_uncertain_undo_waits_for_a_decision`.

### O20 One plan, one action
- Double click/repeated confirmation: `T/execute.rs::a_double_click_runs_the_plan_only_once`, `S/execute_tests.rs::a_second_click_during_a_run_returns_at_once_with_the_running_action`, `::a_click_that_waited_for_the_same_plan_gets_its_action_whatever_the_result`.
- Confirming again after success/failure/recovery: `T/execute.rs::organizing_moves_the_file_and_records_it_exactly_once`, `::a_failed_final_record_needs_a_decision_and_the_move_never_repeats`, `S/recover_tests.rs::stopped_after_the_move_before_the_final_record_is_committed_once`.
- Database constraints: `T/schema.rs::one_plan_has_at_most_one_action`, `::journal_sequence_and_path_history_reason_are_unique_per_action`; re-check inside the transaction: `S/execute_tests.rs::the_confirmation_checks_again_inside_its_transaction`.
- A failed confirmation transaction leaves no root write: see O17's two tests.

### O21 Undo after restart
- `T/undo.rs::organize_then_undo_puts_the_same_file_back`; *(new)* `T/undo.rs::after_a_restart_an_undo_works_once_and_then_only_reports` adds two levels of new folders; Undo after restart, sha256, inode and file_id are all the same as before organizing, and both folder levels are cleaned up.
- **C**: `o18c_killed_after_the_undo_move_is_committed_once_by_the_next_process` (#21).
- **W**: in Gate 2 final acceptance, after a real quit and restart, both files were undone back to their original paths, UUID/hash/inode unchanged; the same in the #18 demo.

### O22 Undo conflicts
- `S/undo_tests.rs::an_edited_moved_or_replaced_file_cannot_be_undone`, `::a_taken_old_name_or_a_changed_old_folder_blocks_the_undo`, `::a_revoked_folder_blocks_the_undo`, `::confirming_checks_everything_again`, `::an_undo_plan_whose_organize_action_changed_meanwhile_never_runs`; `T/undo.rs::an_edited_file_or_a_taken_old_name_is_an_undo_conflict`.

### O23 Folder cleanup
- Deleted only with a receipt, when empty and the same object, from the inside out: `S/undo_tests.rs::an_organized_file_goes_back_exactly_and_its_new_folders_go_away`, `S/recover_tests.rs::stopped_with_receipts_before_the_move_intent_removes_its_empty_folders`.
- No receipt or an untrustworthy receipt: `S/recover_tests.rs::stopped_after_mkdir_before_its_receipt_keeps_the_folder_and_the_file`, `::a_journal_that_does_not_match_its_plan_is_not_read_as_no_move_intent`.
- Same name but swapped for another empty folder: `S/recover_tests.rs::a_swapped_folder_with_the_same_name_is_not_removed` *(new)*; primitive `P::only_the_same_empty_folder_is_ever_removed`.
- Non-empty, pre-existing, `.DS_Store`: `S/undo_tests.rs::folders_that_are_not_empty_or_not_its_own_stay_with_a_warning`, `S/recover_tests.rs::a_ds_store_keeps_the_folder_and_every_folder_around_it`.
- Cleanup intent cannot be written: `S/recover_tests.rs::a_folder_is_not_removed_when_its_cleanup_intent_cannot_be_saved`.
- Folders another unsettled action depends on: `S/recover_tests.rs::removing_folders_never_breaks_another_unsettled_action`, `::cleanup_keeps_a_created_folder_that_became_the_root_of_another_unsettled_action`.

### O24 Repeated Undo
- `S/undo_tests.rs::undoing_twice_moves_nothing_more`, `::only_an_organized_file_can_be_undone`, `T/undo.rs::after_a_restart_an_undo_works_once_and_then_only_reports`, `T/schema.rs::an_action_has_at_most_one_committed_undo`, `S/undo_tests.rs::a_failed_final_undo_record_is_completed_at_the_next_start`.

### O25 Injection is only data
- Fixed instructions, data declaration line, strict schema, no tools, injected text inside numbered pieces, no root names or absolute paths sent: `S/classify_tests.rs::the_request_carries_only_data_and_no_tools`.
- Model answers that follow the injection (F99, /etc, a command field) are refused: `S/classify_tests.rs::an_invalid_answer_is_rejected_and_nothing_is_chosen`, `::the_materials_invalid_examples_are_refused`.
- A valid result is only a suggestion: see O02.
- Markdown scripts, links and image links stay plain characters: `S/observe_tests.rs::markdown_scripts_and_links_stay_plain_text` *(new)*.
- Page code never treats strings as HTML or code: FE `src/features/organize/text-only.test.ts` *(new)*.
- **BLOCKED**: how the real model behaves when facing injection belongs to the #22 independent evaluation (7 cases, including 1 injection case); not run.

### O26 Truncation, unsupported, failure, 20 MiB
- Truncation and omission: `S/observe_tests.rs::longer_texts_use_head_and_tail_with_the_middle_marked`, `S/classify_tests.rs::a_long_text_is_sent_as_head_and_tail_with_the_gap_marked` (*new*: analyses metadata records total_chars 10,003, truncated, head_6000_tail_2000).
- Unsupported/failure stated truthfully: `S/observe_tests.rs::only_txt_and_markdown_files_are_read`, `::utf16_nul_and_invalid_utf8_are_never_guessed` (*new*: an explanatory note on failure), `T/observe.rs::a_file_without_readable_text_can_still_be_registered`, `S/classify_tests.rs::files_without_usable_text_are_never_sent`.
- Over 20 MiB: registration `S/observe_tests.rs::files_over_20_mib_are_refused_without_reading`, `T/observe.rs::files_that_cannot_be_organized_are_refused_and_not_recorded`; window operation layer Blocked and cannot go to manual pick `S/operation_tests.rs::a_file_over_20_mib_is_blocked_and_cannot_be_chosen_by_hand` *(new)*; grows only after the preview or after confirmation `S/execute_tests.rs::a_file_that_grew_past_20_mib_after_the_preview_is_not_moved` *(new)*.
- **W**: the #18 PDF and Gate 2's manual-only.bin both show that the model was not asked and the file went straight to manual pick.

### O29 Encoding boundaries
- `S/observe_tests.rs::characters_are_counted_as_unicode_scalar_values`, `::a_utf8_bom_is_allowed_and_line_endings_are_normalized`, `::empty_and_whitespace_only_texts_are_blank`, `::a_stable_read_returns_the_exact_bytes_and_their_hash`, `::utf16_nul_and_invalid_utf8_are_never_guessed`.
- A file with a Chinese name, BOM, CRLF and emoji is registered; the database sha256 equals the raw-byte hash, and characters are counted after removing the BOM and CRLF→LF: `T/observe.rs::a_bom_crlf_chinese_text_is_hashed_as_its_raw_bytes` *(new)*.
- Chinese, emoji and é beyond 8,000 are cut into head and tail by characters: `S/observe_tests.rs::long_multibyte_texts_are_cut_by_characters` *(new)*.

### O30 8,000-character and 20 MiB boundaries
- `S/observe_tests.rs::exactly_8000_characters_are_used_whole`, `::longer_texts_use_head_and_tail_with_the_middle_marked`, `::a_long_text_keeps_true_line_numbers_in_the_tail`, `::evidence_pieces_have_a_size_limit`, `::usable_text_means_at_least_one_text_piece`, `::files_over_20_mib_are_refused_without_reading`; constants pinned by `::the_read_budget_is_the_specs` *(new)*; for multibyte see O29.

### O31 Changes during reading
- `S/observe_tests.rs::a_stable_read_returns_the_exact_bytes_and_their_hash`, `::a_file_that_keeps_changing_is_not_used`, `::the_time_budget_stops_further_attempts`, `::the_name_must_still_point_at_the_file_that_was_read`, `::a_file_replaced_by_an_atomic_save_is_read_again`, `::moving_a_parent_folder_while_reading_is_caught`, `::a_file_that_keeps_changing_is_not_registered`, `S/operation_tests.rs::a_changing_file_waits_and_an_unsupported_one_is_blocked`, `S/plan_tests.rs::a_file_that_keeps_changing_does_not_end_the_plan`.
- Still changing when read again at analysis: not sent, no request recorded `S/classify_tests.rs::a_file_still_changing_at_the_request_is_not_sent` *(new)*; different from the registered hash after reading: `::a_file_that_changed_since_it_was_read_is_not_sent`.
- 350 ms, 3 times, 5 seconds: `S/observe_tests.rs::the_read_budget_is_the_specs` *(new)*. "No claim that the writer finished" is reflected in the notice text and in the description of `StableRead`; there is no separate test.

### O32 IPC and privacy
- Forged IDs/paths/unknown fields/other windows: IPC `the_main_window_gets_the_cores_answers_and_errors`, `input_that_does_not_fit_the_contract_is_refused`, `nothing_outside_the_capability_can_be_called` (*new*: when the main window shows a remote page, every command is refused, because the capability allows only local URLs), `analysis_and_model_settings_commands_reach_the_core`; two items in `IPC permissions.rs`; `T/plans.rs::plan_ids_and_requests_are_checked`, `T/roots.rs::root_ids_must_be_exact_known_ids`, `T/execute.rs::ids_are_checked` (*new*: a real ID of the wrong kind is always Unknown*).
- No content/raw prompt/secret by default: `S/model_tests.rs::no_file_text_or_key_reaches_the_database_or_app_data`, `::redaction_removes_the_key_and_key_like_text`, `S/settings_tests.rs::the_key_is_kept_out_of_the_settings_file_and_debug_output`, `S/analysis_tests.rs::the_page_flow_runs_through_the_same_plan_preview_and_executor` (the scan can see the checked summary, which shows the scan works).
- The agreed metadata is really saved: `S/model_tests.rs::an_answer_comes_back_and_only_metadata_is_saved`, `S/classify_tests.rs::a_valid_answer_is_checked_kept_and_still_only_a_suggestion` (*new*: prompt/schema/extractor versions equal the constants).
- Diagnostics off by default, can be cleared: `S/settings_tests.rs::diagnostics_are_off_until_turned_on_and_can_be_cleared`.
- **W + M**: Gate 2 final acceptance scanned the two test app-data folders and all evidence: 0 hits for content, raw prompt and keys; #18 also 0 hits.
- **Awaiting the student's decision**: extra top-level command arguments (for example an extra `path` passed to `get_plan`) are ignored by Tauri rather than refused. The core never reads them, so there is no safety impact; unknown fields in structured input (PlanRequest, settings, answers) are already refused. Whether top-level ones must be refused too is for the student to decide; changing it would touch the IPC signatures.

### O33 Cancel, late, restart
- `S/operation_tests.rs::a_cancel_while_reading_wins_over_the_reading`, `::a_cancelled_operation_drops_its_late_result`, `S/analysis_tests.rs::a_request_not_sent_yet_is_never_sent_once_the_user_moves_on`, `::a_cancel_or_a_manual_choice_drops_the_late_result`, `::a_late_result_of_one_operation_never_touches_the_next_one`, `::two_runs_of_one_request_send_it_once`, `::a_confirmed_plan_is_never_cancelled_as_if_nothing_happened`, `::a_restart_ends_the_operation_and_sends_nothing_again`, `S/model_tests.rs::a_cancelled_request_drops_its_late_answer`, `::a_setup_change_while_the_request_waits_means_nothing_is_sent` (Gate 2 #70), `::a_request_still_waiting_when_traeva_quits_is_interrupted_at_the_next_start`; FE “an older or foreign view never replaces the shown one”.
- A restart invalidates unconsumed plans: `T/plans.rs::a_restart_invalidates_open_plans_and_old_folder_lists` (*new*: confirming the old plan after a restart gives StalePlan, 0 actions, tree unchanged), `S/plan_tests.rs::a_restart_invalidates_open_plans_but_not_used_ones`.
- Window refresh: core reads have no side effects, the page calls only `getAppState` on load, and `applyPlan` is called only in the confirm button (code review). There is no frontend test and no dedicated record of a real window refresh.

### O34 Storage unavailable
- With a read-only, broken or higher-schema database, every operation returns a storage error, the test root is unchanged, and the read-only database's bytes are unchanged: `T/startup.rs::blocked_storage_refuses_every_operation_and_keeps_the_file` *(new)*.
- Page-level damage is found by the integrity check at startup, and the file is neither repaired nor replaced: `T/startup.rs::a_damaged_page_is_found_at_the_start_and_the_file_is_kept` *(new)*.
- `T/startup.rs::newer_schema_is_refused_and_the_file_is_kept`, `::corrupt_database_is_refused_and_the_file_is_kept`, `::failed_migration_rolls_back_and_keeps_the_database`, `::second_instance_is_blocked_and_does_not_touch_the_database`, `::read_only_database_is_reported`, `::unwritable_app_data_location_is_reported`; write failure while running: `S/recover_tests.rs::a_check_that_cannot_be_recorded_blocks_storage_until_the_next_start`.
- The second instance is tested with a same-process flock; across processes: #21's tests show that the killed process's lock is released by the kernel and a new process can start; mutual exclusion of two live processes is in #65's multi-process evidence (Claude-A).

### O35
- Same-location no-op: `T/plans.rs::a_file_already_in_the_chosen_folder_needs_no_plan`, `S/plan_tests.rs::choosing_the_place_the_file_is_in_ends_the_earlier_plan`.
- A cleanup failure does not erase the restore (cleanup intent cannot be written, macOS refuses to delete the folder), still committed/undone after restart: `S/undo_tests.rs::a_cleanup_that_fails_never_undoes_the_undo` *(new)*.
- Recovery does not roll back: `S/recover_tests.rs::a_file_at_both_names_needs_a_decision`, `::a_file_at_neither_name_needs_a_decision`, `::the_same_inode_with_other_bytes_needs_a_decision`, `::stopped_after_the_move_intent_before_the_move_is_failed_no_move`, `S/execute_tests.rs::a_change_right_after_the_move_needs_a_decision_and_nothing_moves_back`.

### O36 Independent core, thin commands, real window
- The core does not depend on Tauri (walking the full dependency graph along Cargo.lock): `T/independence.rs::the_core_does_not_depend_on_tauri` *(new)*.
- Thin commands and permissions: four IPC items, two permissions items; `src-tauri/src/commands.rs` only calls the core, and src-tauri does not depend on rusqlite (code review).
- **W**: Gate 2 final acceptance (R) rebuilt the packaged `.app` at `main@ddfb16d`, isolated with `TRAEVA_APP_DATA_DIR`; native picker, separate Keychain account, send permission, recommendation/one clarification question, zero writes before confirmation, Undo after a real quit and restart, and privacy all PASS ([public Gate 2 review](https://github.com/P30g53/Traeva/issues/19#issuecomment-6074407516); the full local `REPORT.txt` is not included in this source package). The #18 demo was run by the implementer on the packaged `.app` at `85ed78d`.

## Open items and observations (none is a blocker)

1. **Not tested in a real environment**: O05 two-level new folder preview card in a real window; O11 real iCloud/File Provider placeholder files; O14 real OpenAI service failures; O33 real window refresh.
2. **BLOCKED**: #22 independent 7-case semantic evaluation (including O25's real-model injection case).
3. **Awaiting the student's decision**: O32 extra top-level IPC arguments are ignored rather than refused.
4. **Inaccurate wording**: a folder macOS refused to create is also listed under "no receipt", and the UI says "Traeva has no receipt for these folders", but this folder does not actually exist (`a_new_folder_the_system_refuses_ends_the_run_without_a_move`); when the destination becomes a project folder after the preview, the message says "moved or replaced" and does not say it is a project folder; when the file exceeds 20 MiB after the preview, FileChanged is reported, and only re-registration says it is too large. All are conservative refusals and do not affect safety.
5. **UI observations from the Gate 2 handover are still not handled**: old actions of a removed authorized folder still show a clickable "Undo…" (the core would refuse it); Recent actions does not show the authorized folder; the preview after an answer does not say "based on your answer"; the manual-pick dropdown defaults to the root folder.
6. **Existing minor issue**: turning diagnostics off while waiting still writes one diagnostic record (already explained in PR #74).
7. **Covered only by code review**: O17's executor mapping of `ChangedButUnconfirmed`.

## How to re-verify

```bash
cargo test --workspace
```

```bash
cargo test -p traeva-core -- --ignored
```

```bash
npm test
```

The second one mounts a temporary disk image (unmounted at the end). O18-C runs separately with `cargo test -p traeva-core --lib o18c -- --nocapture --test-threads=1` (after #21 merges).
