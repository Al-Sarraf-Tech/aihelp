use aihelp::client::{extract_sse_data, find_event_delimiter};

#[test]
fn lf_delimiter_found() {
    let buf = b"data: hello\n\ndata: world\n\n";
    let (pos, len) = find_event_delimiter(buf).unwrap();
    assert_eq!(pos, 11); // index of first \n in \n\n
    assert_eq!(len, 2);
}

#[test]
fn crlf_delimiter_found() {
    let buf = b"data: hello\r\n\r\ndata: world\r\n\r\n";
    let (pos, len) = find_event_delimiter(buf).unwrap();
    assert_eq!(pos, 11); // index of first \r in \r\n\r\n
    assert_eq!(len, 4);
}

#[test]
fn no_delimiter_returns_none() {
    let buf = b"data: partial event with no delimiter";
    assert!(find_event_delimiter(buf).is_none());
}

#[test]
fn single_newline_not_a_delimiter() {
    let buf = b"data: line1\ndata: line2\n";
    assert!(find_event_delimiter(buf).is_none());
}

#[test]
fn extract_single_data_line() {
    let data = extract_sse_data("data: {\"text\": \"hello\"}");
    assert_eq!(data, "{\"text\": \"hello\"}");
}

#[test]
fn extract_strips_cr_from_data_lines() {
    let data = extract_sse_data("data: hello\r");
    assert_eq!(data, "hello");
}

#[test]
fn extract_ignores_non_data_lines() {
    let block = "event: message\nid: 42\ndata: payload";
    let data = extract_sse_data(block);
    assert_eq!(data, "payload");
}

#[test]
fn extract_empty_block_returns_empty() {
    assert!(extract_sse_data("").is_empty());
    assert!(extract_sse_data("event: ping").is_empty());
}

#[test]
fn extract_done_marker() {
    let data = extract_sse_data("data: [DONE]");
    assert_eq!(data, "[DONE]");
}

#[test]
fn crlf_delimiter_preferred_over_lf() {
    // \r\n\r\n should be found before any embedded \n\n
    let buf = b"data: test\r\n\r\n";
    let (pos, len) = find_event_delimiter(buf).unwrap();
    assert_eq!(len, 4);
    assert_eq!(pos, 10);
}
