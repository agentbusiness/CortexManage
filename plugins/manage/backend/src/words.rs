//! The one word a person sees for a request or a package. Never stored: computed from the facts
//! each time, by the contract's precedence (the first that holds):
//! on hold (yours) › needs you › stuck › rework › working › watching › held › queued › done › idle.

use crate::store::{Data, ReqRec};
use cc_protocol::contracts::manage::{Package, Word, stage};
use cc_protocol::contracts::now_ms;

fn running(stage_now: &str) -> bool {
    stage_now == stage::BUILDING || stage_now == stage::PROVING
}

pub fn package(data: &Data, package: &Package) -> Word {
    let open = package.stage == stage::HELD || running(&package.stage);
    let asks = data.asks_of(package);
    // A package a person stopped or took over is theirs; so is everything unfinished while the
    // Hold switch is on.
    if open && (package.stopped || data.hold) {
        return Word::OnHold;
    }
    if asks.iter().any(|rec| rec.ask.kind != "stuck") {
        return Word::NeedsYou;
    }
    if !asks.is_empty() {
        return Word::Stuck;
    }
    if running(&package.stage) {
        if package.reworks > 0 {
            return Word::Rework;
        }
        // Watching is working with at least one intervention in this attempt.
        let stepped_in = data.interventions.get(&package.id).is_some_and(|list| list.iter().any(|rec| rec.attempt == package.attempt && rec.i.action != "note"));
        return if stepped_in { Word::Watching } else { Word::Working };
    }
    match package.stage.as_str() {
        // Its hold is over and it waits for its turn on the repository's runner.
        stage::HELD if package.hold_until_ms != 0 && package.hold_until_ms <= now_ms() => Word::Queued,
        stage::HELD => Word::Held,
        stage::READY | stage::LANDED | stage::SOAKING | stage::RELEASED => Word::Done,
        _ => Word::Idle,
    }
}

pub fn request(data: &Data, rec: &ReqRec) -> Word {
    let r = &rec.r;
    if !r.ended_as.is_empty() {
        return if r.stage == stage::RELEASED { Word::Done } else { Word::Idle };
    }
    if let Some(package) = data.packages.get(&r.package) {
        return self::package(data, package);
    }
    if data.open_asks().any(|ask| ask.ask.request == r.id || ask.other_request == r.id) {
        return Word::NeedsYou;
    }
    match r.stage.as_str() {
        // The clarify form waits for the person.
        stage::CLARIFYING => Word::NeedsYou,
        // Assessing, planning and validating are the manager's own work.
        stage::ARRIVED | stage::ASSESSED | stage::PLANNED | stage::VALIDATED => Word::Working,
        stage::HELD => Word::Held,
        stage::READY | stage::LANDED | stage::SOAKING | stage::RELEASED => Word::Done,
        _ => Word::Idle,
    }
}
