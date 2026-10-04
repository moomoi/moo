//! Time zone answers for root search: "time in tokyo", "tokyo time", "3pm pst to cet",
//! "15:30 london in new york". Zones come from macOS's own database (`NSTimeZone`), so daylight
//! saving is right; cities are the database's (`Asia/Tokyo` -> "tokyo") plus a few common aliases.

use objc2::rc::Retained;
use objc2_foundation::{NSDate, NSString, NSTimeZone};

use crate::calc::Answer;

/// Abbreviations by region (so "PST" in July still means Los Angeles time), cities the database
/// names differently, and countries with one main zone.
const ALIASES: [(&str, &str); 47] = [
    ("pt", "America/Los_Angeles"),
    ("pst", "America/Los_Angeles"),
    ("pdt", "America/Los_Angeles"),
    ("pacific", "America/Los_Angeles"),
    ("mt", "America/Denver"),
    ("mst", "America/Denver"),
    ("mdt", "America/Denver"),
    ("ct", "America/Chicago"),
    ("cst", "America/Chicago"),
    ("cdt", "America/Chicago"),
    ("central", "America/Chicago"),
    ("et", "America/New_York"),
    ("est", "America/New_York"),
    ("edt", "America/New_York"),
    ("eastern", "America/New_York"),
    ("utc", "UTC"),
    ("gmt", "GMT"),
    ("bst", "Europe/London"),
    ("cet", "Europe/Paris"),
    ("cest", "Europe/Paris"),
    ("eet", "Europe/Athens"),
    ("ist", "Asia/Kolkata"),
    ("jst", "Asia/Tokyo"),
    ("kst", "Asia/Seoul"),
    ("hkt", "Asia/Hong_Kong"),
    ("sgt", "Asia/Singapore"),
    ("aest", "Australia/Sydney"),
    ("aedt", "Australia/Sydney"),
    ("sf", "America/Los_Angeles"),
    ("san francisco", "America/Los_Angeles"),
    ("seattle", "America/Los_Angeles"),
    ("nyc", "America/New_York"),
    ("boston", "America/New_York"),
    ("miami", "America/New_York"),
    ("austin", "America/Chicago"),
    ("dallas", "America/Chicago"),
    ("beijing", "Asia/Shanghai"),
    ("mumbai", "Asia/Kolkata"),
    ("delhi", "Asia/Kolkata"),
    ("bangalore", "Asia/Kolkata"),
    ("japan", "Asia/Tokyo"),
    ("india", "Asia/Kolkata"),
    ("china", "Asia/Shanghai"),
    ("uk", "Europe/London"),
    ("germany", "Europe/Berlin"),
    ("france", "Europe/Paris"),
    ("australia", "Australia/Sydney"),
];

struct Zone {
    tz: Retained<NSTimeZone>,
    /// "Tokyo", "Local".
    label: String,
}

fn city_of(id: &str) -> String {
    id.rsplit('/').next().unwrap_or(id).replace('_', " ")
}

fn zone(name: &str) -> Option<Zone> {
    let n = name.trim().trim_end_matches('?').trim().to_lowercase();
    if n.is_empty() || n.len() > 40 {
        return None;
    }
    if matches!(n.as_str(), "local" | "here" | "my time" | "local time") {
        let tz = NSTimeZone::localTimeZone();
        return Some(Zone { label: format!("Local ({})", city_of(&tz.name().to_string())), tz });
    }
    let by_name = |id: &str| NSTimeZone::timeZoneWithName(&NSString::from_str(id));
    if let Some((_, id)) = ALIASES.iter().find(|(a, _)| *a == n) {
        let label = if n.len() <= 4 { n.to_uppercase() } else { title(&n) };
        return by_name(id).map(|tz| Zone { tz, label });
    }
    for id in NSTimeZone::knownTimeZoneNames().iter() {
        let id = id.to_string();
        let city = city_of(&id);
        if city.to_lowercase() == n || id.to_lowercase() == n {
            return by_name(&id).map(|tz| Zone { tz, label: city });
        }
    }
    if n.len() <= 5 && n.chars().all(|c| c.is_ascii_alphabetic()) {
        let tz = NSTimeZone::timeZoneWithAbbreviation(&NSString::from_str(&n.to_uppercase()))?;
        return Some(Zone { tz, label: n.to_uppercase() });
    }
    None
}

