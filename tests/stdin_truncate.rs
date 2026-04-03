use aihelp::prompt::{build_user_message, truncate_stdin_bytes, StdinContext};

#[test]
fn stdin_truncation_logic_is_correct() {
    let input = b"abcdefghijklmnopqrstuvwxyz";
    let (out, truncated) = truncate_stdin_bytes(input, 10);

    assert!(truncated);
    assert_eq!(out, b"abcdefghij");

    let (out2, truncated2) = truncate_stdin_bytes(input, 26);
    assert!(!truncated2);
    assert_eq!(out2, input);
}

#[test]
fn prompt_mentions_truncation_when_needed() {
    let ctx = StdinContext {
        content: "abc".to_string(),
        truncated: true,
        bytes_read: 3,
        max_bytes: 3,
    };

    let msg = build_user_message("what is this?", Some(&ctx));
    assert!(msg.contains("stdin was truncated"));
}

#[test]
fn truncate_at_2byte_utf8_boundary() {
    // "aé" in UTF-8: [0x61, 0xC3, 0xA9] — 3 bytes total
    let input = "aé".as_bytes();
    assert_eq!(input.len(), 3);

    // max=2 would cut the 2-byte é in half; must back up to 'a'
    let (out, truncated) = truncate_stdin_bytes(input, 2);
    assert!(truncated);
    assert_eq!(out, b"a");
}

#[test]
fn truncate_at_3byte_utf8_boundary() {
    // "€" in UTF-8: [0xE2, 0x82, 0xAC] — 3 bytes total
    let input = "€".as_bytes();
    assert_eq!(input.len(), 3);

    // max=2 would cut inside the 3-byte sequence; must back up to empty
    let (out, truncated) = truncate_stdin_bytes(input, 2);
    assert!(truncated);
    assert!(out.is_empty());
}

#[test]
fn truncate_at_4byte_utf8_boundary() {
    // "𝄞" (musical symbol G clef) in UTF-8: [0xF0, 0x9D, 0x84, 0x9E] — 4 bytes
    let input = "𝄞".as_bytes();
    assert_eq!(input.len(), 4);

    // max=3 would cut inside the 4-byte sequence; must back up to empty
    let (out, truncated) = truncate_stdin_bytes(input, 3);
    assert!(truncated);
    assert!(out.is_empty());
}

#[test]
fn build_user_message_without_stdin_context() {
    let msg = build_user_message("what is this?", None);
    assert!(msg.contains("Question:"));
    assert!(msg.contains("what is this?"));
    assert!(!msg.contains("Context (stdin)"));
}
