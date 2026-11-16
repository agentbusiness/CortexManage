//! Packages and the hold (docs/specs/02-HOLD-PACKAGES-RELEASE.md §1-3). Every validated request
//! goes into a package: a new one, or a held one of the same repository it belongs with. A package
//! waits out its hold (a clock for safe and minor, a yes for major, nothing for urgent), then
//! builds, one package per repository at a time.

use crate::inbox::{self, NewAsk};
use crate::service::Manager;
use crate::store::{Data, ReqRec};
use crate::{job, pipeline, policy};
use cc_protocol::contracts::manage::{MatchAnswer, MatchParams, Package, PackageEditParams, Point, Tier, flow, kinds, stage};
use cc_protocol::contracts::now_ms;
use cc_protocol::{RpcError, codes};
use serde_json::json;
use std::sync::Arc;

/// A package holds at most this many requests.
const MAX_REQUESTS: usize = 5;

/// A kept point as the package's joined plan has it.
#[derive(Debug, Clone)]
pub struct Joined {
    /// Numbered within the package, its `depends_on` in the same numbers, its `request` set.
    pub point: Point,
    /// Its number in its own request's plan.
    pub origin: u32,
}

/// The joined plan of a package: its requests' kept points, each after what it depends on,
/// numbered 1..n. (Points of one request keep their order; requests keep theirs.)
pub fn joined(data: &Data, package: &Package) -> Vec<Joined> {
    let mut items: Vec<(String, Point)> = Vec::new();
    for request in &package.requests {
        for point in data.kept(request) {
            items.push((request.clone(), point));
        }
    }
    let position = |request: &str, n: u32| items.iter().position(|(owner, point)| owner == request && point.n == n);
    let needs: Vec<Vec<usize>> = items.iter().map(|(request, point)| point.depends_on.iter().filter_map(|n| position(request, *n)).collect()).collect();
    let mut order: Vec<usize> = Vec::with_capacity(items.len());
    let mut placed = vec![false; items.len()];
    while order.len() < items.len() {
        // The first point whose needs are all placed; in a circle of needs, simply the first left.
        let next = (0..items.len())
            .find(|index| !placed[*index] && needs[*index].iter().all(|need| placed[*need]))
            .or_else(|| (0..items.len()).find(|index| !placed[*index]));
        let Some(next) = next else { break };
        placed[next] = true;
        order.push(next);
    }
    let number = |index: usize| order.iter().position(|placed| *placed == index).map(|at| at as u32 + 1);
    order
        .iter()
        .enumerate()
        .map(|(at, index)| {
            let (request, point) = &items[*index];
            let mut joined = point.clone();
            joined.n = at as u32 + 1;
            joined.depends_on = needs[*index].iter().filter_map(|need| number(*need)).collect();
            joined.request = request.clone();
            Joined { point: joined, origin: point.n }
        })
        .collect()
}

fn touches(points: &[Point]) -> Vec<String> {
    points.iter().flat_map(|point| point.touches.iter().cloned()).collect()
}

fn has_standard(data: &Data, package: &Package) -> bool {
    package.requests.iter().any(|id| data.requests.get(id).is_some_and(|rec| rec.r.kind == "standard"))
}

async fn package_entry(manager: &Manager, package: &Package, text: &str) {
    let sessions = manager.store.read(|data| data.sessions_of(package));
    let data = json!({"package": package.id, "text": text, "hold_until_ms": package.hold_until_ms, "joined": package.requests, "urgent": package.urgent});
    manager.entry_all(&sessions, kinds::PACKAGE, data).await;
}

/// What the hold of a package is, in a sentence.
fn hold_line(manager: &Manager, package: &Package) -> String {
    if package.urgent {
        manager.t("pkg_urgent", &[("package", &package.id)])
    } else if package.hold_until_ms == 0 {
        manager.t("pkg_held_major", &[("package", &package.id)])
    } else {
        let minutes = package.hold_until_ms.saturating_sub(now_ms()) / 60_000;
        let time = if minutes >= 120 { manager.t("time_hours", &[("count", &(minutes / 60).to_string())]) } else { manager.t("time_minutes", &[("count", &minutes.to_string())]) };
        manager.t("pkg_held", &[("package", &package.id), ("tier", package.tier.as_str()), ("time", &time)])
    }
}

