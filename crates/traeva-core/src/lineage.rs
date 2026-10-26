//! Select saved version pairs for comparison and retrieval.
//! Reads records without opening user files or writing data.

use std::collections::{BTreeSet, HashMap, HashSet};

use crate::OrganizeService;
use crate::contracts::{AppError, ErrorCode};
use crate::family::{CandidateRow, EdgeKind, FamilyFacts, MemberBasis, RelationStatus, RunHistory};
use crate::is_canonical_uuid;
use crate::memory_consent::memory_error;
use crate::memory_contracts::{
    GapKind, PairBasis, PairCheck, PairEvidence, PairRefusal, SnapshotRef, UsablePair,
};
use crate::snapshot::{check_file_id, unknown_file};
use crate::store::{Store, sqlite_error};

// List pairs by work, file or connected section, and recheck selected pairs.
impl OrganizeService {
    pub fn usable_pairs_of_family(&self, family_id: &str) -> Result<Vec<UsablePair>, AppError> {
        let store = self.store()?;
        if !is_canonical_uuid(family_id) {
            #[rustfmt::skip]
            return Err(memory_error(ErrorCode::InvalidInput, "The work ID is not valid.", false, None));
        }
        let live = match store.live_family(family_id)? {
            Some(live) => live,
            None => return Ok(Vec::new()),
        };
        let facts = store.family_facts(&live)?;
        let runs = self.run_history(store)?;
        let pairs = pairs_of(&facts, &runs);
        Ok(pairs)
    }

    pub fn usable_pairs_of_file(&self, file_id: &str) -> Result<Vec<UsablePair>, AppError> {
        let store = self.store()?;
        check_file_id(file_id)?;
        if store.get_file(file_id)?.is_none() {
            return Err(unknown_file());
        }
        let family_id = match store.family_of_file(file_id)? {
            Some(family_id) => family_id,
            None => return Ok(Vec::new()),
        };
        let facts = store.family_facts(&family_id)?;
        let runs = self.run_history(store)?;
        let pairs = pairs_of(&facts, &runs);
        let mut found = Vec::new();
        for pair in pairs {
            if pair.before.file_id == file_id || pair.after.file_id == file_id {
                found.push(pair);
            }
        }
        Ok(found)
    }

    #[rustfmt::skip]
    pub fn usable_pairs_of_line(&self, file_id: &str) -> Result<Vec<UsablePair>, AppError> {
        let store = self.store()?;
        check_file_id(file_id)?;
        if store.get_file(file_id)?.is_none() {
            return Err(unknown_file());
        }
        let family_id = match store.family_of_file(file_id)? {
            Some(family_id) => family_id,
            None => return Ok(Vec::new()),
        };
        let facts = store.family_facts(&family_id)?;
        let runs = self.run_history(store)?;
        let pairs = pairs_of(&facts, &runs);
        let parts = facts_parts(&facts, &HashSet::new());

        // Files outside a record line keep their own pairs and incoming relations.
        let part = match part_of(&parts, file_id) {
            Some(part) => part,
            None => {
                let mut found = Vec::new();
                for pair in pairs {
                    if pair.before.file_id == file_id || pair.after.file_id == file_id {
                        found.push(pair);
                    }
                }
                return Ok(found);
            }
        };

        // Both versions must be in this section; branches leading out are excluded.
        let mut on_line: HashSet<&str> = HashSet::new();
        for file in &part.file_ids {
            on_line.insert(file.as_str());
        }
        let mut line = Vec::new();
        for pair in pairs {
            if on_line.contains(pair.before.file_id.as_str()) && on_line.contains(pair.after.file_id.as_str())
            {
                line.push(pair);
            }
        }

        // Use database write order, not capture timestamps.
        let mut ids = Vec::new();
        for pair in &line {
            ids.push(&pair.after.snapshot_id);
        }
        let written = store.written_order(ids.into_iter())?;
        line.sort_by_key(|pair| written.get(&pair.after.snapshot_id).copied());
        Ok(line)
    }

    pub fn all_usable_pairs(&self) -> Result<Vec<UsablePair>, AppError> {
        let store = self.store()?;
        let runs = self.run_history(store)?;
        let mut pairs = Vec::new();
        for family_id in store.all_family_ids()? {
            let facts = store.family_facts(&family_id)?;
            let work_pairs = pairs_of(&facts, &runs);
            for pair in work_pairs {
                pairs.push(pair);
            }
        }
        Ok(pairs)
    }

