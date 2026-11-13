/// origin が指す GitHub 上のリポジトリ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHubRepository {
    pub owner: String,
    pub name: String,
}

impl GitHubRepository {
    /// origin の URL が github.com のリポジトリを指すとき、owner と名前を読み取る。
    /// 指さないときは `None`。origin が GitHub のリポジトリかは、ここだけが決める。
    pub fn from_origin_url(url: &str) -> Option<Self> {
        let (authority, path) = match url.split_once("://") {
            Some((_, rest)) => rest.split_once('/')?,
            None => url.split_once(':')?,
        };
        let host = authority
            .rsplit_once('@')
            .map_or(authority, |(_, host)| host);
        if !host.eq_ignore_ascii_case("github.com") {
            return None;
        }
        let path = path.strip_suffix('/').unwrap_or(path);
        let (owner, name) = path.split_once('/')?;
        let name = name.strip_suffix(".git").unwrap_or(name);
        (!owner.is_empty() && !name.is_empty() && !name.contains('/')).then(|| Self {
            owner: owner.into(),
            name: name.into(),
        })
    }
}

/// Repository の名前を決める材料。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryNameSource {
    /// Repository のパス。
    pub path: String,
    /// origin の URL。origin が無い、または読めなかったときは `None`。
    pub origin_url: Option<String>,
}

/// 登録された Repository の並びから、各 Repository の名前を、同じ順で決める。
///
/// origin が GitHub のリポジトリなら GitHub 上のリポジトリ名、それ以外はフォルダ名にする。
/// 同じ名前が並ぶときだけ、origin が GitHub の Repository の名前を `owner/名前` にする。
pub fn repository_names(sources: &[RepositoryNameSource]) -> Vec<String> {
    let repositories = sources
        .iter()
        .map(|source| {
            let github = source
                .origin_url
                .as_deref()
                .and_then(GitHubRepository::from_origin_url);
            let name = match &github {
                Some(github) => github.name.clone(),
                None => folder_name(&source.path),
            };
            (name, github)
        })
        .collect::<Vec<_>>();
    repositories
        .iter()
        .map(|(name, github)| {
            let duplicated = repositories
                .iter()
                .filter(|(other, _)| other == name)
                .count()
                > 1;
            match github {
                Some(github) if duplicated => format!("{}/{}", github.owner, github.name),
                _ => name.clone(),
            }
        })
        .collect()
}

fn folder_name(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    trimmed
        .rsplit_once('/')
        .map_or(trimmed, |(_, name)| name)
        .to_string()
}

#[cfg(test)]
#[path = "repository_name_test.rs"]
mod repository_name_tests;
