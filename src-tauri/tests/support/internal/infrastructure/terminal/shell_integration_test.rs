pub(crate) mod tests {
    use super::super::*;

    #[test]
    pub fn test_create_shell_integration_files() {
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

        assert!(result.join("fish-init.fish").exists());
        let fish_content = std::fs::read_to_string(result.join("fish-init.fish")).unwrap();
        assert!(fish_content.contains("__releash_postexec"));
        assert!(fish_content.contains("fish_postexec"));
    }
}