    #[rustfmt::skip]
    pub fn check_usable_pair(
        &self,
        before_snapshot_id: &str,
        after_snapshot_id: &str,
    ) -> Result<PairCheck, AppError> {
        let store = self.store()?;
        for id in [before_snapshot_id, after_snapshot_id] {
            if !is_canonical_uuid(id) {
                return Err(memory_error(
                    ErrorCode::InvalidInput,
                    "The saved version ID is not valid.",
                    false,
                    None,
                ));
            }
        }
        let before = known_snapshot(store, before_snapshot_id)?;
        let after = known_snapshot(store, after_snapshot_id)?;
        let family_id = match store.family_of_file(&after.file_id)? {
            Some(family_id) => family_id,
            None => {
                let reason = if store.is_candidate_file(&after.file_id)? {
                    PairRefusal::CandidateVersion
                } else {
                    PairRefusal::NotAMember
                };
                return Ok(PairCheck::NotUsable { reason });
            }
        };
        let facts = store.family_facts(&family_id)?;
        let runs = self.run_history(store)?;
        let pairs = pairs_of(&facts, &runs);
        for pair in pairs {
            if pair.before.snapshot_id == before.snapshot_id && pair.after.snapshot_id == after.snapshot_id {
                return Ok(PairCheck::Usable { pair });
            }
        }
        if before.file_id == after.file_id {
            return Ok(PairCheck::NotUsable { reason: PairRefusal::NotAdjacent });
        }

        // Report disputed relations before revoked or replaced ones.
        let statuses = store.edge_statuses(&before.snapshot_id, &after.snapshot_id)?;
        for status in &statuses {
            if status == "disputed" {
                return Ok(PairCheck::NotUsable { reason: PairRefusal::RelationDisputed });
            }
        }
        let mut all_superseded = true;
        for status in &statuses {
            if status != "superseded" {
                all_superseded = false;
                break;
            }
        }
        if !statuses.is_empty() && all_superseded {
            return Ok(PairCheck::NotUsable { reason: PairRefusal::RelationSuperseded });
        }
        let before_family = store.family_of_file(&before.file_id)?;
        if before_family.is_none() {
            let reason = if store.is_candidate_file(&before.file_id)? {
                PairRefusal::CandidateVersion
            } else {
                PairRefusal::NotAMember
            };
            return Ok(PairCheck::NotUsable { reason });
        }
        Ok(PairCheck::NotUsable { reason: PairRefusal::NoConfirmedRelation })
    }
}

fn confirmed(status: RelationStatus) -> bool {
    status == RelationStatus::ConfirmedSystem || status == RelationStatus::ConfirmedUser
}

// Records inherited through a user merge count as user evidence.
fn evidence_in(facts: &FamilyFacts, status: RelationStatus, row_family: &str) -> PairEvidence {
    if row_family == facts.family_id {
        evidence_of(status)
    } else {
        PairEvidence::User
    }
}

fn evidence_of(status: RelationStatus) -> PairEvidence {
    if status == RelationStatus::ConfirmedSystem {
        PairEvidence::System
    } else {
        PairEvidence::User
    }
}

