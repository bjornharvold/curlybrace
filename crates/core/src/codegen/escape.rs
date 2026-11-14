//! Helpers every renderer shares: indentation, string literals, number text,
//! base64/hex and calendar arithmetic. No renderer escapes text by itself.

/// `depth` repetitions of `unit`.
pub(crate) fn indent(unit: &str, depth: usize) -> String {
    unit.repeat(depth)
}

/// Prefixes every non-empty line of `text` with `depth` indent units.
pub(crate) fn indent_lines(text: &str, unit: &str, depth: usize) -> String {
    let prefix = indent(unit, depth);
    text.lines()
        .map(|line| {
            if line.is_empty() {
                String::new()
            } else {
                format!("{prefix}{line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A double-quoted string literal valid in JavaScript and Python (and most C
/// family languages): quotes, backslashes and every control character are
/// escaped, so the text can neither end the literal nor start an escape.
/// `hex_escape` renders a control character below U+0100 (`\x1f` style) or
/// above (` ` style) the way the target language spells it.
pub(crate) fn double_quoted(text: &str, hex_escape: fn(u32) -> String) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => out.push_str(&hex_escape(c as u32)),
            '\u{2028}' | '\u{2029}' => out.push_str(&hex_escape(c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// `\u00XX` style escape (JavaScript, Java, C#, Rust-adjacent).
pub(crate) fn unicode_escape(code: u32) -> String {
    format!("\\u{code:04x}")
}

/// A double with a decimal point or exponent so it re-reads as a float.
/// Non-finite values are the caller's business.
pub(crate) fn float_text(v: f64) -> String {
    format!("{v:?}")
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard padded base64.
pub(crate) fn base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            B64[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            B64[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// `8-4-4-4-12` text of 16 bytes, or `None` for any other length.
pub(crate) fn uuid_text(bytes: &[u8]) -> Option<String> {
    if bytes.len() != 16 {
        return None;
    }
    let h = hex(bytes);
    Some(format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    ))
}

/// Calendar fields of a UTC instant given in milliseconds since the epoch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Civil {
    pub year: i64,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    pub millis: u32,
}

const MS_PER_DAY: i64 = 86_400_000;

/// Proleptic Gregorian breakdown (works before 1970 and for huge years).
pub(crate) fn civil_from_millis(ms: i64) -> Civil {
    let days = ms.div_euclid(MS_PER_DAY);
    let rem = ms.rem_euclid(MS_PER_DAY);
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    Civil {
        year,
        month,
        day,
        hour: (rem / 3_600_000) as u32,
        minute: (rem / 60_000 % 60) as u32,
        second: (rem / 1000 % 60) as u32,
        millis: (rem % 1000) as u32,
    }
}

impl Civil {
    /// `2024-01-15T10:30:00.123Z`, only meaningful for years 1 to 9999.
    pub(crate) fn iso(&self) -> String {
        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
            self.year, self.month, self.day, self.hour, self.minute, self.second, self.millis
        )
    }

    /// Whether a four-digit ISO year and Python's `datetime` can hold it.
    pub(crate) fn in_common_range(&self) -> bool {
        (1..=9999).contains(&self.year)
    }
}

#[cfg(test)]
#[path = "escape_tests.rs"]
mod tests;
