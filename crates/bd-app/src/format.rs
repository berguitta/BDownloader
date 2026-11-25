//! Human-readable sizes, speeds and durations.

pub fn bytes(value: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = value as f64;
    let mut unit = 0;
    while v >= 1024.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    match unit {
        0 => format!("{value} B"),
        _ if v < 10.0 => format!("{v:.2} {}", UNITS[unit]),
        _ => format!("{v:.1} {}", UNITS[unit]),
    }
}

pub fn speed(bytes_per_sec: f64) -> String {
    if bytes_per_sec < 1.0 {
        return String::new();
    }
    format!("{}/s", bytes(bytes_per_sec as u64))
}

pub fn duration(secs: f64) -> String {
    if !secs.is_finite() || secs <= 0.0 {
        return String::new();
    }
    let s = secs.round() as u64;
    let (h, m, x) = (s / 3600, s % 3600 / 60, s % 60);
    if h > 0 {
        format!("{h}h {m:02}m")
    } else if m > 0 {
        format!("{m}m {x:02}s")
    } else {
        format!("{x}s")
    }
}

/// Local-time-free short date for "added" (UTC, YYYY-MM-DD HH:MM).
pub fn date(secs: u64) -> String {
    if secs == 0 {
        return String::new();
    }
    let days = (secs / 86_400) as i64;
    let (h, m) = ((secs % 86_400) / 3600, (secs % 3600) / 60);
    // Civil from days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(mo <= 2);
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{m:02}")
}

pub fn thousands(v: usize) -> String {
    let s = v.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        assert_eq!(bytes(512), "512 B");
        assert_eq!(bytes(5 * 1024 * 1024), "5.00 MB");
        assert_eq!(bytes(150 * 1024 * 1024), "150.0 MB");
        assert_eq!(speed(0.0), "");
        assert_eq!(duration(75.0), "1m 15s");
        assert_eq!(duration(3_700.0), "1h 01m");
        assert_eq!(thousands(1_234_567), "1,234,567");
        assert_eq!(date(1_764_000_000), "2025-11-24 16:00");
    }
}