/// A major package's hold ends by a yes: ask for it (and withdraw an older ask about the package
/// as it was).
async fn ask_plan(manager: &Arc<Manager>, package: &Package) {
    inbox::close_where(manager, "policy", "superseded", |ask| ask.ask.kind == "approve_plan" && ask.ask.package == package.id);
    let (title, points) = manager.store.read(|data| {
        let title = package.requests.first().and_then(|id| data.requests.get(id)).map(|rec| rec.r.title.clone()).unwrap_or_default();
        (title, joined(data, package).len())
    });
    let detail = manager.t("detail_approve_plan", &[("requests", &package.requests.join(", ")), ("points", &points.to_string())]);
    inbox::raise(
        manager,
        NewAsk {
            kind: "approve_plan".into(),
            repo: package.repo.clone(),
            package: package.id.clone(),
            request: package.requests.first().cloned().unwrap_or_default(),
            subject: manager.t("subject_approve_plan", &[("title", &title)]),
            title,
            detail,
            actions: inbox::actions(&["approve", "send_back", "decline"]),
            attempt: package.attempt,
            ..NewAsk::default()
        },
    )
    .await;
}

/// Put a validated request into a package.
pub async fn place(manager: &Arc<Manager>, id: &str) {
    let Ok(rec) = manager.request(id) else { return };
    if rec.r.stage != stage::VALIDATED || !rec.r.ended_as.is_empty() {
        return;
    }
    let Ok(repo) = manager.repo(&rec.r.repo) else { return };
    let kept = manager.store.read(|data| data.kept(id));
    // An urgent request never joins and nothing joins it; a standard change is always alone.
    let alone = rec.r.urgent || rec.r.kind == "standard";
    if !alone {
        let now = now_ms();
        let hold_max = u64::from(repo.repo.policy.hold_max_min) * 60_000;
        let candidates: Vec<(Package, Vec<Point>, Vec<ReqRec>)> = manager.store.read(|data| {
            let mut found: Vec<&Package> = data
                .packages
                .values()
                .filter(|package| package.repo == rec.r.repo && package.stage == stage::HELD && !package.urgent && !has_standard(data, package))
                .filter(|package| package.requests.len() < MAX_REQUESTS && now.saturating_sub(package.held_since_ms) < hold_max)
                .collect();
            found.sort_by_key(|package| package.held_since_ms);
            found
                .into_iter()
                .map(|package| {
                    let points: Vec<Point> = joined(data, package).into_iter().map(|joined| joined.point).collect();
                    let members = package.requests.iter().filter_map(|id| data.requests.get(id).cloned()).collect();
                    (package.clone(), points, members)
                })
                .filter(|(_, points, _)| points.len() + kept.len() <= policy::MAX_POINTS)
                .collect()
        });
        // Code first: plans that touch the same files would collide on landing.
        let mine = touches(&kept);
        for (package, points, _) in &candidates {
            let theirs = touches(points);
            if mine.iter().any(|a| theirs.iter().any(|b| policy::overlap(a, b))) {
                join(manager, &package.id, id, &manager.t("joined_files", &[])).await;
                return;
            }
        }
        // Then the model, one question for each pair.
        for (package, _, members) in &candidates {
            for member in members {
                let params = MatchParams { a_title: member.r.title.clone(), a_text: member.r.text.clone(), b_title: rec.r.title.clone(), b_text: rec.r.text.clone() };
                let answer: MatchAnswer = match manager.peer(flow::MATCH, json!(params)).await {
                    Ok(answer) => answer,
                    Err(error) => {
                        manager.unavailable(&format!("request:{id}"), flow::MATCH, std::slice::from_ref(&rec.r.session_id), &error).await;
                        return;
                    }
                };
                // The request may have moved on while the model thought.
                if manager.request(id).map(|now| now.r.stage != stage::VALIDATED).unwrap_or(true) {
                    return;
                }
                match answer.relation.trim() {
                    "same" => {
                        merge(manager, &member.r.id, id, answer.because.trim()).await;
                        return;
                    }
                    "together" => {
                        join(manager, &package.id, id, &manager.t("joined_together", &[("because", answer.because.trim())])).await;
                        return;
                    }
                    "contradict" => {
                        // It gets its own package, and neither builds until the person chose.
                        let package_id = create(manager, id).await;
                        let detail = manager.t(
                            "detail_contradiction",
                            &[("first", &member.r.id), ("first_title", &member.r.title), ("second", id), ("second_title", &rec.r.title), ("because", answer.because.trim())],
                        );
                        inbox::raise(
                            manager,
                            NewAsk {
                                kind: "contradiction".into(),
                                repo: rec.r.repo.clone(),
                                request: member.r.id.clone(),
                                other_request: id.to_owned(),
                                package: package_id.unwrap_or_default(),
                                title: rec.r.title.clone(),
                                subject: manager.t("subject_contradiction", &[("first", &member.r.title), ("second", &rec.r.title)]),
                                detail,
                                actions: inbox::actions(&["first", "second"]),
                                ..NewAsk::default()
                            },
                        )
                        .await;
                        return;
                    }
                    _ => {}
                }
            }
        }
    }
    manager.settled(&format!("request:{id}"));
    create(manager, id).await;
}

