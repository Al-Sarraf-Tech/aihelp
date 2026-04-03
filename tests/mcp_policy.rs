use aihelp::config::McpAllowPolicy;
use aihelp::mcp::{is_read_only_tool_name, is_tool_allowed};

#[test]
fn read_only_heuristic_allows_and_blocks_expected_names() {
    assert!(is_read_only_tool_name("read_file"));
    assert!(is_read_only_tool_name("search_docs"));
    assert!(is_read_only_tool_name("list-users"));

    assert!(!is_read_only_tool_name("write_file"));
    assert!(!is_read_only_tool_name("delete_record"));
    assert!(!is_read_only_tool_name("run_command"));
    assert!(!is_read_only_tool_name("spawn_job"));
    assert!(!is_read_only_tool_name("rm_cache"));
}

#[test]
fn allow_list_policy_requires_explicit_match() {
    let allow = vec!["search_docs".to_string(), "read_file".to_string()];

    assert!(is_tool_allowed(
        McpAllowPolicy::AllowList,
        &allow,
        "search_docs"
    ));
    assert!(!is_tool_allowed(
        McpAllowPolicy::AllowList,
        &allow,
        "delete_file"
    ));
}

#[test]
fn all_policy_allows_everything() {
    assert!(is_tool_allowed(McpAllowPolicy::All, &[], "anything_goes"));
}

#[test]
fn allow_list_with_empty_list_blocks_all_tools() {
    let empty: Vec<String> = vec![];
    assert!(!is_tool_allowed(
        McpAllowPolicy::AllowList,
        &empty,
        "read_file"
    ));
    assert!(!is_tool_allowed(
        McpAllowPolicy::AllowList,
        &empty,
        "any_tool"
    ));
}

#[test]
fn read_only_positive_with_negative_blocks() {
    // "read_and_delete" has a positive ("read") but also a negative ("delete")
    assert!(!is_read_only_tool_name("read_and_delete"));
}

#[test]
fn read_only_no_positive_blocks() {
    // "compute_stats" has no positive token match at all
    assert!(!is_read_only_tool_name("compute_stats"));
}

#[test]
fn rm_token_blocked_in_read_context() {
    // "list_rm_files" has positive ("list") but "rm" as a word token blocks it
    assert!(!is_read_only_tool_name("list_rm_files"));
}

#[test]
fn firmware_not_rm_false_positive() {
    // "get_firmware" has positive ("get") and "firmware" contains "rm" as a
    // substring but NOT as a standalone word token — should be allowed
    assert!(is_read_only_tool_name("get_firmware"));
}
