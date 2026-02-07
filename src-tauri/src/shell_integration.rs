use std::path::{Path, PathBuf};

const BASH_INIT_SCRIPT: &str = r#"
# Releash shell integration for bash
# Source user's bashrc
if [ -f "$HOME/.bashrc" ]; then
    source "$HOME/.bashrc"
fi

# Command completion hook
__releash_precmd() {
    local exit_code=$?
    printf '\033]777;cmd_done;%d\007' "$exit_code"
}

if [[ ! "${PROMPT_COMMAND:-}" == *"__releash_precmd"* ]]; then
    PROMPT_COMMAND="__releash_precmd${PROMPT_COMMAND:+;$PROMPT_COMMAND}"
fi
"#;

const ZSH_INIT_SCRIPT: &str = r#"
# Releash shell integration for zsh
# Source user's zshrc
if [[ -n "$RELEASH_USER_ZDOTDIR" && -f "$RELEASH_USER_ZDOTDIR/.zshrc" ]]; then
    source "$RELEASH_USER_ZDOTDIR/.zshrc"
elif [[ -f "$HOME/.zshrc" ]]; then
    source "$HOME/.zshrc"
fi

# Command completion hook
__releash_precmd() {
    local exit_code=$?
    printf '\033]777;cmd_done;%d\007' "$exit_code"
}

if (( ! ${precmd_functions[(I)__releash_precmd]} )); then
    precmd_functions=(__releash_precmd $precmd_functions)
fi
"#;

const OSC_PREFIX: &str = "\x1b]777;cmd_done;";
const OSC_TERMINATOR: char = '\x07';

pub fn create_shell_integration_files(data_dir: &Path) -> Result<PathBuf, String> {
    let dir = data_dir.join("shell-integration");
    std::fs::create_dir_all(&dir).map_err(|e| format!("シェル統合ディレクトリ作成失敗: {e}"))?;

    std::fs::write(dir.join("bash-init.sh"), BASH_INIT_SCRIPT)
        .map_err(|e| format!("bash init書き込み失敗: {e}"))?;

    let zsh_dir = dir.join("zsh");
    std::fs::create_dir_all(&zsh_dir).map_err(|e| format!("zsh dir作成失敗: {e}"))?;
    std::fs::write(zsh_dir.join(".zshrc"), ZSH_INIT_SCRIPT)
        .map_err(|e| format!("zsh init書き込み失敗: {e}"))?;

    Ok(dir)
}

pub struct OscParseResult {
    pub filtered_output: String,
    pub cmd_done_exit_codes: Vec<i32>,
}

pub fn strip_osc_cmd_done(data: &str) -> OscParseResult {
    let mut output = String::with_capacity(data.len());
    let mut exit_codes = Vec::new();
    let mut remaining = data;

    while let Some(start) = remaining.find(OSC_PREFIX) {
        output.push_str(&remaining[..start]);
        let after_prefix = &remaining[start + OSC_PREFIX.len()..];

        if let Some(end) = after_prefix.find(OSC_TERMINATOR) {
            let exit_code_str = &after_prefix[..end];
            if let Ok(exit_code) = exit_code_str.parse::<i32>() {
                exit_codes.push(exit_code);
            }
            remaining = &after_prefix[end + 1..];
        } else {
            // Incomplete sequence at the end - keep it (will be completed in next read)
            output.push_str(&remaining[start..]);
            remaining = "";
            break;
        }
    }

    output.push_str(remaining);

    OscParseResult {
        filtered_output: output,
        cmd_done_exit_codes: exit_codes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_no_osc() {
        let result = strip_osc_cmd_done("hello world\n");
        assert_eq!(result.filtered_output, "hello world\n");
        assert!(result.cmd_done_exit_codes.is_empty());
    }

    #[test]
    fn test_strip_single_cmd_done() {
        let data = "output\x1b]777;cmd_done;0\x07prompt$ ";
        let result = strip_osc_cmd_done(data);
        assert_eq!(result.filtered_output, "outputprompt$ ");
        assert_eq!(result.cmd_done_exit_codes, vec![0]);
    }

    #[test]
    fn test_strip_nonzero_exit_code() {
        let data = "error\x1b]777;cmd_done;1\x07$ ";
        let result = strip_osc_cmd_done(data);
        assert_eq!(result.filtered_output, "error$ ");
        assert_eq!(result.cmd_done_exit_codes, vec![1]);
    }

    #[test]
    fn test_strip_multiple_cmd_done() {
        let data = "a\x1b]777;cmd_done;0\x07b\x1b]777;cmd_done;127\x07c";
        let result = strip_osc_cmd_done(data);
        assert_eq!(result.filtered_output, "abc");
        assert_eq!(result.cmd_done_exit_codes, vec![0, 127]);
    }

    #[test]
    fn test_strip_preserves_other_osc() {
        let data = "text\x1b]0;title\x07more";
        let result = strip_osc_cmd_done(data);
        assert_eq!(result.filtered_output, "text\x1b]0;title\x07more");
        assert!(result.cmd_done_exit_codes.is_empty());
    }

    #[test]
    fn test_strip_negative_exit_code() {
        let data = "\x1b]777;cmd_done;-1\x07";
        let result = strip_osc_cmd_done(data);
        assert_eq!(result.filtered_output, "");
        assert_eq!(result.cmd_done_exit_codes, vec![-1]);
    }

    #[test]
    fn test_create_shell_integration_files() {
        let dir = tempfile::TempDir::new().unwrap();
        let result = create_shell_integration_files(dir.path()).unwrap();

        assert!(result.join("bash-init.sh").exists());
        assert!(result.join("zsh").join(".zshrc").exists());

        let bash_content = std::fs::read_to_string(result.join("bash-init.sh")).unwrap();
        assert!(bash_content.contains("__releash_precmd"));
        assert!(bash_content.contains("PROMPT_COMMAND"));

        let zsh_content = std::fs::read_to_string(result.join("zsh").join(".zshrc")).unwrap();
        assert!(zsh_content.contains("__releash_precmd"));
        assert!(zsh_content.contains("precmd_functions"));
    }
}
