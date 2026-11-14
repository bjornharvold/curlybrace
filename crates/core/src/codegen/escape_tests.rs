use super::*;

#[test]
fn a_string_cannot_break_out_of_its_literal() {
    let text = "a\"b\\c\nd\re\tf\u{0}g\u{1f}h\u{7f}i\u{2028}j";
    let out = double_quoted(text, unicode_escape);
    assert_eq!(
        out,
        "\"a\\\"b\\\\c\\nd\\re\\tf\\u0000g\\u001fh\\u007fi\\u2028j\""
    );
    assert!(!out.contains('\n'));
}

#[test]
fn text_with_sigils_and_unicode_is_left_alone() {
    let out = double_quoted(
        "${x} #{y} $z %s {} emoji \u{1F600} e\u{301}",
        unicode_escape,
    );
    assert_eq!(out, "\"${x} #{y} $z %s {} emoji \u{1F600} e\u{301}\"");
}

#[test]
fn doubles_keep_a_decimal_point_or_exponent() {
    assert_eq!(float_text(1.0), "1.0");
    assert_eq!(float_text(-0.0), "-0.0");
    assert_eq!(float_text(1.5), "1.5");
    assert_eq!(float_text(1e300), "1e300");
}

#[test]
fn base64_and_hex_match_known_vectors() {
    assert_eq!(base64(b""), "");
    assert_eq!(base64(b"f"), "Zg==");
    assert_eq!(base64(b"fo"), "Zm8=");
    assert_eq!(base64(b"foo"), "Zm9v");
    assert_eq!(base64(&[0, 1, 2, 255]), "AAEC/w==");
    assert_eq!(hex(&[0, 1, 255]), "0001ff");
}

#[test]
fn uuid_text_needs_sixteen_bytes() {
    let bytes: Vec<u8> = (0..16).collect();
    assert_eq!(
        uuid_text(&bytes).as_deref(),
        Some("00010203-0405-0607-0809-0a0b0c0d0e0f")
    );
    assert_eq!(uuid_text(&bytes[..15]), None);
}

#[test]
fn civil_time_handles_epoch_pre_1970_and_leap_days() {
    assert_eq!(civil_from_millis(0).iso(), "1970-01-01T00:00:00.000Z");
    assert_eq!(civil_from_millis(-1).iso(), "1969-12-31T23:59:59.999Z");
    assert_eq!(
        civil_from_millis(1_709_210_096_789).iso(),
        "2024-02-29T12:34:56.789Z"
    );
    assert_eq!(
        civil_from_millis(-62_135_596_800_000).iso(),
        "0001-01-01T00:00:00.000Z"
    );
}

#[test]
fn civil_time_flags_years_outside_the_common_range() {
    assert!(civil_from_millis(0).in_common_range());
    assert!(!civil_from_millis(-62_135_596_800_001).in_common_range());
    assert!(!civil_from_millis(253_402_300_800_000).in_common_range());
    assert_eq!(civil_from_millis(253_402_300_800_000).year, 10_000);
}

#[test]
fn invisible_and_bidi_characters_are_flagged_and_normal_text_is_not() {
    for c in [
        '\u{0}',
        '\u{1f}',
        '\u{7f}',
        '\u{80}',
        '\u{85}',
        '\u{9f}',
        '\u{61c}',
        '\u{200b}',
        '\u{200f}',
        '\u{2028}',
        '\u{2029}',
        '\u{202a}',
        '\u{202e}',
        '\u{2060}',
        '\u{2069}',
        '\u{feff}',
        '\u{fffe}',
        '\u{ffff}',
        '\u{ad}',
        '\u{180e}',
        '\u{fff9}',
        '\u{fffa}',
        '\u{fffb}',
        '\u{e0000}',
        '\u{e0001}',
        '\u{e0020}',
        '\u{e007f}',
    ] {
        assert!(is_unsafe_invisible(c), "{:x}", c as u32);
    }
    for c in [
        ' ',
        'a',
        '"',
        '\u{a0}',
        '\u{a1}',
        '\u{e9}',
        '\u{301}',
        '\u{200a}',
        '\u{2010}',
        '\u{205f}',
        '\u{206a}',
        '\u{1F600}',
        '\u{fffd}',
        '\u{ac}',
        '\u{ae}',
        '\u{180d}',
        '\u{180f}',
        '\u{fff8}',
        '\u{fffc}',
        '\u{dffff}',
        '\u{e0080}',
        '\u{e0100}',
    ] {
        assert!(!is_unsafe_invisible(c), "{:x}", c as u32);
    }
}

#[test]
fn bidi_characters_are_escaped_in_double_quoted_literals() {
    let out = double_quoted(
        "a\u{202e}b\u{200b}c\u{feff}d\u{85}e\u{301}\u{1F600}",
        x_or_u_escape,
    );
    assert_eq!(out, "\"a\\u202eb\\u200bc\\ufeffd\\x85e\u{301}\u{1F600}\"");
}

#[test]
fn characters_beyond_the_bmp_are_escaped_the_way_each_language_reads_them() {
    // U+E0001 (a tag character) is above U+FFFF: a bare `\ue0001` would be U+E000 then `1`.
    let text = "a\u{e0001}b";
    assert_eq!(double_quoted(text, unicode_escape), "\"a\\udb40\\udc01b\"");
    assert_eq!(double_quoted(text, x_or_u_escape), "\"a\\U000e0001b\"");
    assert_eq!(double_quoted(text, go_escape), "\"a\\U000e0001b\"");
    assert_eq!(double_quoted("\u{ad}", go_escape), "\"\\u00ad\"");
    assert_eq!(double_quoted("\u{ad}", x_or_u_escape), "\"\\xad\"");
}

#[test]
fn plain_text_is_single_quoted_with_only_backslash_and_quote_escaped() {
    assert_eq!(
        single_or_double_quoted("it's a\\b $x #{y} \"q\"", '$'),
        "'it\\'s a\\\\b $x #{y} \"q\"'"
    );
}

#[test]
fn text_with_invisible_characters_switches_to_a_safe_double_quoted_form() {
    let php = single_or_double_quoted("a\n$x {$y} \"\\\u{202e}", '$');
    assert_eq!(php, "\"a\\n\\$x {\\$y} \\\"\\\\\\u{202e}\"");
    let ruby = single_or_double_quoted("#{evil} #$x \u{200b}", '#');
    assert_eq!(ruby, "\"\\#{evil} \\#$x \\u{200b}\"");
}