/// A new package for one request. Answers its id.
async fn create(manager: &Arc<Manager>, id: &str) -> Option<String> {
    let rec = manager.request(id).ok()?;
    let repo = manager.repo(&rec.r.repo).ok()?;
    let now = now_ms();
    let package = Package {
        id: manager.store.next_package_id(),
        repo: rec.r.repo.clone(),
        requests: vec![id.to_owned()],
        tier: rec.r.tier,
        urgent: rec.r.urgent,
        stage: stage::HELD.into(),
        hold_until_ms: policy::hold_until(&repo.repo.policy, rec.r.tier, rec.r.urgent, now, now),
        held_since_ms: now,
        parts: rec.r.parts.clone(),
        created_ms: now,
        ..Package::default()
    };
    manager.store.put_package(package.clone());
    manager.store.update_request(id, |rec| rec.r.package = package.id.clone());
    manager.store.set_stage(id, stage::HELD);
    let line = hold_line(manager, &package);
    package_entry(manager, &package, &line).await;
    manager.line(&rec.r.session_id, &line).await;
    if package.tier == Tier::Major && !package.urgent {
        ask_plan(manager, &package).await;
    }
    manager.changed("package", &package.id, &package.repo);
    manager.changed("request", id, &package.repo);
    manager.changed("inbox", "", &package.repo);
    manager.look_now();
    Some(package.id)
}

/// A request joins a held package: the strictest tier counts, and the clock starts again at that
/// tier's time, never past the longest hold since the package was first held.
async fn join(manager: &Arc<Manager>, package_id: &str, id: &str, why: &str) {
    let Ok(rec) = manager.request(id) else { return };
    let Ok(repo) = manager.repo(&rec.r.repo) else { return };
    let now = now_ms();
    let Some(package) = manager.store.update_package(package_id, |package| {
        if !package.requests.iter().any(|member| member == id) {
            package.requests.push(id.to_owned());
        }
        package.tier = package.tier.max(rec.r.tier);
        for part in &rec.r.parts {
            if !package.parts.contains(part) {
                package.parts.push(part.clone());
            }
        }
        package.hold_until_ms = policy::hold_until(&repo.repo.policy, package.tier, false, now, package.held_since_ms);
    }) else {
        // The package left meanwhile: the request gets its own.
        create(manager, id).await;
        return;
    };
    manager.store.update_request(id, |rec| rec.r.package = package.id.clone());
    manager.store.set_stage(id, stage::HELD);
    manager.settled(&format!("request:{id}"));
    let line = format!("{} {}", manager.t("pkg_joined", &[("request", id), ("package", &package.id), ("why", why)]), hold_line(manager, &package));
    package_entry(manager, &package, &line).await;
    manager.line(&rec.r.session_id, &line).await;
    if package.tier == Tier::Major {
        ask_plan(manager, &package).await;
    }
    manager.changed("package", &package.id, &package.repo);
    manager.changed("request", id, &package.repo);
    manager.changed("inbox", "", &package.repo);
}