// Find confirmed files with a known place in the work's history.
#[rustfmt::skip]
pub(crate) fn placed_files(facts: &FamilyFacts) -> HashSet<String> {
    let mut members: HashMap<&str, (RelationStatus, MemberBasis)> = HashMap::new();
    for member in &facts.members {
        members.insert(member.file_id.as_str(), (member.status, member.basis));
    }
    let file_of = snapshot_files(facts);
    let parts = facts_parts(facts, &HashSet::new());
    let mut placed = HashSet::new();
    for part in &parts {
        let mut started_here = false;
        for chain in &facts.chains {
            if chain.chain_id != part.chain_id || chain.origin_file_id != part.first_file_id {
                continue;
            }
            let (status, basis) = match members.get(chain.origin_file_id.as_str()) {
                Some(member) => member,
                None => continue,
            };
            if confirmed(*status) && (*basis == MemberBasis::MemoryOn || *basis == MemberBasis::FromBefore) {
                started_here = true;
                break;
            }
        }
        if !started_here {
            continue;
        }
        for file_id in &part.file_ids {
            let (status, _) = match members.get(file_id.as_str()) {
                Some(member) => member,
                None => continue,
            };
            if confirmed(*status) {
                placed.insert(file_id.clone());
            }
        }
    }

    loop {
        let before = placed.len();
        for edge in &facts.edges {
            let parent = match &edge.parent_snapshot_id {
                Some(parent) => parent,
                None => continue,
            };
            if !confirmed(edge.status) {
                continue;
            }
            let parent_file = file_of.get(parent.as_str());
            let child_file = file_of.get(edge.child_snapshot_id.as_str());
            let parent_file = match parent_file {
                Some(file) => file,
                None => continue,
            };
            let child_file = match child_file {
                Some(file) => file,
                None => continue,
            };
            if !placed.contains(*parent_file) {
                continue;
            }
            let (status, _) = match members.get(*child_file) {
                Some(member) => member,
                None => continue,
            };
            if confirmed(*status) {
                placed.insert(child_file.to_string());
            }
        }
        for part in &parts {
            if !placed.contains(&part.first_file_id) {
                continue;
            }
            for file_id in &part.file_ids {
                let (status, _) = match members.get(file_id.as_str()) {
                    Some(member) => member,
                    None => continue,
                };
                if confirmed(*status) {
                    placed.insert(file_id.clone());
                }
            }
        }
        if placed.len() == before {
            return placed;
        }
    }
}

fn snapshot_files(facts: &FamilyFacts) -> HashMap<&str, &str> {
    let mut files = HashMap::new();
    for (file_id, snapshots) in &facts.snapshots {
        for snapshot in snapshots {
            files.insert(snapshot.snapshot_id.as_str(), file_id.as_str());
        }
    }
    files
}

// Files on a record chain; joins hold edge, parent-file and child-file IDs.
pub(crate) struct ChainLine {
    pub(crate) chain_id: String,
    pub(crate) origin_file_id: String,
    pub(crate) files: Vec<String>,
    pub(crate) joins: Vec<(String, String, String)>,
}

// A section joined by confirmed continuations; revoked links can split it.
// first_file_id is the origin if present, otherwise the section's starting file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LinePart {
    pub(crate) chain_id: String,
    pub(crate) first_file_id: String,
    pub(crate) file_ids: Vec<String>,
}

// Group connected files in both directions, excluding stopped continuations.
pub(crate) fn line_parts(line: &ChainLine, stopped: &HashSet<String>) -> Vec<LinePart> {
    let mut joins = Vec::new();
    for join in &line.joins {
        let (edge, from, to) = join;
        if !stopped.contains(edge) && line.files.contains(from) && line.files.contains(to) {
            joins.push(join);
        }
    }
    let mut seen: HashSet<&str> = HashSet::new();
    let mut parts: Vec<Vec<&str>> = Vec::new();
    for file in &line.files {
        if seen.contains(file.as_str()) {
            continue;
        }
        let mut part = vec![file.as_str()];
        seen.insert(file.as_str());
        let mut at = 0;
        while at < part.len() {
            let current = part[at];
            for (_, from, to) in &joins {
                let next = if from == current {
                    to.as_str()
                } else if to == current {
                    from.as_str()
                } else {
                    continue;
                };
                if !seen.contains(next) {
                    seen.insert(next);
                    part.push(next);
                }
            }
            at += 1;
        }
        parts.push(part);
    }

    let mut result = Vec::new();
    for part in parts {
        let first = if part.contains(&line.origin_file_id.as_str()) {
            line.origin_file_id.clone()
        } else {
            let mut first = part[0];
            for file in &part {
                let mut has_parent = false;
                for (_, from, to) in &joins {
                    if to == *file && part.contains(&from.as_str()) {
                        has_parent = true;
                        break;
                    }
                }
                if !has_parent {
                    first = *file;
                    break;
                }
            }
            first.to_string()
        };
        let mut file_ids = Vec::new();
        for file in part {
            file_ids.push(file.to_string());
        }
        file_ids.sort_by_key(|f| line.files.iter().position(|l| l == f));
        #[rustfmt::skip]
        result.push(LinePart { chain_id: line.chain_id.clone(), first_file_id: first, file_ids });
    }
    result
}

