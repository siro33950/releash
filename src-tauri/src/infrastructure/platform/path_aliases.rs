//! PathAliases — 起動環境（dev / 本番）から一意に決まる CLI alias の解決単位。
//!
//! alias 名・実行 binary・データディレクトリの三者を組として保持する。
//! 子プロセス起動経路（PTY / oneshot / agent bridge）が同じソースから値を引く。
//!
//! [01] CLI alias と実行対象の一意な対応:
//! 本番ビルド (release) → alias 名 `releash` / data dir `com.releash.app`
//! dev ビルド (debug)   → alias 名 `releash-dev` / data dir `com.releash.app.dev`

use std::path::{Path, PathBuf};

/// debug / release ビルド種別。
///
/// `cfg!(debug_assertions)` をテスト境界へ閉じ込めるための拡張点。
/// pure helper (`alias_name_for_profile` / `default_data_dir_name_for_profile`) に
/// 渡すことで、テストから dev / 本番 双方の分岐を同一バイナリで検証する。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildProfile {
    /// 本番ビルド (release)。
    Production,
    /// dev ビルド (debug)。
    Development,
}

impl BuildProfile {
    /// 現在の cargo ビルド種別から `BuildProfile` を導出する。
    pub fn current() -> Self {
        if cfg!(debug_assertions) {
            Self::Development
        } else {
            Self::Production
        }
    }
}

/// `BuildProfile` から CLI alias 名を決定する。
pub fn alias_name_for_profile(profile: BuildProfile) -> &'static str {
    match profile {
        BuildProfile::Production => "releash",
        BuildProfile::Development => "releash-dev",
    }
}

/// `BuildProfile` から既定の data dir 名（bundle identifier）を決定する。
pub fn default_data_dir_name_for_profile(profile: BuildProfile) -> &'static str {
    match profile {
        BuildProfile::Production => "com.releash.app",
        BuildProfile::Development => "com.releash.app.dev",
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
    /// 起動環境から `PathAliases` を構築する。
    ///
    /// `data_dir_override` を渡した場合、その値を `releash` alias の data_dir として使う。
    /// Tauri 側では `AppHandle::path().app_data_dir()` の解決結果を渡すことで、
    /// `tauri.conf.dev.json` の identifier (`com.releash.app.dev`) が反映された
    /// 実 data dir と一致させる。CLI 単体起動などで override が無い場合は
    /// `dirs::data_dir()` + bundle identifier 既定値から組み立てる。
    ///
    /// `data_dir_override` が `None` かつ `dirs::data_dir()` が解決失敗した場合は
    /// `Err` を返す。spec [01]「CLI alias と実行対象の一意な対応」境界を守るため、
    /// 失敗時は cwd へのサイレントフォールバックではなく明示エラー化する。
    pub fn from_runtime(data_dir_override: Option<PathBuf>) -> Result<Self, String> {
        let profile = BuildProfile::current();
        let exe_path = std::env::current_exe()
            .map_err(|e| format!("failed to resolve current executable path: {e}"))?;
        let name = alias_name_for_profile(profile);
        let data_dir = match data_dir_override {
            Some(d) => d,
            None => default_data_dir_for_profile(profile)?,
        };
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

    /// 公開対象の alias key 一覧（namespace `path_alias.<key>` の `<key>` 部分）。
    ///
    /// facet 展開エンジン側で「既知 alias key」を判定する際に使う。
    #[cfg(test)]
    pub fn known_keys() -> &'static [&'static str] {
        &["releash"]
    }
}

/// 起動環境別の既定 data dir。
///
/// 明示 override がない場合の解決ロジックを CLI / Tauri 両方で共有するための
/// 単一エントリ。`dirs::data_dir()` 解決失敗時は `.` フォールバックではなく
/// 明示エラーを返す（spec [01]「alias は alias 名・実行 binary・データディレクトリの
/// 三者を組として保持」境界が曖昧化するのを防ぐため）。
pub fn default_data_dir_for_profile(profile: BuildProfile) -> Result<PathBuf, String> {
    let base = dirs::data_dir()
        .ok_or_else(|| "failed to resolve OS data directory (dirs::data_dir)".to_string())?;
    Ok(base.join(default_data_dir_name_for_profile(profile)))
}

