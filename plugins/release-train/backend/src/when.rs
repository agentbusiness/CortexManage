//! The wall clock and the train's time of day. The train leaves at `HH:MM` on this computer's
//! local clock, so days and times here are local.

use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, NaiveTime, TimeZone};

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

/// How often a repository's train leaves: every few hours, every day, or one day a week, at its
/// time of day (`policy.train_every`, `train_day`, `train_at`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Every {
    Hours(u32),
    Day,
    /// 0 is Monday.
    Week(u32),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Schedule {
    pub every: Every,
    pub at: (u32, u32),
}

impl Schedule {
    /// `every`: `3h` | `week` | anything else (daily). `day`: 0 Monday … 6 Sunday.
    pub fn parse(every: &str, day: u32, at: &str) -> Self {
        let every = match every.trim() {
            "3h" => Every::Hours(3),
            "week" => Every::Week(day.min(6)),
            _ => Every::Day,
        };
        Self { every, at: parse_at(at) }
    }
}

/// The train's moments on one day, each with its name (the day, or the day and the hour).
fn slots_of(day: NaiveDate, schedule: &Schedule) -> Vec<(String, u64)> {
    match schedule.every {
        Every::Day => vec![(name(day), moment(day, schedule.at))],
        Every::Week(weekday) if day.weekday().num_days_from_monday() == weekday => vec![(name(day), moment(day, schedule.at))],
        Every::Week(_) => Vec::new(),
        Every::Hours(hours) => {
            let hours = hours.max(1);
            let mut hour = schedule.at.0 % hours;
            let mut out = Vec::new();
            while hour < 24 {
                out.push((format!("{} {hour:02}:{:02}", name(day), schedule.at.1), moment(day, (hour, schedule.at.1))));
                hour += hours;
            }
            out
        }
    }
}

/// The latest train moment at or before `now` (this week at most), with its name.
pub fn current(now: u64, schedule: &Schedule) -> Option<(String, u64)> {
    let mut day = local(now).date_naive();
    for _ in 0..8 {
        if let Some(found) = slots_of(day, schedule).into_iter().filter(|(_, ms)| *ms <= now).max_by_key(|(_, ms)| *ms) {
            return Some(found);
        }
        day = day.pred_opt()?;
    }
    None
}

/// The first train moment after `after` whose name is not `skip`, with its name.
pub fn next_of(after: u64, schedule: &Schedule, skip: &str) -> (String, u64) {
    let mut day = local(after).date_naive();
    for _ in 0..16 {
        if let Some(found) = slots_of(day, schedule).into_iter().find(|(slot, ms)| *ms > after && slot != skip) {
            return found;
        }
        day = day.succ_opt().unwrap_or(day);
    }
    (name(day), moment(day, schedule.at))
}