// Build connected sections for every record line in the work.
pub(crate) fn facts_parts(facts: &FamilyFacts, stopped: &HashSet<String>) -> Vec<LinePart> {
    let file_of = snapshot_files(facts);
    let mut joins = Vec::new();
    for edge in &facts.edges {
        if edge.kind != EdgeKind::Continuation || !confirmed(edge.status) {
            continue;
        }
        let parent_id = match &edge.parent_snapshot_id {
            Some(id) => id,
            None => continue,
        };
        let parent = match file_of.get(parent_id.as_str()) {
            Some(file) => file,
            None => continue,
        };
        let child = match file_of.get(edge.child_snapshot_id.as_str()) {
            Some(file) => file,
            None => continue,
        };
        if parent != child {
            joins.push((edge.edge_id.clone(), parent.to_string(), child.to_string()));
        }
    }

    let mut parts = Vec::new();
    for chain in &facts.chains {
        let mut files = Vec::new();
        for (chain_id, file) in &facts.lines {
            if chain_id == &chain.chain_id {
                files.push(file.clone());
            }
        }
        let line = ChainLine {
            chain_id: chain.chain_id.clone(),
            origin_file_id: chain.origin_file_id.clone(),
            files,
            joins: joins.clone(),
        };
        let sections = line_parts(&line, stopped);
        for section in sections {
            parts.push(section);
        }
    }
    parts
}

pub(crate) fn part_of<'a>(parts: &'a [LinePart], file_id: &str) -> Option<&'a LinePart> {
    for part in parts {
        for file in &part.file_ids {
            if file == file_id {
                return Some(part);
            }
        }
    }
    None
}

// Read saved chain membership, confirmed continuations and database write order.
#[rustfmt::skip]
pub(crate) fn chain_line_in(conn: &rusqlite::Connection, chain_id: &str) -> Result<ChainLine, AppError> {
    let fail = |err: rusqlite::Error| sqlite_error("Cannot read the Memory records", &err);
    let origin_file_id: String = conn
        .query_row("SELECT origin_file_id FROM memory_chains WHERE chain_id = ?1", [chain_id], |r| r.get(0))
        .map_err(fail)?;
    let files: Vec<String> = {
        let mut stmt = conn
            .prepare(
                "SELECT file_id FROM (
                     SELECT w.file_id, min(w.rowid) AS at FROM memory_chain_watches AS link
                     JOIN memory_watches AS w ON w.watch_id = link.watch_id
                     WHERE link.chain_id = ?1 GROUP BY w.file_id
                     UNION ALL
                     SELECT c.file_id, NULL FROM memory_candidates AS c
                     WHERE c.chain_id = ?1 AND c.state = 'continued')
                 GROUP BY file_id ORDER BY min(at) IS NULL, min(at), file_id",
            )
            .map_err(fail)?;
        stmt.query_map([chain_id], |r| r.get(0))
            .map_err(fail)?
            .collect::<Result<_, _>>()
            .map_err(fail)?
    };
    let joins: Vec<(String, String, String)> = {
        let mut stmt = conn
            .prepare(
                "SELECT e.edge_id, p.file_id, c.file_id FROM memory_edges AS e
                 JOIN snapshots AS p ON p.snapshot_id = e.parent_snapshot_id
                 JOIN snapshots AS c ON c.snapshot_id = e.child_snapshot_id
                 WHERE e.kind = 'continuation'
                   AND e.status IN ('confirmed_system', 'confirmed_user')
                   AND p.file_id <> c.file_id
                   AND EXISTS (SELECT 1 FROM memory_watches AS w
                               JOIN memory_chain_watches AS link ON link.watch_id = w.watch_id
                               WHERE link.chain_id = ?1 AND w.file_id = c.file_id
                               UNION ALL
                               SELECT 1 FROM memory_candidates AS k
                               WHERE k.chain_id = ?1 AND k.state = 'continued'
                                 AND k.file_id = c.file_id)
                 ORDER BY e.rowid",
            )
            .map_err(fail)?;
        stmt.query_map([chain_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map_err(fail)?
            .collect::<Result<_, _>>()
            .map_err(fail)?
    };
    Ok(ChainLine { chain_id: chain_id.to_string(), origin_file_id, files, joins })
}

impl Store {
    fn edge_statuses(&self, parent: &str, child: &str) -> Result<Vec<String>, AppError> {
        let conn = self.connection()?;
        let fail = |err: rusqlite::Error| sqlite_error("Cannot read the Memory records", &err);
        let mut stmt = conn
            .prepare(
                "SELECT status FROM memory_edges
                 WHERE parent_snapshot_id = ?1 AND child_snapshot_id = ?2",
            )
            .map_err(fail)?;
        stmt.query_map([parent, child], |r| r.get(0))
            .map_err(fail)?
            .collect::<Result<_, _>>()
            .map_err(fail)
    }

    fn written_order<'a>(
        &self,
        snapshot_ids: impl Iterator<Item = &'a String>,
    ) -> Result<HashMap<String, i64>, AppError> {
        let conn = self.connection()?;
        let fail = |err: rusqlite::Error| sqlite_error("Cannot read the versions", &err);
        let mut stmt = conn
            .prepare("SELECT rowid FROM snapshots WHERE snapshot_id = ?1")
            .map_err(fail)?;
        let mut order = HashMap::new();
        for id in snapshot_ids {
            let rowid: i64 = stmt.query_row([id], |r| r.get(0)).map_err(fail)?;
            order.insert(id.clone(), rowid);
        }
        Ok(order)
    }
}

