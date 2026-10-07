//! PathAliases — 起動環境（dev / 本番）から一意に決まる CLI alias の解決単位。
//!
//! alias 名・実行 binary・データディレクトリの三者を組として保持する。
//! 子プロセス起動経路（PTY / oneshot / agent bridge）が同じソースから値を引く。
//!
//! [01] CLI alias と実行対象の一意な対応:
//! 本番ビルド (release) → alias 名 `releash` / data dir `com.releash.app`
//! dev ビルド (debug)   → alias 名 `releash-dev` / data dir `com.releash.app.dev`

use std::path::{Path, PathBuf};

pub use releash_sdk::data_dir::BuildProfile;

/// `BuildProfile` から CLI alias 名を決定する。
pub fn alias_name_for_profile(profile: BuildProfile) -> &'static str {
    match profile {
        BuildProfile::Production => "releash",
        BuildProfile::Development => "releash-dev",
    }
}

/// 単一の path alias 解決単位。
#[derive(Debug, Clone)]
pub struct PathAlias {
    /// agent / facet に提示する alias 名（例: `releash`, `releash-dev`）。
    pub name: String,
    /// alias が実行する binary 実体への絶対パス。
    pub exe_path: PathBuf,
    /// alias が内包するデータディレクトリ。
    /// 子プロセスに `RELEASH_DATA_DIR` として伝搬する既定値。
    pub data_dir: PathBuf,
}

/// 起動環境から確定する path alias 集合。
///
/// 当面 `releash` キーのみを公開する（spec scope: `path_alias.releash` のみ）。
#[derive(Debug, Clone)]
pub struct PathAliases {
    releash: PathAlias,
}

impl PathAliases {
    #[cfg(feature = "test-support")]
    pub fn test_from_releash(releash: PathAlias) -> Self {
        Self { releash }
    }
    /// 起動環境から `PathAliases` を構築する。
    ///
    /// daemon が解決した data_dir を使い、子プロセスの接続先を一致させる。
    pub fn from_runtime(data_dir: PathBuf) -> Result<Self, std::io::Error> {
        let profile = BuildProfile::current();
        let exe_path = std::env::current_exe().map_err(|e| {
            std::io::Error::new(
                e.kind(),
                format!("failed to resolve current executable path: {e}"),
            )
        })?;
        let exe_path = exe_path.with_file_name("releash");
        let name = alias_name_for_profile(profile);
        Ok(Self {
            releash: PathAlias {
                name: name.to_string(),
                exe_path,
                data_dir,
            },
        })
    }

    /// `releash` alias の解決結果を返す。
    pub fn releash(&self) -> &PathAlias {
        &self.releash
    }
}

/// 子プロセスへ alias 解決可能な PATH と alias 内包の `RELEASH_DATA_DIR` を提供する。
///
/// spec issues-1054 「agent 子プロセスへの実行環境の伝搬」:
/// PTY / oneshot / agent bridge いずれの起動経路でも、起動環境に対応する CLI alias が
/// `PATH` 経由で解決可能で、`RELEASH_DATA_DIR` が起動環境別に設定される必要がある。
///
/// `PATH` には alias wrapper の bin dir を**先頭**に追加する。既存 PATH 前方に
/// `releash` / `releash-dev` を含む別ディレクトリ（例: `/usr/local/bin`）があると
/// 末尾追加では wrapper が解決されず alias と実行 binary の一意対応 (spec [01]) が崩れる。
/// wrapper bin dir に置く実体は `releash` / `releash-dev` の wrapper のみで、他システム
/// コマンド (`node` / `git` / `sh` 等) を shadow する経路は生じない。
///
/// `RELEASH_DATA_DIR` の解決順序（spec: 明示指定 > alias 内包値 > プロセス既定）は
/// 本 helper の入力 `parent_releash_data_dir` で守る:
/// - Releash プロセス自身に明示指定がある場合（`Some(...)`）は alias 内包値で上書き
///   しない（戻り値に `RELEASH_DATA_DIR` を含めない）。子プロセスは inherit で
///   親の明示値を受け取る。
/// - 明示指定が無い場合（`None`）は alias 内包値を戻り値に積む。
pub fn child_env_overrides(aliases: &PathAliases) -> Result<Vec<(String, String)>, std::io::Error> {
    child_env_overrides_from(
        aliases,
        std::env::var("PATH").ok().as_deref(),
        std::env::var("RELEASH_DATA_DIR").ok().as_deref(),
    )
}

/// `child_env_overrides` の pure 版。env をパラメータで受け取り副作用を持たない。
///
/// `parent_releash_data_dir` の Some / None で alias 内包値の上書き有無が分岐する
/// （spec 解決順序: 明示指定 > alias 内包値）。テストはこちらを直接叩く。
pub fn child_env_overrides_from(
    aliases: &PathAliases,
    parent_path: Option<&str>,
    parent_releash_data_dir: Option<&str>,
) -> Result<Vec<(String, String)>, std::io::Error> {
    let releash = aliases.releash();
    let bin_dir = ensure_alias_wrapper(releash)?;
    let path_value = compose_path_with_alias_bin(parent_path, &bin_dir);
    let mut env = vec![("PATH".to_string(), path_value)];
    // spec issues-1054: 利用者明示指定（親プロセスの RELEASH_DATA_DIR）は alias 内包値で
    // 上書きしない。明示指定が無いときだけ alias 既定値を子プロセス env に積む。
    if parent_releash_data_dir.is_none_or(str::is_empty) {
        env.push((
            "RELEASH_DATA_DIR".to_string(),
            releash.data_dir.display().to_string(),
        ));
    }
    Ok(env)
}