/// The same thing was asked twice: the newer request ends as merged, its text joins the older
/// one, which is assessed again.
async fn merge(manager: &Arc<Manager>, older: &str, newer: &str, because: &str) {
    let Ok(new_rec) = manager.request(newer) else { return };
    let Ok(old_rec) = manager.request(older) else { return };
    manager.next_run(newer);
    manager.store.update_request(newer, |rec| {
        rec.r.ended_as = cc_protocol::contracts::manage::ended::MERGED.into();
        rec.r.ended_reason = manager.t("merged_reason", &[("into", older), ("because", because)]);
        rec.r.merged_into = older.to_owned();
    });
    manager.settled(&format!("request:{newer}"));
    let line = manager.t("line_merged", &[("into", older), ("title", &old_rec.r.title)]);
    manager.entry(&new_rec.r.session_id, kinds::STAGE, json!({"stage": "merged", "text": line})).await;
    manager.line(&new_rec.r.session_id, &line).await;
    manager.note(&old_rec.r.session_id, &manager.t("note_merged_in", &[("from", newer), ("title", &new_rec.r.title)])).await;
    manager.changed("request", newer, &new_rec.r.repo);
    pipeline::again(manager, older, &new_rec.r.text, true);
}

/// Take a request out of the held package it is in. The package goes on without it, or is retired
/// when it was the only one. False when the request is in no held package.
pub fn take_out(manager: &Manager, id: &str) -> bool {
    let found = manager.store.read(|data| {
        let package = data.requests.get(id).and_then(|rec| data.packages.get(&rec.r.package))?;
        (package.stage == stage::HELD).then(|| package.clone())
    });
    let Some(package) = found else { return false };
    manager.store.update_request(id, |rec| rec.r.package.clear());
    let left: Vec<String> = package.requests.iter().filter(|member| *member != id).cloned().collect();
    if left.is_empty() {
        inbox::close_where(manager, "policy", "superseded", |ask| ask.ask.package == package.id);
        manager.store.retire_package(&package.id, "emptied");
    } else {
        let (tier, parts, policy) = manager.store.read(|data| {
            let members: Vec<&ReqRec> = left.iter().filter_map(|member| data.requests.get(member)).collect();
            let tier = members.iter().map(|rec| rec.r.tier).max().unwrap_or_default();
            let mut parts: Vec<String> = members.iter().flat_map(|rec| rec.r.parts.iter().cloned()).collect();
            parts.sort();
            parts.dedup();
            (tier, parts, data.repos.get(&package.repo).map(|rec| rec.repo.policy.clone()).unwrap_or_default())
        });
        let now = now_ms();
        manager.store.update_package(&package.id, |package| {
            package.requests = left.clone();
            package.parts = parts.clone();
            // A package that is no longer major gets a clock; one that still is asks again below.
            if package.tier == Tier::Major && tier != Tier::Major {
                package.hold_until_ms = policy::hold_until(&policy, tier, false, now, package.held_since_ms);
            }
            package.tier = tier;
        });
        // What was asked about the package as it was is no longer the question.
        inbox::close_where(manager, "policy", "superseded", |ask| ask.ask.package == package.id && (ask.ask.kind == "approve_plan" && tier != Tier::Major));
    }
    manager.changed("package", &package.id, &package.repo);
    true
}

/// End a package that will not be built: its requests are declined.
pub async fn end(manager: &Arc<Manager>, id: &str, reason: &str) {
    let Ok(package) = manager.package(id) else { return };
    inbox::close_where(manager, "policy", "superseded", |ask| ask.ask.package == package.id);
    manager.store.update_package(id, |package| package.stage = cc_protocol::contracts::manage::ended::DECLINED.into());
    for request in &package.requests {
        pipeline::decline(manager, request, reason).await;
    }
    manager.changed("package", id, &package.repo);
    manager.look_now();
}