fn known_snapshot(store: &Store, id: &str) -> Result<SnapshotRef, AppError> {
    match store.get_snapshot(id)? {
        Some(snapshot) => Ok(snapshot),
        None => Err(AppError::snapshot(
            ErrorCode::UnknownSnapshot,
            "There is no saved version with this ID.",
            None,
        )),
    }
}

// Resolve a chain through its section, then through its candidate entry.
fn chain_of(facts: &FamilyFacts, parts: &[LinePart], file_id: &str) -> Option<String> {
    if let Some(part) = part_of(parts, file_id) {
        return Some(part.chain_id.clone());
    }
    for candidate in &facts.candidates {
        if candidate.file_id == file_id {
            return Some(candidate.chain_id.clone());
        }
    }
    None
}

// Use the section's first file for send permission, or the file itself.
fn permission_of(parts: &[LinePart], file_id: &str) -> String {
    match part_of(parts, file_id) {
        Some(part) => part.first_file_id.clone(),
        None => file_id.to_string(),
    }
}

fn candidate_of<'a>(facts: &'a FamilyFacts, snapshot_id: &str) -> Option<&'a CandidateRow> {
    for candidate in &facts.candidates {
        if candidate.first_snapshot_id == snapshot_id {
            return Some(candidate);
        }
    }
    None
}

// Include before and the candidate versions skipped between the pair.
fn anchors_of(
    facts: &FamilyFacts,
    file_of: &HashMap<&str, &str>,
    before: &SnapshotRef,
    after: &SnapshotRef,
) -> Vec<String> {
    let mut anchors = vec![before.snapshot_id.clone()];
    let mut at = match candidate_of(facts, &after.snapshot_id) {
        Some(candidate) => candidate.after_snapshot_id.clone(),
        None => None,
    };
    let mut steps = 0;
    while let Some(snapshot) = at {
        if snapshot == before.snapshot_id || steps > facts.candidates.len() {
            break;
        }
        steps += 1;
        let mut owner = None;
        if let Some(file_id) = file_of.get(snapshot.as_str()) {
            for candidate in &facts.candidates {
                if &candidate.file_id == file_id {
                    owner = Some(candidate);
                    break;
                }
            }
        }
        anchors.push(snapshot);
        at = match owner {
            Some(candidate) => candidate.after_snapshot_id.clone(),
            None => None,
        };
    }
    anchors
}

// Collect gap reasons from saved gaps, watcher downtime and skipped versions.
fn gap_kinds(
    facts: &FamilyFacts,
    runs: &RunHistory,
    file_of: &HashMap<&str, &str>,
    before: &SnapshotRef,
    after: &SnapshotRef,
    skipped: bool,
) -> Vec<GapKind> {
    let anchors = anchors_of(facts, file_of, before, after);
    let mut kinds = BTreeSet::new();
    for gap in &facts.gaps {
        let anchor = match &gap.after_snapshot_id {
            Some(anchor) => anchor,
            None => continue,
        };
        if anchors.contains(anchor) {
            kinds.insert(gap.kind);
        }
    }
    for kind in runs.kinds_between(before.captured_at_ms, after.captured_at_ms) {
        kinds.insert(kind);
    }
    if skipped {
        kinds.insert(GapKind::Skipped);
    }
    let mut result = Vec::new();
    for kind in kinds {
        result.push(kind);
    }
    result
}