/// `resolve_session_data_dir_env` の判定結果。
///
/// spec issues-1054 「明示指定 > alias 内包値」原則を保ちつつ、別 Releash binary 由来の
/// inherit (例: prod Releash の Terminal Panel から起動した shell の env に prod
/// `RELEASH_DATA_DIR` が入っており、その shell から dev binary を起動した場合) を
/// 「ユーザー明示指定」と区別するための判定型。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedDataDirEnv {
    /// `RELEASH_DATA_DIR` を指定パスに設置する。
    Set(PathBuf),
    /// 親プロセスの値を尊重する (no-op)。
    Keep,
}

/// 既知 Releash alias の data_dir パス一覧 (両 `BuildProfile` 分)。
///
/// `resolve_session_data_dir_env` で「親プロセス env が別 Releash binary 由来の inherit
/// かどうか」を判定する材料。`dirs::data_dir()` 解決失敗時は `Err` を返し、呼び出し側が
/// 判定を諦めて安全側 (env 変更なし) にフォールバックできるようにする。
pub fn known_alias_data_dirs() -> Result<Vec<PathBuf>, String> {
    Ok(vec![
        default_data_dir_for_profile(BuildProfile::Production)?,
        default_data_dir_for_profile(BuildProfile::Development)?,
    ])
}

/// 親プロセス env の `RELEASH_DATA_DIR` をどう扱うかを決定する pure 関数。
///
/// spec issues-1054 Implementation Freedom L104「`RELEASH_DATA_DIR` の明示指定検出方法」に
/// 対する追加判定:
/// - `None` / 空文字 → `Set(self_data_dir)`: 親 env なし、自プロセスの alias data_dir を設置
/// - 既知 Releash alias data_dir のいずれかと一致 → `Set(self_data_dir)`: 別 Releash binary
///   由来の inherit と判定し、自プロセスの alias data_dir で上書き
/// - 上記いずれにも一致しない任意パス → `Keep`: ユーザーの真の明示指定として尊重
///
/// 「`RELEASH_DATA_DIR` の利用者明示指定は alias 内包値で上書きしない」(spec issues-1054
/// design.md L92) は維持しつつ、Releash app 自身が PTY 等で inject した同一 env が異種 binary
/// に伝搬する経路だけを正す。
pub fn resolve_session_data_dir_env(
    parent_env: Option<&str>,
    self_data_dir: &Path,
    known_alias_data_dirs: &[PathBuf],
) -> ResolvedDataDirEnv {
    match parent_env {
        None | Some("") => ResolvedDataDirEnv::Set(self_data_dir.to_path_buf()),
        Some(value) => {
            let parent_path = PathBuf::from(value);
            if known_alias_data_dirs.iter().any(|p| p == &parent_path) {
                ResolvedDataDirEnv::Set(self_data_dir.to_path_buf())
            } else {
                ResolvedDataDirEnv::Keep
            }
        }
    }
}