/// `manage.package.edit`: the hold's buttons.
pub async fn edit(manager: &Arc<Manager>, params: &PackageEditParams) -> Result<(), RpcError> {
    let package = manager.package(&params.id)?;
    let running = matches!(package.stage.as_str(), stage::BUILDING | stage::PROVING);
    let held = package.stage == stage::HELD;
    if !running && !held {
        return Err(RpcError::new(codes::CONFLICT, manager.t("edit_too_late", &[("package", &package.id)])));
    }
    if params.start_now {
        if held {
            // Starting a major package now is the yes its hold waits for.
            inbox::close_where(manager, "person", "approve", |ask| ask.ask.kind == "approve_plan" && ask.ask.package == package.id);
            let updated = manager.store.update_package(&package.id, |package| {
                package.hold_until_ms = now_ms();
                package.stopped = false;
            });
            if let Some(updated) = updated {
                package_entry(manager, &updated, &manager.t("pkg_start_now", &[("package", &package.id)])).await;
            }
        } else {
            manager.store.update_package(&package.id, |package| package.stopped = false);
            job::tell(manager, &package.id, job::Msg::Resume);
        }
        manager.look_now();
    }
    if params.stop {
        // A held package no longer goes by the clock; a running job is told to stop with the
        // answer to its next beat.
        let updated = manager.store.update_package(&package.id, |package| package.stopped = true);
        if let Some(updated) = updated {
            package_entry(manager, &updated, &manager.t("pkg_stopped", &[("package", &package.id)])).await;
        }
    }
    if let Some(request) = &params.unbundle {
        if !held {
            return Err(RpcError::new(codes::CONFLICT, manager.t("unbundle_late", &[("package", &package.id)])));
        }
        if !package.requests.contains(request) || package.requests.len() < 2 {
            return Err(RpcError::invalid_params(format!("{request} is not bundled in {}", package.id)));
        }
        take_out(manager, request);
        manager.store.set_stage(request, stage::VALIDATED);
        // Its own package, not `place`: that would put it straight back.
        create(manager, request).await;
    }
    manager.changed("package", &package.id, &package.repo);
    manager.changed("inbox", "", &package.repo);
    Ok(())
}

/// The clock's look at the holds: per repository, when the Hold switch is off and nothing of the
/// repository is building, the package whose hold ended starts (an urgent one first, then the
/// one held longest).
pub fn tick(manager: &Arc<Manager>) {
    let now = now_ms();
    let (start, stranded): (Vec<String>, Vec<String>) = manager.store.read(|data| {
        // A dispatch that a restart cut off before the runner had the plan is tried again.
        let stranded: Vec<String> = data
            .packages
            .values()
            .filter(|package| package.stage == stage::BUILDING && package.session_id.is_empty() && !job::dispatching(manager, &package.id))
            .map(|package| package.id.clone())
            .collect();
        if data.hold {
            return (Vec::new(), stranded);
        }
        let mut start = Vec::new();
        for repo in data.repos.values().filter(|rec| rec.repo.managed) {
            let of_repo: Vec<&Package> = data.packages.values().filter(|package| package.repo == repo.repo.id).collect();
            if of_repo.iter().any(|package| matches!(package.stage.as_str(), stage::BUILDING | stage::PROVING)) {
                continue;
            }
            let mut due: Vec<&&Package> = of_repo
                .iter()
                .filter(|package| package.stage == stage::HELD && !package.stopped && package.hold_until_ms != 0 && package.hold_until_ms <= now)
                .filter(|package| data.asks_of(package).is_empty())
                .collect();
            due.sort_by_key(|package| (!package.urgent, package.held_since_ms));
            if let Some(first) = due.first() {
                start.push(first.id.clone());
            }
        }
        (start, stranded)
    });
    for id in stranded {
        if let Some(package) = manager.store.update_package(&id, |package| package.stage = stage::HELD.into()) {
            for request in &package.requests {
                manager.store.set_stage(request, stage::HELD);
            }
        }
    }
    for id in start {
        // Marked as building here, in the same step that chose it, so nothing else of the
        // repository starts beside it.
        manager.store.update_package(&id, |package| package.stage = stage::BUILDING.into());
        job::dispatch(manager, &id);
    }
}
