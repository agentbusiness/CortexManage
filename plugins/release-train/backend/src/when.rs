//! The wall clock and the train's time of day. The train leaves at `HH:MM` on this computer's
//! local clock, so days and times here are local.

use chrono::{DateTime, Duration, Local, NaiveDate, NaiveTime, TimeZone};

/// The train time when a repository's policy names none that can be read (`03:00`).
const DEFAULT_AT: (u32, u32) = (3, 0);

pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// `HH:MM` as hour and minute; the default when it is not a time of day.
pub fn parse_at(text: &str) -> (u32, u32) {
    let parsed = text.trim().split_once(':').and_then(|(hour, minute)| Some((hour.parse::<u32>().ok()?, minute.parse::<u32>().ok()?)));
    parsed.filter(|(hour, minute)| *hour < 24 && *minute < 60).unwrap_or(DEFAULT_AT)
}

fn local(ms: u64) -> DateTime<Local> {
    Local.timestamp_millis_opt(ms as i64).earliest().unwrap_or_else(Local::now)
}

fn name(day: NaiveDate) -> String {
    day.format("%Y-%m-%d").to_string()
}

/// The moment `at` on `day`. A time the clock jumps over (the night summer time starts) is taken
/// an hour later, so that night's train still leaves.
fn moment(day: NaiveDate, at: (u32, u32)) -> u64 {
    let time = NaiveTime::from_hms_opt(at.0, at.1, 0).unwrap_or_default();
    let naive = day.and_time(time);
    let found = Local.from_local_datetime(&naive).earliest().or_else(|| Local.from_local_datetime(&(naive + Duration::hours(1))).earliest());
    found.map(|moment| moment.timestamp_millis().max(0) as u64).unwrap_or(0)
}

/// Today's date (`2026-11-16`) and the moment of today's train.
pub fn today(now: u64, at: (u32, u32)) -> (String, u64) {
    let day = local(now).date_naive();
    (name(day), moment(day, at))
}

/// The next train after `now`: the first train time later than `now` whose day is not `skip_day`.
/// Its date and its moment.
pub fn next(now: u64, at: (u32, u32), skip_day: &str) -> (String, u64) {
    let mut day = local(now).date_naive();
    // Today, tomorrow, and the day after when tomorrow is skipped: three days always hold one.
    for _ in 0..4 {
        let leaves = moment(day, at);
        if leaves > now && name(day) != skip_day {
            return (name(day), leaves);
        }
        day = day.succ_opt().unwrap_or(day);
    }
    (name(day), moment(day, at))
}
