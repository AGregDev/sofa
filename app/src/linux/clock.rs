//! The clock and the date in the person's own format (their locale's `LC_TIME`), as Windows'
//! GetTimeFormatEx and GetDateFormatEx give them.

use std::ffi::{CStr, CString};

fn local(at: i64) -> libc::tm {
    // SAFETY: localtime_r fills our own struct from our own time value.
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&(at as libc::time_t), &mut tm);
        tm
    }
}

fn now_secs() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

fn format(fmt: &str, tm: &libc::tm) -> String {
    let Ok(f) = CString::new(fmt) else { return String::new() };
    let mut buf = [0u8; 160];
    // SAFETY: strftime writes at most `buf.len()` bytes and returns how many it wrote.
    let n = unsafe { libc::strftime(buf.as_mut_ptr().cast(), buf.len(), f.as_ptr(), tm) };
    String::from_utf8_lossy(&buf[..n]).into_owned()
}

/// The locale's time without seconds: "%H:%M", or "%-I:%M %p" where clocks have 12 hours.
fn time_format() -> String {
    // SAFETY: nl_langinfo returns a string owned by the C library, valid until the next call.
    let raw = unsafe { CStr::from_ptr(libc::nl_langinfo(libc::T_FMT)) }.to_string_lossy().into_owned();
    let f = raw.replace("%r", "%I:%M:%S %p").replace("%T", "%H:%M:%S").replace(":%S", "").replace(".%S", "").replace("%I", "%-I");
    if f.trim().is_empty() { "%H:%M".into() } else { f }
}

/// Minutes since midnight, local time.
pub fn minute_of_day() -> u16 {
    let tm = local(now_secs());
    (tm.tm_hour * 60 + tm.tm_min) as u16
}

/// The time in the person's own format, without seconds.
pub fn clock_text() -> String {
    format(&time_format(), &local(now_secs()))
}

/// The time `minutes` from now ("11:42 PM").
pub fn clock_in(minutes: u32) -> String {
    format(&time_format(), &local(now_secs() + i64::from(minutes) * 60))
}

/// Today's date, long ("Friday, November 27, 2026"; the day first where that is the custom).
pub fn date_text() -> String {
    let lang = ["LC_ALL", "LC_TIME", "LANG"].iter().find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty())).unwrap_or_default();
    let month_first = lang.is_empty() || lang == "C" || lang.starts_with("C.") || lang == "POSIX" || lang.starts_with("en_US") || lang.starts_with("en_PH");
    format(if month_first { "%A, %B %-d, %Y" } else { "%A %-d %B %Y" }, &local(now_secs()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_have_no_seconds() {
        let t = clock_text();
        assert!(t.matches(':').count() == 1, "{t}");
        assert!(minute_of_day() < 24 * 60);
        assert!(!date_text().is_empty());
    }
}