// Pair adjacent saves and confirmed relations between confirmed members.
// Names, hashes and timestamps do not establish a relation.
pub(crate) fn pairs_of(facts: &FamilyFacts, runs: &RunHistory) -> Vec<UsablePair> {
    let mut members: HashMap<&str, RelationStatus> = HashMap::new();
    for member in &facts.members {
        members.insert(member.file_id.as_str(), member.status);
    }
    let placed = placed_files(facts);
    let file_of = snapshot_files(facts);
    let parts = facts_parts(facts, &HashSet::new());
    let mut pairs = Vec::new();
    for member in &facts.members {
        if !confirmed(member.status) {
            continue;
        }
        let mut snapshots = Vec::new();
        for snapshot in facts.snapshots_of(&member.file_id) {
            snapshots.push(snapshot);
        }
        for i in 1..snapshots.len() {
            let before = snapshots[i - 1];
            let after = snapshots[i];
            let kinds = gap_kinds(facts, runs, &file_of, before, after, false);
            pairs.push(UsablePair {
                before: before.clone(),
                after: after.clone(),
                basis: PairBasis::SameFile,
                evidence: evidence_in(facts, member.status, &member.family_id),
                gap: !kinds.is_empty(),
                gap_kinds: kinds,
                unplaced: !placed.contains(&member.file_id),
                family_id: facts.family_id.clone(),
                chain_id: chain_of(facts, &parts, &member.file_id),
                permission_file_ids: vec![permission_of(&parts, &member.file_id)],
                edge_id: None,
            });
        }
    }
    for edge in &facts.edges {
        let parent_id = match &edge.parent_snapshot_id {
            Some(id) => id,
            None => continue,
        };
        if !confirmed(edge.status) {
            continue;
        }
        let before = facts.snapshot(parent_id);
        let after = facts.snapshot(&edge.child_snapshot_id);
        let before = match before {
            Some(snapshot) => snapshot,
            None => continue,
        };
        let after = match after {
            Some(snapshot) => snapshot,
            None => continue,
        };
        let parent_file = file_of.get(parent_id.as_str());
        let child_file = file_of.get(edge.child_snapshot_id.as_str());
        let parent_file = match parent_file {
            Some(file) => file,
            None => continue,
        };
        let child_file = match child_file {
            Some(file) => file,
            None => continue,
        };
        let parent_status = match members.get(*parent_file) {
            Some(status) => status,
            None => continue,
        };
        if !confirmed(*parent_status) {
            continue;
        }
        let child_status = match members.get(*child_file) {
            Some(status) => status,
            None => continue,
        };
        if !confirmed(*child_status) {
            continue;
        }

        // Mark a gap when the relation's starting point was not observed.
        let mut unseen_start = edge.associated;
        if !unseen_start && edge.kind == EdgeKind::Branch {
            if let Some(candidate) = candidate_of(facts, &edge.child_snapshot_id) {
                unseen_start = match &candidate.after_snapshot_id {
                    Some(previous) => previous != parent_id,
                    None => true,
                };
            }
        }
        let skipped = edge.skipped_items > 0 || unseen_start;
        let kinds = gap_kinds(facts, runs, &file_of, before, after, skipped);

        // Cross-file pairs need both permissions; remove a duplicate owner.
        let parent_permission = permission_of(&parts, parent_file);
        let child_permission = permission_of(&parts, child_file);
        let different_permissions = parent_permission != child_permission;
        let mut permission_file_ids = vec![parent_permission];
        if different_permissions {
            permission_file_ids.push(child_permission);
        }
        let basis = if edge.kind == EdgeKind::Continuation {
            PairBasis::Continuation
        } else {
            PairBasis::Branch
        };
        pairs.push(UsablePair {
            before: before.clone(),
            after: after.clone(),
            basis,
            evidence: evidence_in(facts, edge.status, &edge.family_id),
            gap: !kinds.is_empty(),
            gap_kinds: kinds,
            unplaced: !placed.contains(*child_file),
            family_id: facts.family_id.clone(),
            chain_id: chain_of(facts, &parts, child_file),
            permission_file_ids,
            edge_id: Some(edge.edge_id.clone()),
        });
    }
    #[rustfmt::skip]
    pairs.sort_by(|a, b| {
        (a.after.captured_at_ms, &a.after.file_id, a.after.sequence, a.basis).cmp(&(
            b.after.captured_at_ms,
            &b.after.file_id,
            b.after.sequence,
            b.basis,
        ))
    });
    pairs
}

#[cfg(test)]
#[path = "lineage_tests.rs"]
mod tests;