/// alias の wrapper bin dir を既存 `PATH` の**先頭**に追加した値を組み立てる。
///
/// 末尾追加にすると、既存 PATH 前方に `releash` / `releash-dev` を含む別ディレクトリが
/// あった場合に wrapper が解決されず alias と実行 binary の一意対応 (spec [01]) が崩れる。
/// wrapper bin dir に置くファイルは `releash` / `releash-dev` の wrapper のみで、
/// 他システムコマンドを shadow する経路は生じないため、先頭挿入で問題ない。
fn compose_path_with_alias_bin(existing_path: Option<&str>, bin_dir: &Path) -> String {
    match existing_path {
        Some(existing) if !existing.is_empty() => format!("{}:{}", bin_dir.display(), existing),
        _ => bin_dir.display().to_string(),
    }
}

/// PTY / oneshot / agent bridge 起動経路の env 準備ロジックの単一エントリ。
///
/// spec issues-1054 「agent 子プロセスへの実行環境の伝搬」: 各経路の env 構築を
/// `PathAliases::from_runtime` → `child_env_overrides` の組として一箇所に束ね、
/// テストから直接検証可能にする。
///
/// - `data_dir` が `None` の場合（data_dir 解決失敗時など）は空の env を
///   返し、呼び出し側が alias なしで spawn する既存挙動を温存する。
/// - `data_dir` が `Some` の場合に wrapper 作成等で失敗したら `Err` を返し、呼び出し側で
///   spawn を中止する。
pub fn prepare_child_env(
    data_dir: Option<PathBuf>,
) -> Result<Vec<(String, String)>, std::io::Error> {
    let Some(data_dir) = data_dir else {
        return Ok(Vec::new());
    };
    let aliases = PathAliases::from_runtime(data_dir)?;
    child_env_overrides(&aliases)
}

/// `<data_dir>/bin/<alias_name>` に CLI 実行 binary を指す wrapper を用意し、bin dir を返す。
///
/// wrapper はシェルスクリプトで、呼び出し側が `RELEASH_DATA_DIR` を明示指定していない
/// 場合のみ alias 内包の data_dir を設定する（spec 解決順序: 明示指定 > alias 内包値）。
pub fn ensure_alias_wrapper(releash: &PathAlias) -> Result<PathBuf, std::io::Error> {
    let bin_dir = releash.data_dir.join("bin");
    std::fs::create_dir_all(&bin_dir).map_err(|e| {
        std::io::Error::new(
            e.kind(),
            format!("failed to create alias bin dir {}: {e}", bin_dir.display()),
        )
    })?;
    let wrapper_path = bin_dir.join(&releash.name);
    let exe_str = releash.exe_path.to_str().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "exe_path is not valid UTF-8",
        )
    })?;
    let data_dir_str = releash.data_dir.to_str().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "data_dir is not valid UTF-8",
        )
    })?;
    let script = build_wrapper_script(exe_str, data_dir_str);
    // 既存の wrapper と内容が同じ場合は書き換えを省く（mtime ノイズを避ける）。
    if wrapper_path.exists() {
        if let Ok(existing) = std::fs::read_to_string(&wrapper_path) {
            if existing == script {
                return Ok(bin_dir);
            }
        }
    }
    write_wrapper_script(&wrapper_path, &script)?;
    Ok(bin_dir)
}

fn build_wrapper_script(exe_path: &str, data_dir: &str) -> String {
    // 呼び出し側の明示指定を奪わない: `RELEASH_DATA_DIR` 未設定時のみ alias 内包値を使う。
    format!(
        "#!/bin/sh\nif [ -z \"$RELEASH_DATA_DIR\" ]; then\n  export RELEASH_DATA_DIR={}\nfi\nexec {} \"$@\"\n",
        shell_quote(data_dir),
        shell_quote(exe_path)
    )
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(unix)]
fn write_wrapper_script(path: &Path, script: &str) -> Result<(), std::io::Error> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, script).map_err(|e| {
        std::io::Error::new(
            e.kind(),
            format!("failed to write wrapper {}: {e}", path.display()),
        )
    })?;
    let mut perms = std::fs::metadata(path)
        .map_err(|e| {
            std::io::Error::new(
                e.kind(),
                format!("failed to stat wrapper {}: {e}", path.display()),
            )
        })?
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).map_err(|e| {
        std::io::Error::new(
            e.kind(),
            format!("failed to chmod wrapper {}: {e}", path.display()),
        )
    })?;
    Ok(())
}

#[cfg(not(unix))]
fn write_wrapper_script(path: &Path, script: &str) -> Result<(), std::io::Error> {
    std::fs::write(path, script).map_err(|e| {
        std::io::Error::new(
            e.kind(),
            format!("failed to write wrapper {}: {e}", path.display()),
        )
    })
}

#[cfg(test)]
#[path = "path_aliases_test.rs"]
mod path_aliases_tests;
