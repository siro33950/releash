use super::*;

#[test]
fn test_strip_no_osc() {
    let result = strip_osc_cmd_done("hello world\n");
    assert_eq!(result.filtered_output, "hello world\n");
}

#[test]
fn test_strip_single_cmd_done() {
    let data = "output\x1b]777;cmd_done;0\x07prompt$ ";
    let result = strip_osc_cmd_done(data);
    assert_eq!(result.filtered_output, "outputprompt$ ");
}

#[test]
fn test_strip_nonzero_exit_code() {
    let data = "error\x1b]777;cmd_done;1\x07$ ";
    let result = strip_osc_cmd_done(data);
    assert_eq!(result.filtered_output, "error$ ");
}

#[test]
fn test_strip_multiple_cmd_done() {
    let data = "a\x1b]777;cmd_done;0\x07b\x1b]777;cmd_done;127\x07c";
    let result = strip_osc_cmd_done(data);
    assert_eq!(result.filtered_output, "abc");
}

#[test]
fn test_strip_preserves_other_osc() {
    let data = "text\x1b]0;title\x07more";
    let result = strip_osc_cmd_done(data);
    assert_eq!(result.filtered_output, "text\x1b]0;title\x07more");
}

#[test]
fn test_strip_negative_exit_code() {
    let data = "\x1b]777;cmd_done;-1\x07";
    let result = strip_osc_cmd_done(data);
    assert_eq!(result.filtered_output, "");
}
