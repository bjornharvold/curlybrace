use super::*;

fn decode(chunks: &[&[u8]]) -> String {
    let mut decoder = Utf8Decoder::default();
    let mut out: String = chunks.iter().map(|c| decoder.push(c)).collect();
    out.push_str(&decoder.finish());
    out
}

#[test]
fn ascii_passes_through() {
    assert_eq!(decode(&[b"abc", b"def"]), "abcdef");
}

#[test]
fn a_character_split_across_pushes_is_kept_whole() {
    let mut decoder = Utf8Decoder::default();
    assert_eq!(decoder.push(b"caf\xC3"), "caf");
    assert_eq!(decoder.push(b"\xA9!"), "\u{e9}!");
    assert_eq!(decoder.finish(), "");
}

#[test]
fn a_four_byte_character_split_three_ways_is_kept_whole() {
    assert_eq!(decode(&[b"\xF0\x9F", b"\x98", b"\x80"]), "\u{1F600}");
}

#[test]
fn invalid_bytes_become_replacement_characters() {
    assert_eq!(decode(&[b"\xFF\xFEabc"]), "\u{FFFD}\u{FFFD}abc");
}

#[test]
fn an_unfinished_character_at_the_end_becomes_one_replacement_character() {
    assert_eq!(decode(&[b"ab\xE2\x82"]), "ab\u{FFFD}");
}

#[test]
fn a_broken_sequence_is_replaced_and_decoding_continues() {
    assert_eq!(decode(&[b"\xC3", b"A"]), "\u{FFFD}A");
}
