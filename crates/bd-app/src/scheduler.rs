//! Scheduler: rules that start, pause, limit the speed, reconnect or turn
//! the computer off at given times, edited in Settings › Scheduler.

use std::collections::HashSet;

use bd_core::Settings;
use chrono::{Datelike, Local, NaiveTime, Timelike, Weekday};
use serde::{Deserialize, Serialize};

use crate::app::App;
use crate::RuleRow;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    pub enabled: bool,
    /// 0 every day, 1 weekdays, 2 weekends, 3 Monday … 9 Sunday.
    pub days: u8,
    pub from: String,
    pub to: String,
    /// 0 limit speed, 1 pause everything (windows); 2 start all, 3 pause all,
    /// 4 reconnect, 5 turn off (at `from`).
    pub action: u8,
    /// Speed in KiB/s for action 0.
    pub value: u64,
}

impl Default for Rule {
    fn default() -> Self {
        Rule { enabled: true, days: 0, from: "09:00".into(), to: "18:00".into(), action: 0, value: 1024 }
    }
}

pub fn rules(settings: &Settings) -> Vec<Rule> {
    settings.option("schedule").and_then(|json| serde_json::from_str(json).ok()).unwrap_or_default()
}

fn save(settings: &mut Settings, rules: &[Rule]) {
    settings.set_option("schedule", serde_json::to_string(rules).unwrap_or_default());
}

pub fn rule_rows(settings: &Settings) -> Vec<RuleRow> {
    rules(settings)
        .into_iter()
        .map(|r| RuleRow {
            enabled: r.enabled,
            days: r.days as i32,
            from: r.from.into(),
            to: r.to.into(),
            action: r.action as i32,
            value: r.value.min(i32::MAX as u64) as i32,
        })
        .collect()
}

pub fn add(settings: &mut Settings) {
    let mut list = rules(settings);
    list.push(Rule::default());
    save(settings, &list);
}

pub fn remove(settings: &mut Settings, index: usize) {
    let mut list = rules(settings);
    if index < list.len() {
        list.remove(index);
    }
    save(settings, &list);
}

pub fn change(settings: &mut Settings, index: usize, field: &str, value: &str) {
    let mut list = rules(settings);
    let Some(rule) = list.get_mut(index) else { return };
    match field {
        "enabled" => rule.enabled = value == "true",
        "days" => rule.days = value.parse().unwrap_or(0),
        "from" => rule.from = value.trim().to_owned(),
        "to" => rule.to = value.trim().to_owned(),
        "action" => rule.action = value.parse().unwrap_or(0),
        "value" => rule.value = value.parse().unwrap_or(1024),
        _ => {}
    }
    save(settings, &list);
}

fn day_matches(days: u8, weekday: Weekday) -> bool {
    let index = weekday.num_days_from_monday() as u8;
    match days {
        0 => true,
        1 => index < 5,
        2 => index >= 5,
        d => d.saturating_sub(3) == index,
    }
}

fn parse_time(text: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(text.trim(), "%H:%M").ok()
}

fn in_window(now: NaiveTime, from: NaiveTime, to: NaiveTime) -> bool {
    if from <= to {
        now >= from && now < to
    } else {
        now >= from || now < to
    }
}

/// What the rules ask for right now.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Wanted {
    pub limit_kib: Option<u64>,
    pub pause: bool,
    /// Indices of point rules due now.
    pub fire: Vec<usize>,
}

pub fn evaluate(rules: &[Rule], weekday: Weekday, now: NaiveTime) -> Wanted {
    let mut wanted = Wanted::default();
    let minute = now.with_second(0).unwrap_or(now);
    for (i, rule) in rules.iter().enumerate() {
        if !rule.enabled || !day_matches(rule.days, weekday) {
            continue;
        }
        let Some(from) = parse_time(&rule.from) else { continue };
        let to = parse_time(&rule.to).unwrap_or(from);
        match rule.action {
            0 if in_window(now, from, to) => {
                wanted.limit_kib = Some(wanted.limit_kib.map_or(rule.value, |l| l.min(rule.value)));
            }
            1 if in_window(now, from, to) => wanted.pause = true,
            2..=5 if minute == from => wanted.fire.push(i),
            _ => {}
        }
    }
    wanted
}

/// Remembers what the scheduler did so it can undo it.
#[derive(Default)]
pub struct State {
    pub paused: bool,
    pub limit_kib: Option<u64>,
    fired: HashSet<(usize, u32, u32)>,
}

/// Applies the rules; called a few times per minute.
pub fn tick(app: &mut App) {
    if !app.modules.is_enabled("scheduler") {
        if app.schedule.limit_kib.take().is_some() {
            app.apply_config();
        }
        return;
    }
    let now = Local::now();
    let list = rules(&app.settings);
    let wanted = evaluate(&list, now.weekday(), now.time());

    if wanted.limit_kib != app.schedule.limit_kib {
        app.schedule.limit_kib = wanted.limit_kib;
        app.apply_config();
    }
    if wanted.pause && !app.schedule.paused {
        app.schedule.paused = true;
        app.manager.pause_all();
        app.toast("Scheduler: paused");
    } else if !wanted.pause && app.schedule.paused {
        app.schedule.paused = false;
        app.manager.resume_all();
        app.toast("Scheduler: resumed");
    }
    for index in wanted.fire {
        let key = (index, now.ordinal(), now.hour() * 60 + now.minute());
        if !app.schedule.fired.insert(key) {
            continue;
        }
        match list[index].action {
            2 => app.manager.resume_all(),
            3 => app.manager.pause_all(),
            4 => crate::actions::reconnect(app),
            5 => crate::platform::shutdown_computer(),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> NaiveTime {
        parse_time(s).unwrap()
    }

    #[test]
    fn windows_and_days() {
        let rules = vec![
            Rule { action: 0, value: 500, ..Rule::default() },
            Rule { action: 0, value: 200, days: 1, from: "08:00".into(), to: "20:00".into(), ..Rule::default() },
            Rule { action: 1, from: "23:00".into(), to: "06:00".into(), ..Rule::default() },
            Rule { action: 2, from: "07:30".into(), to: String::new(), ..Rule::default() },
        ];
        let w = evaluate(&rules, Weekday::Mon, t("10:00"));
        assert_eq!(w.limit_kib, Some(200));
        assert!(!w.pause);
        let w = evaluate(&rules, Weekday::Sat, t("10:00"));
        assert_eq!(w.limit_kib, Some(500));
        assert!(evaluate(&rules, Weekday::Sat, t("02:00")).pause, "overnight window");
        assert_eq!(evaluate(&rules, Weekday::Tue, t("07:30")).fire, vec![3]);
        assert!(evaluate(&rules, Weekday::Tue, t("07:31")).fire.is_empty());
    }
}
