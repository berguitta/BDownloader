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
    }
}