/// Tauri app 起動直後に呼び、自プロセスの `RELEASH_DATA_DIR` env を
/// startup boundaryで解決済みのalias data_dirで正す。
///
/// 別 Releash binary (例: prod 版 Releash の Terminal Panel) から inherit された env を
/// 「ユーザー明示指定」と誤認すると、子プロセス (agent / pty / oneshot) への伝搬時に
/// 異種 data_dir を渡してしまう。本関数は起動初期に env を判定 (`resolve_session_data_dir_env`)
/// して必要なら `std::env::set_var` で自プロセスの alias data_dir に揃える。
///
/// alias一覧の解決に失敗した場合はenvを変更せずlogのみに留める。
pub fn ensure_release_data_dir_env_for_resolved_path(self_data_dir: &Path) {
    let known = match known_alias_data_dirs() {
        Ok(v) => v,
        Err(e) => {
            log::warn!(
                "ensure_release_data_dir_env_for_resolved_path: failed to enumerate known alias data dirs: {e}"
            );
            return;
        }
    };
    let parent = std::env::var("RELEASH_DATA_DIR").ok();
    if let ResolvedDataDirEnv::Set(path) =
        resolve_session_data_dir_env(parent.as_deref(), self_data_dir, &known)
    {
        std::env::set_var("RELEASH_DATA_DIR", &path);
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
pub fn child_env_overrides(aliases: &PathAliases) -> Result<Vec<(String, String)>, String> {
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
) -> Result<Vec<(String, String)>, String> {
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
/// - `data_dir` が `None` の場合（Tauri 側 `app_data_dir()` 解決失敗時など）は空の env を
///   返し、呼び出し側が alias なしで spawn する既存挙動を温存する。
/// - `data_dir` が `Some` の場合に wrapper 作成等で失敗したら `Err` を返し、呼び出し側で
///   spawn を中止する。
pub fn prepare_child_env(data_dir: Option<PathBuf>) -> Result<Vec<(String, String)>, String> {
    let Some(data_dir) = data_dir else {
        return Ok(Vec::new());
    };
    let aliases = PathAliases::from_runtime(Some(data_dir))?;
    child_env_overrides(&aliases)
}

/// `<data_dir>/bin/<alias_name>` に CLI 実行 binary を指す wrapper を用意し、bin dir を返す。
///
/// wrapper はシェルスクリプトで、呼び出し側が `RELEASH_DATA_DIR` を明示指定していない
/// 場合のみ alias 内包の data_dir を設定する（spec 解決順序: 明示指定 > alias 内包値）。
fn ensure_alias_wrapper(releash: &PathAlias) -> Result<PathBuf, String> {
    let bin_dir = releash.data_dir.join("bin");
    std::fs::create_dir_all(&bin_dir)
        .map_err(|e| format!("failed to create alias bin dir {}: {e}", bin_dir.display()))?;
    let wrapper_path = bin_dir.join(&releash.name);
    let exe_str = releash
        .exe_path
        .to_str()
        .ok_or_else(|| "exe_path is not valid UTF-8".to_string())?;
    let data_dir_str = releash
        .data_dir
        .to_str()
        .ok_or_else(|| "data_dir is not valid UTF-8".to_string())?;
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
fn write_wrapper_script(path: &Path, script: &str) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, script)
        .map_err(|e| format!("failed to write wrapper {}: {e}", path.display()))?;
    let mut perms = std::fs::metadata(path)
        .map_err(|e| format!("failed to stat wrapper {}: {e}", path.display()))?
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms)
        .map_err(|e| format!("failed to chmod wrapper {}: {e}", path.display()))?;
    Ok(())
}

#[cfg(not(unix))]
fn write_wrapper_script(path: &Path, script: &str) -> Result<(), String> {
    std::fs::write(path, script)
        .map_err(|e| format!("failed to write wrapper {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alias_name_for_profile_returns_releash_for_production() {
        assert_eq!(alias_name_for_profile(BuildProfile::Production), "releash");
    }

    #[test]
    fn alias_name_for_profile_returns_releash_dev_for_development() {
        assert_eq!(
            alias_name_for_profile(BuildProfile::Development),
            "releash-dev"
        );
    }

    #[test]
    fn default_data_dir_name_distinguishes_dev_and_production() {
        assert_eq!(
            default_data_dir_name_for_profile(BuildProfile::Production),
            "com.releash.app"
        );
        assert_eq!(
            default_data_dir_name_for_profile(BuildProfile::Development),
            "com.releash.app.dev"
        );
    }

    #[test]
    fn from_runtime_uses_build_profile_for_alias_name() {
        let aliases = PathAliases::from_runtime(Some(PathBuf::from("/tmp/data"))).unwrap();
        let releash = aliases.releash();
        assert_eq!(
            releash.name,
            alias_name_for_profile(BuildProfile::current())
        );
        assert_eq!(releash.data_dir, PathBuf::from("/tmp/data"));
    }

    #[test]
    fn known_keys_contains_only_releash() {
        assert_eq!(PathAliases::known_keys(), &["releash"]);
    }

    #[test]
    fn from_runtime_uses_default_dir_when_no_override() {
        // dirs::data_dir() が解決できる環境では default 解決経路に乗る。
        if dirs::data_dir().is_none() {
            return;
        }
        let aliases = PathAliases::from_runtime(None).unwrap();
        let releash = aliases.releash();
        let expected_suffix = default_data_dir_name_for_profile(BuildProfile::current());
        assert!(
            releash.data_dir.ends_with(expected_suffix),
            "data_dir should end with {expected_suffix}: {}",
            releash.data_dir.display()
        );
    }

    #[test]
    fn compose_path_with_alias_bin_prepends_to_head_when_path_set() {
        // alias bin dir は既存 PATH の**先頭**にあること。末尾だと既存 PATH 前方に
        // `releash` / `releash-dev` を含む別ディレクトリがあると wrapper が解決されず
        // alias と実行 binary の一意対応 (spec [01]) が崩れる。bin dir に置く実体は
        // wrapper のみで、システムコマンドの shadow 経路は生じない。
        let bin_dir = PathBuf::from("/tmp/my-data/bin");
        let composed =
            compose_path_with_alias_bin(Some("/system-bin:/other-bin"), bin_dir.as_path());
        assert_eq!(composed, "/tmp/my-data/bin:/system-bin:/other-bin");
    }

    #[test]
    fn compose_path_with_alias_bin_falls_back_to_bin_only_when_path_unset() {
        let bin_dir = PathBuf::from("/tmp/my-data/bin");
        assert_eq!(
            compose_path_with_alias_bin(None, bin_dir.as_path()),
            "/tmp/my-data/bin"
        );
        assert_eq!(
            compose_path_with_alias_bin(Some(""), bin_dir.as_path()),
            "/tmp/my-data/bin"
        );
    }

    #[cfg(unix)]
    #[test]
    fn child_env_overrides_from_sets_releash_data_dir_when_parent_unset() {
        let tmp = tempfile::TempDir::new().unwrap();
        let exe = tmp.path().join("releash-bin");
        std::fs::write(&exe, "").unwrap();
        let aliases = PathAliases {
            releash: PathAlias {
                name: "releash-test".to_string(),
                exe_path: exe,
                data_dir: tmp.path().join("data"),
            },
        };
        let overrides = child_env_overrides_from(&aliases, Some("/bin"), None).unwrap();
        let data_dir_value = overrides
            .iter()
            .find_map(|(k, v)| (k == "RELEASH_DATA_DIR").then(|| v.clone()))
            .expect("RELEASH_DATA_DIR override missing");
        assert_eq!(
            data_dir_value,
            tmp.path().join("data").display().to_string()
        );
    }

    #[cfg(unix)]
    #[test]
    fn child_env_overrides_from_omits_releash_data_dir_when_parent_set() {
        // spec issues-1054 解決順序「明示指定 > alias 内包値」: 親プロセスに
        // RELEASH_DATA_DIR が明示されているときは alias 内包値で上書きしない。
        let tmp = tempfile::TempDir::new().unwrap();
        let exe = tmp.path().join("releash-bin");
        std::fs::write(&exe, "").unwrap();
        let aliases = PathAliases {
            releash: PathAlias {
                name: "releash-test".to_string(),
                exe_path: exe,
                data_dir: tmp.path().join("data"),
            },
        };
        let overrides =
            child_env_overrides_from(&aliases, Some("/bin"), Some("/explicit/path")).unwrap();
        assert!(
            overrides.iter().all(|(k, _)| k != "RELEASH_DATA_DIR"),
            "RELEASH_DATA_DIR must not be in overrides when parent set it: {overrides:?}"
        );
        // PATH は依然として alias bin を先頭に積む。
        let path_value = overrides
            .iter()
            .find_map(|(k, v)| (k == "PATH").then(|| v.clone()))
            .expect("PATH override missing");
        assert!(path_value.starts_with(tmp.path().join("data").join("bin").to_str().unwrap()));
        assert!(path_value.ends_with(":/bin"));
    }

    #[cfg(unix)]
    #[test]
    fn child_env_overrides_from_treats_empty_parent_data_dir_as_unset() {
        let tmp = tempfile::TempDir::new().unwrap();
        let exe = tmp.path().join("releash-bin");
        std::fs::write(&exe, "").unwrap();
        let aliases = PathAliases {
            releash: PathAlias {
                name: "releash-test".to_string(),
                exe_path: exe,
                data_dir: tmp.path().join("data"),
            },
        };
        let overrides = child_env_overrides_from(&aliases, None, Some("")).unwrap();
        assert!(
            overrides.iter().any(|(k, _)| k == "RELEASH_DATA_DIR"),
            "empty parent RELEASH_DATA_DIR should be treated as unset: {overrides:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn prepare_child_env_returns_empty_when_data_dir_none() {
        // app_data_dir() 解決失敗時の経路: 既存挙動 (silent skip) を温存。
        let env = prepare_child_env(None).unwrap();
        assert!(
            env.is_empty(),
            "no overrides expected when data_dir is None"
        );
    }

    #[cfg(unix)]
    #[test]
    fn prepare_child_env_returns_path_and_data_dir_when_data_dir_provided() {
        // spec issues-1054「agent 子プロセスへの実行環境の伝搬」:
        // PTY / agent bridge が共有する env builder は alias bin の PATH と
        // RELEASH_DATA_DIR の両方を出力する。
        let tmp = tempfile::TempDir::new().unwrap();
        let env = prepare_child_env(Some(tmp.path().join("data"))).unwrap();
        assert!(env.iter().any(|(k, _)| k == "PATH"));
        // 親プロセスに RELEASH_DATA_DIR が無いテスト前提でのみ data_dir が積まれる。
        if std::env::var("RELEASH_DATA_DIR").is_err() {
            assert!(env.iter().any(|(k, _)| k == "RELEASH_DATA_DIR"));
        }
    }

    #[cfg(unix)]
    #[test]
    fn prepare_child_env_propagates_wrapper_failure() {
        // wrapper 作成不能（既存ファイルが bin dir 位置を占有）→ Err を返し、
        // 呼び出し側（PTY / bridge）で spawn を中止する。
        let tmp = tempfile::TempDir::new().unwrap();
        let data_dir = tmp.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        // bin に通常ファイルを置くと create_dir_all が失敗する。
        std::fs::write(data_dir.join("bin"), "").unwrap();
        let err = prepare_child_env(Some(data_dir)).unwrap_err();
        assert!(
            err.contains("alias bin dir"),
            "expected wrapper bin dir error, got: {err}"
        );
    }

    #[test]
    fn default_data_dir_for_profile_returns_path_when_dirs_available() {
        // dirs::data_dir() が解決できる環境では Ok を返し、bundle identifier suffix を持つ。
        if dirs::data_dir().is_none() {
            return;
        }
        let path = default_data_dir_for_profile(BuildProfile::Production).unwrap();
        assert!(path.ends_with("com.releash.app"));
    }

    #[cfg(unix)]
    #[test]
    fn ensure_alias_wrapper_exports_data_dir_only_when_unset() {
        let tmp = tempfile::TempDir::new().unwrap();
        let exe = tmp.path().join("releash-bin");
        std::fs::write(&exe, "").unwrap();
        let alias = PathAlias {
            name: "releash-test".to_string(),
            exe_path: exe,
            data_dir: tmp.path().join("data"),
        };
        let bin_dir = ensure_alias_wrapper(&alias).unwrap();
        let wrapper = bin_dir.join("releash-test");
        let script = std::fs::read_to_string(&wrapper).unwrap();
        // wrapper は `RELEASH_DATA_DIR` 未設定時のみ alias 内包値を export する
        // （spec 解決順序: 明示指定 > alias 内包値）。
        assert!(
            script.contains(r#"if [ -z "$RELEASH_DATA_DIR" ]; then"#),
            "wrapper must guard RELEASH_DATA_DIR export: {script}"
        );
        assert!(
            script.contains("export RELEASH_DATA_DIR="),
            "wrapper must export alias data_dir when unset: {script}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn ensure_alias_wrapper_creates_executable() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::TempDir::new().unwrap();
        let exe = tmp.path().join("releash-bin");
        std::fs::write(&exe, "").unwrap();
        let alias = PathAlias {
            name: "releash-test".to_string(),
            exe_path: exe,
            data_dir: tmp.path().join("data"),
        };
        let bin_dir = ensure_alias_wrapper(&alias).unwrap();
        let wrapper = bin_dir.join("releash-test");
        let mode = std::fs::metadata(&wrapper).unwrap().permissions().mode();
        assert_eq!(mode & 0o111, 0o111, "wrapper should be executable");
    }

    // ----- resolve_session_data_dir_env -----

    /// 親 env が None なら自分の data_dir を設置する。
    #[test]
    fn resolve_session_data_dir_env_sets_self_when_parent_env_absent() {
        let self_dir = PathBuf::from("/home/u/.local/share/com.releash.app.dev");
        let known = vec![
            PathBuf::from("/home/u/.local/share/com.releash.app"),
            self_dir.clone(),
        ];
        let result = resolve_session_data_dir_env(None, &self_dir, &known);
        assert_eq!(result, ResolvedDataDirEnv::Set(self_dir));
    }

    /// 親 env が空文字なら自分の data_dir を設置する。
    #[test]
    fn resolve_session_data_dir_env_sets_self_when_parent_env_empty() {
        let self_dir = PathBuf::from("/home/u/.local/share/com.releash.app.dev");
        let known = vec![
            PathBuf::from("/home/u/.local/share/com.releash.app"),
            self_dir.clone(),
        ];
        let result = resolve_session_data_dir_env(Some(""), &self_dir, &known);
        assert_eq!(result, ResolvedDataDirEnv::Set(self_dir));
    }

    /// 親 env が「別 alias の data_dir」を指している場合は、別 Releash binary 由来の
    /// inherit と判定して自分の alias data_dir で上書きする (バグ修正の主シナリオ)。
    #[test]
    fn resolve_session_data_dir_env_overrides_when_parent_matches_other_alias() {
        let self_dir = PathBuf::from("/home/u/.local/share/com.releash.app.dev");
        let other_alias = PathBuf::from("/home/u/.local/share/com.releash.app");
        let known = vec![other_alias.clone(), self_dir.clone()];
        let result =
            resolve_session_data_dir_env(Some(other_alias.to_str().unwrap()), &self_dir, &known);
        assert_eq!(result, ResolvedDataDirEnv::Set(self_dir));
    }

    /// 親 env が「自分と同じ alias の data_dir」を指している場合も Set(self) を返す
    /// (同種 inherit 経路で値が一致しているケースの整合性確認、結果は no-op 同等)。
    #[test]
    fn resolve_session_data_dir_env_overrides_when_parent_matches_own_alias() {
        let self_dir = PathBuf::from("/home/u/.local/share/com.releash.app.dev");
        let known = vec![
            PathBuf::from("/home/u/.local/share/com.releash.app"),
            self_dir.clone(),
        ];
        let result =
            resolve_session_data_dir_env(Some(self_dir.to_str().unwrap()), &self_dir, &known);
        assert_eq!(result, ResolvedDataDirEnv::Set(self_dir));
    }

    /// 親 env が「既知 alias data_dir のいずれにも一致しない任意パス」を指している場合は
    /// ユーザーの真の明示指定として尊重 (Keep) する。
    #[test]
    fn resolve_session_data_dir_env_keeps_parent_when_arbitrary_path() {
        let self_dir = PathBuf::from("/home/u/.local/share/com.releash.app.dev");
        let known = vec![
            PathBuf::from("/home/u/.local/share/com.releash.app"),
            self_dir.clone(),
        ];
        let result = resolve_session_data_dir_env(Some("/tmp/custom-releash"), &self_dir, &known);
        assert_eq!(result, ResolvedDataDirEnv::Keep);
    }
}
