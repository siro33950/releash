/// ブランチ（ローカル / リモート）を表すエンティティ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Branch {
    pub name: String,
    pub is_remote: bool,
}

impl Branch {
    pub fn local(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            is_remote: false,
        }
    }

    pub fn remote(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            is_remote: true,
        }
    }
}

#[cfg(test)]
mod branch_tests {
    use super::*;

    #[test]
    fn test_ブランチ生成_ローカルとリモートの区別() {
        assert!(!Branch::local("main").is_remote);
        assert!(Branch::remote("feature").is_remote);
    }
}