fn title(s: &str) -> String {
    s.split(' ')
        .map(|w| {
            let mut c = w.chars();
            c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn offset_at(tz: &NSTimeZone, unix: f64) -> i64 {
    tz.secondsFromGMTForDate(&NSDate::dateWithTimeIntervalSince1970(unix)) as i64
}

fn abbreviation_at(tz: &NSTimeZone, unix: f64) -> String {
    tz.abbreviationForDate(&NSDate::dateWithTimeIntervalSince1970(unix)).map(|s| s.to_string()).unwrap_or_default()
}

// ── Portable parts ─────────────────────────────────────────────────────────

/// Minutes after midnight: "3pm", "3:30 pm", "15:00", "noon", "midnight". A bare "3" is not a time.
pub fn parse_clock(s: &str) -> Option<u32> {
    let t = s.trim().to_lowercase().replace('.', "");
    match t.as_str() {
        "noon" | "midday" => return Some(720),
        "midnight" => return Some(0),
        _ => {}
    }
    let (num, half) = if let Some(n) = t.strip_suffix("am") {
        (n.trim(), Some(false))
    } else if let Some(n) = t.strip_suffix("pm") {
        (n.trim(), Some(true))
    } else {
        (t.as_str(), None)
    };
    let (h, m) = match num.split_once(':') {
        Some((h, m)) if m.len() == 2 => (h.parse::<u32>().ok()?, m.parse::<u32>().ok()?),
        None if half.is_some() => (num.parse::<u32>().ok()?, 0),
        _ => return None,
    };
    if m > 59 {
        return None;
    }
    let h = match half {
        Some(pm) if (1..=12).contains(&h) => h % 12 + if pm { 12 } else { 0 },
        Some(_) => return None,
        None if h < 24 => h,
        None => return None,
    };
    Some(h * 60 + m)
}

/// Local seconds since the epoch -> "3:05 PM".
pub fn clock(local: i64) -> String {
    let mins = local.rem_euclid(86_400) / 60;
    let (h, m) = (mins / 60, mins % 60);
    let h12 = if h % 12 == 0 { 12 } else { h % 12 };
    format!("{h12}:{m:02} {}", if h < 12 { "AM" } else { "PM" })
}

/// Local seconds since the epoch -> "Sun".
pub fn weekday(local: i64) -> &'static str {
    // 1970-01-01 was a Thursday.
    ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"][local.div_euclid(86_400).rem_euclid(7) as usize]
}

/// Offset difference -> "same time", "16 h ahead", "5.5 h behind".
pub fn relative(diff_secs: i64) -> String {
    if diff_secs == 0 {
        return "same time as here".into();
    }
    let h = diff_secs.abs() as f64 / 3600.0;
    let n = if h.fract() == 0.0 { format!("{h}") } else { format!("{h:.1}").trim_end_matches('0').to_string() };
    format!("{n} h {}", if diff_secs > 0 { "ahead" } else { "behind" })
}

// ── Answers ────────────────────────────────────────────────────────────────

fn now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

fn now_in(z: &Zone) -> Answer {
    let t = now();
    let off = offset_at(&z.tz, t);
    let here = offset_at(&NSTimeZone::localTimeZone(), t);
    let local = t as i64 + off;
    let shown = clock(local);
    Answer {
        display: shown.clone(),
        copy: shown,
        detail: format!("{} · {} · {} · {}", z.label, weekday(local), abbreviation_at(&z.tz, t), relative(off - here)),
    }
}

/// `minutes` today in `from` (local when None), shown in `to`.
fn convert(minutes: u32, from: Option<Zone>, to: &Zone) -> Answer {
    let from = from.unwrap_or_else(|| {
        let tz = NSTimeZone::localTimeZone();
        Zone { label: "Local".into(), tz }
    });
    let t = now();
    let off_from = offset_at(&from.tz, t);
    let day_start = (t as i64 + off_from).div_euclid(86_400) * 86_400;
    let utc = day_start + minutes as i64 * 60 - off_from;
    let off_to = offset_at(&to.tz, utc as f64);
    let (src, dst) = (utc + off_from, utc + off_to);
    let mut shown = clock(dst);
    if dst.div_euclid(86_400) != src.div_euclid(86_400) {
        shown = format!("{shown} {}", weekday(dst));
    }
    Answer {
        display: shown.clone(),
        copy: clock(dst),
        detail: format!(
            "{} {} = {} {} {}",
            clock(src),
            abbreviation_at(&from.tz, utc as f64),
            clock(dst),
            abbreviation_at(&to.tz, utc as f64),
            to.label
        ),
    }
}

/// "<clock> [zone]" -> (minutes, zone); the clock is the first one or two words.
fn clock_and_zone(left: &str) -> Option<(u32, Option<Zone>)> {
    let words: Vec<&str> = left.split_whitespace().collect();
    for k in (1..=words.len().min(2)).rev() {
        if let Some(m) = parse_clock(&words[..k].join(" ")) {
            let rest = words[k..].join(" ");
            if rest.is_empty() {
                return Some((m, None));
            }
            if let Some(z) = zone(&rest) {
                return Some((m, Some(z)));
            }
        }
    }
    None
}

pub fn answer(input: &str) -> Option<Answer> {
    let mut t = input.trim().trim_end_matches('?').trim().to_lowercase();
    for p in ["what time is it in ", "what's the time in ", "whats the time in ", "current time in "] {
        if let Some(rest) = t.strip_prefix(p) {
            t = format!("time in {rest}");
        }
    }
    if let Some(rest) = t.strip_prefix("time in ").or_else(|| t.strip_prefix("now in ")).or_else(|| t.strip_prefix("time ")) {
        return zone(rest).map(|z| now_in(&z));
    }
    if let Some(rest) = t.strip_suffix(" time") {
        return zone(rest).map(|z| now_in(&z));
    }
    for sep in [" in ", " to "] {
        if let Some(i) = t.rfind(sep) {
            let (left, right) = (&t[..i], &t[i + sep.len()..]);
            if let (Some((m, from)), Some(to)) = (clock_and_zone(left), zone(right)) {
                return Some(convert(m, from, &to));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clocks() {
        assert_eq!(parse_clock("3pm"), Some(900));
        assert_eq!(parse_clock("3:30 PM"), Some(930));
        assert_eq!(parse_clock("12am"), Some(0));
        assert_eq!(parse_clock("12 pm"), Some(720));
        assert_eq!(parse_clock("15:05"), Some(905));
        assert_eq!(parse_clock("noon"), Some(720));
        for bad in ["3", "13pm", "25:00", "3:7", "tokyo"] {
            assert_eq!(parse_clock(bad), None, "{bad}");
        }
        assert_eq!(clock(0), "12:00 AM");
        assert_eq!(clock(13 * 3600 + 5 * 60), "1:05 PM");
        assert_eq!(weekday(0), "Thu");
        assert_eq!(relative(9 * 3600), "9 h ahead");
        assert_eq!(relative(-19_800), "5.5 h behind");
    }

    #[test]
    fn zones_and_answers() {
        assert_eq!(zone("tokyo").map(|z| z.tz.name().to_string()).as_deref(), Some("Asia/Tokyo"));
        assert_eq!(zone("New York").map(|z| z.label).as_deref(), Some("New York"));
        assert_eq!(zone("pst").map(|z| z.tz.name().to_string()).as_deref(), Some("America/Los_Angeles"));
        assert!(zone("screen").is_none() && zone("notes").is_none());
        let a = answer("time in tokyo").unwrap();
        assert!(a.display.ends_with('M') && a.detail.starts_with("Tokyo · "), "{a:?}");
        assert!(answer("tokyo time").is_some() && answer("what time is it in london?").is_some());
        // Tokyo has no daylight saving: noon there is always 03:00 UTC.
        let c = answer("noon tokyo in utc").unwrap();
        assert_eq!(c.copy, "3:00 AM", "{c:?}");
        let d = answer("3pm utc to jst").unwrap();
        assert_eq!(d.copy, "12:00 AM");
        assert!(d.display.ends_with(|c: char| c.is_alphabetic()) && d.display.len() > 8, "next day shown: {d:?}");
        for none in ["screen time", "notes to self", "5 km in mi", "safari"] {
            assert!(answer(none).is_none(), "{none}");
        }
    }
}
