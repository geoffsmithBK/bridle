//! Local time formatting without a date crate, via libc.

/// Local time broken into fields: (year, month, day, hour, minute, second).
fn local_now() -> (i32, u32, u32, u32, u32, u32) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as libc::time_t)
        .unwrap_or(0);
    // SAFETY: localtime_r only writes into the struct we pass it.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&now, &mut tm) };
    (
        tm.tm_year + 1900,
        (tm.tm_mon + 1) as u32,
        tm.tm_mday as u32,
        tm.tm_hour as u32,
        tm.tm_min as u32,
        tm.tm_sec as u32,
    )
}

/// "2026-10-26"
pub fn today() -> String {
    let (y, mo, d, ..) = local_now();
    format!("{y:04}-{mo:02}-{d:02}")
}

/// "2026-10-26T18-51-02" (safe for file names)
pub fn file_stamp() -> String {
    let (y, mo, d, h, mi, s) = local_now();
    format!("{y:04}-{mo:02}-{d:02}T{h:02}-{mi:02}-{s:02}")
}
