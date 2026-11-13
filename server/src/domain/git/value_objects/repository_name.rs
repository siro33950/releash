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
        todo!()
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
    todo!()
}
