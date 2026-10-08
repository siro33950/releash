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

const FISH_INIT_SCRIPT: &str = r#"
# Releash shell integration for fish
function __releash_postexec --on-event fish_postexec
    printf '\033]777;cmd_done;%d\007' $status
end
"#;

const OSC_PREFIX: &str = "\x1b]777;cmd_done;";
const OSC_TERMINATOR: char = '\x07';

pub fn create_shell_integration_files(data_dir: &Path) -> Result<PathBuf, String> {
    let dir = data_dir.join("shell-integration");
    std::fs::create_dir_all(&dir).map_err(|e| format!("シェル統合ディレクトリ作成失敗: {e}"))?;

    std::fs::write(dir.join("bash-init.sh"), BASH_INIT_SCRIPT)
        .map_err(|e| format!("bash init書き込み失敗: {e}"))?;

    std::fs::write(dir.join("fish-init.fish"), FISH_INIT_SCRIPT)
        .map_err(|e| format!("fish init書き込み失敗: {e}"))?;

    let zsh_dir = dir.join("zsh");
    std::fs::create_dir_all(&zsh_dir).map_err(|e| format!("zsh dir作成失敗: {e}"))?;
    std::fs::write(zsh_dir.join(".zshrc"), ZSH_INIT_SCRIPT)
        .map_err(|e| format!("zsh init書き込み失敗: {e}"))?;

    Ok(dir)
}

pub struct OscParseResult {
    pub filtered_output: String,
}

pub fn strip_osc_cmd_done(data: &str) -> OscParseResult {
    let mut output = String::with_capacity(data.len());
    let mut remaining = data;

    while let Some(start) = remaining.find(OSC_PREFIX) {
        output.push_str(&remaining[..start]);
        let after_prefix = &remaining[start + OSC_PREFIX.len()..];

        if let Some(end) = after_prefix.find(OSC_TERMINATOR) {
            remaining = &after_prefix[end + 1..];
        } else {
            output.push_str(&remaining[start..]);
            remaining = "";
            break;
        }
    }

    output.push_str(remaining);

    OscParseResult {
        filtered_output: output,
    }
}

#[cfg(test)]
#[path = "shell_integration_test.rs"]
mod shell_integration_tests;
