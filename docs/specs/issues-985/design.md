# Design — issues-985

Issue: #985 「`git_host`（Git hosting integration）を clean architecture 配置へ移行する」

本書は `requirements.md`（R1〜R9）と `behavior.md`（移行後も観測可能でなければならない振る舞い）を前提に、`git_host` flat module を 4 レイヤー（`domain` / `usecase` / `adaptor/gateway` / `adaptor/controller`）へ分割移行する実装方針を定める。external observable behavior は不変（R8）であり、本書が扱うのは内部構造の再配置（型・関数の配置先、port、DI 配線、テストの移送）である。

設計は、既に移行済みの `repository` ドメイン（`domain/repository/` + `usecase/repository_*` + `adaptor/gateway/repository/` + `adaptor/controller/command/repository/` + `adaptor/controller/wiring.rs` + `AppState`）が確立した慣習に揃える（requirements A3）。

---

## 1. 概要

- `git_host` の責務を「value object（domain）」「port trait（domain）」「業務手順 / cache orchestration（usecase）」「gh 実行・出力 parse・git remote 判定・in-memory cache（gateway）」「Tauri command wrapper（controller）」へ分離する。
- usecase は domain port（`GitHostProvider` / `PrStatusCache` / `IssueCache`）にのみ依存し、`gh` 実行・git2・Mutex・Tauri を知らない。
- GitHub の具象（`gh` CLI 実行と JSON parse）と provider discovery（git2 origin URL 判定）と in-memory cache（`Mutex<HashMap>`）は gateway 具象に閉じる。
- cache TTL（30 秒）は domain value object `CacheTtl` として表現し、hit / miss / stale 判定ルール（`elapsed < TTL`）を domain に持たせる。in-memory 保持と lookup-or-fetch-store の組み立てはそれぞれ gateway / usecase が担う。
- 5 本の Tauri command は `adaptor/controller/command/git_host/` の薄い wrapper として再配置し、command 名・引数・成功 DTO 形状・cache TTL 挙動を不変に保つ。
- R9 の dead-code（review comment 関連型・`get_pr_review_comments`・`parse_pr_review_comments`・時刻変換ヘルパ）は新レイヤーへ移送せず削除する。
- 旧 `src-tauri/src/git_host/` ディレクトリは削除し、`crate::git_host::*` 参照を 0 にする。

## 2. 変更対象

### 2.1 新規追加

| レイヤー | パス | 内容 |
|---|---|---|
| domain | `domain/git_host/mod.rs` | 再エクスポート |
| domain | `domain/git_host/git_host.rs` | port trait（`GitHostProvider` / `PrStatusCache` / `IssueCache`）。`repository.rs` 慣習に倣い `module_inception` を許容 |
| domain | `domain/git_host/value_objects/mod.rs` | value object 再エクスポート |
| domain | `domain/git_host/value_objects/pr.rs` | `PrInfo` / `PrStatus` |
| domain | `domain/git_host/value_objects/provider_status.rs` | `ProviderStatus` |
| domain | `domain/git_host/value_objects/issue.rs` | `IssueInfo` / `IssueLabel` / `Milestone` / `PrAuthor` |
| domain | `domain/git_host/value_objects/cache.rs` | `CacheTtl`（TTL と `is_fresh` 判定ルール） |
| usecase | `usecase/git_host/mod.rs` | 再エクスポート |
| usecase | `usecase/git_host/git_host_usecase.rs` | `GitHostUsecase`（provider detection / fetch / cache orchestration）+ test |
| usecase | `usecase/git_host/dto.rs` | Tauri response DTO（`PrStatusDto` / `IssueInfoDto` / `ProviderStatusDto` など）と domain → DTO 変換 |
| gateway | `adaptor/gateway/git_host/mod.rs` | module 宣言 + 再エクスポート |
| gateway | `adaptor/gateway/git_host/github.rs` | `GitHubGitHostGateway`（`GitHostProvider` 実装）・`gh` 実行・gh JSON service model（`GhIssueInfo` など）・JSON parse + parse test |
| gateway | `adaptor/gateway/git_host/discovery.rs` | git2 origin URL lookup・GitHub 判定 + discovery test |
| gateway | `adaptor/gateway/git_host/cache.rs` | `InMemoryTtlCache<T>`（`PrStatusCache` / `IssueCache` 実装） |
| controller | `adaptor/controller/command/git_host/mod.rs` | command module 宣言 |
| controller | `adaptor/controller/command/git_host/pr.rs` | `check_pr_provider_status` / `fetch_pr_status` / `get_cached_pr_status` |
| controller | `adaptor/controller/command/git_host/issue.rs` | `fetch_issues` / `get_cached_issues` |

> command のファイル分割（`pr.rs` / `issue.rs`）は責務の凝集に基づく提案（仮定 D1）。1 ファイルでも可。

### 2.2 既存ファイルの変更

| パス | 変更 |
|---|---|
| `domain/mod.rs` | `pub(crate) mod git_host;` を追加 |
| `usecase/mod.rs` | `pub(crate) mod git_host;` を追加 |
| `adaptor/gateway/mod.rs` | `pub(crate) mod git_host;` を追加 |
| `adaptor/controller/command/mod.rs` | `git_host` module 宣言追加。`generate_handler!` の登録を `crate::git_host::*` から `crate::adaptor::controller::command::git_host::*` へ差し替え（R5） |
| `adaptor/controller/state.rs` | `AppState` に `pub git_host_usecase: Arc<GitHostUsecase>` を追加 |
| `adaptor/controller/wiring.rs` | `build_git_host_usecase()` を追加（gateway + cache を合成して `GitHostUsecase` を返す composition root） |
| `lib.rs` | `mod git_host;` 削除。`.manage(Arc::new(git_host::PrCache::new()))` / `IssueCache` の 2 行削除。`AppState` 構築に `git_host_usecase` を追加（R5） |

### 2.3 削除

- `src-tauri/src/git_host/`（`mod.rs` / `github.rs` / `types.rs`）ディレクトリ全体（R6）。
- 移送しない dead-code（R9、新レイヤーへ移さない）:
  - `PrReviewComment` / `PrReviewCommentAuthor`（types.rs）。
  - `GitHostProvider::get_pr_review_comments`（新 port trait に含めない）。
  - `github.rs` の `parse_pr_review_comments` と `#[cfg(test)]` test（`parse_review_comments_*`）。
  - `mod.rs` の `parse_rfc3339_to_millis` / `days_from_civil` と `#[cfg(test)]` test。

## 3. アーキテクチャと責務分割

依存方向は `infrastructure → adaptor/gateway → domain ← usecase ← adaptor/controller` を遵守する。

```
controller/command/git_host  ── invoke ──▶  usecase::git_host::GitHostUsecase
                                                │ depends only on domain ports
                                                ▼
                          domain::git_host::{GitHostProvider, PrStatusCache, IssueCache}
                                                ▲ implemented by
                                                │
        adaptor/gateway/git_host::{GitHubGitHostGateway, InMemoryTtlCache<PrStatus>, InMemoryTtlCache<Vec<IssueInfo>>}
              (gh 実行 / JSON parse / git2 discovery / Mutex<HashMap> cache)
```

### 3.1 domain — value object と port

- value object: `ProviderStatus` / `PrInfo` / `PrStatus` / `IssueInfo` / `IssueLabel` / `Milestone` / `PrAuthor`。domain は `docs/architecture/DOMAIN.md` に従い serde 非依存とし、転送形式の derive / 属性を持たせない。型名は JSON 表現に影響しないため現名を維持し churn を抑える。
- 外部表現は境界側に分離する。gh JSON の入力 service model（`GhIssueInfo` / `GhIssueLabel` / `GhPrAuthor` / `GhMilestone`）は gateway が持ち、`Deserialize` と `alias` / `default` をそこで扱う。Tauri response DTO（`PrStatusDto` / `IssueInfoDto` / `ProviderStatusDto` など）は usecase 側の `dto.rs` が持ち、`Serialize` / `Deserialize` と `rename_all` をそこで扱う。controller は usecase が返す domain value object を DTO へ変換して返す（DTO 形状不変 = R8）。
- cache policy value object `CacheTtl`: TTL 値（30 秒）と freshness 判定ルールを保持する。これが domain decision（R3）。
- port trait（`git_host.rs`）:
  - `GitHostProvider` — 外部 host アクセス port。`gh`・git2 を知らない。
  - `PrStatusCache` / `IssueCache` — cache の lookup / store port。
- domain は infrastructure 依存を持たない。`std::time::{Duration, Instant}` のみ使用（external client ではないため domain 内で許容）。
- **error 型は持たない**: 現行の観測挙動は「`gh` 失敗・タイムアウト・parse 不能でも空結果 + `Ok`」（behavior A4）であり、port メソッドは失敗を値（空 map / 空 vec / `ProviderStatus` enum）に畳み込む infallible シグネチャとする。error を domain port に持ち込むと現行の fallback 挙動とずれるため導入しない。

### 3.2 usecase — `GitHostUsecase`

- domain port のみに依存し、provider detection / fetch / cache orchestration を担う。
- 5 つの application 操作を提供する（いずれも infallible、戻り値は domain value object）:
  - `check_provider_status(repo_path) -> ProviderStatus`
  - `fetch_pr_status(repo_path) -> PrStatus`（cache 不使用）
  - `get_cached_pr_status(repo_path) -> PrStatus`（cache orchestration）
  - `fetch_issues(repo_path) -> Vec<IssueInfo>`（cache 不使用）
  - `get_cached_issues(repo_path) -> Vec<IssueInfo>`（cache orchestration）
- cache orchestration（lookup-or-fetch-store）は application behavior としてここに置く（R3）。

### 3.3 gateway — GitHub 具象・discovery・in-memory cache

- `GitHubGitHostGateway`（`GitHostProvider` 実装）:
  - `provider_status`: discovery（origin URL）→ 非 GitHub なら `UnsupportedPlatform` / remote 無しなら `NoRemote`、GitHub なら `gh --version` → `gh auth status` で `CliNotFound` / `NotAuthenticated` / `Available` を判定。**現 `check_provider_status` / `check_github_status` のロジックをそのまま移送**。
  - `fetch_pr_status`: discovery → GitHub なら `gh pr list --state open/merged` を実行・parse して `PrStatus { open_prs, merged_branches }` を構成、非 GitHub なら `PrStatus::default()`。**現 `create_provider` + `fetch_pr_status_inner` + `GitHubProvider::detect_open_prs/detect_merged_prs` の合成を移送**。open/merged の合成（現 `fetch_pr_status_inner`）は host の PR ビュー構成として gateway に置く。
  - `list_issues`: discovery → GitHub なら `gh issue list` を実行・parse、非 GitHub なら空 vec。
  - 内部に `gh` 実行（`run_gh_with_timeout`、10 秒 timeout・別スレッド stdout 読み取り）と parse（`parse_gh_pr_list_output` / `parse_gh_merged_pr_output` / `parse_gh_issue_list_output`）を保持。`gh` 引数（`--json` フィールド・`--limit 100` 等）は不変（R8）。
- `discovery.rs`: `get_origin_url`（git2 `Repository::open` → `find_remote("origin")` → URL）と `is_github(url)`（`url.contains("github.com")`）。現ロジックをそのまま移送（R2: provider discovery は gateway 具象）。
- `InMemoryTtlCache<T>`（`PrStatusCache` / `IssueCache` 実装）:
  - `Mutex<HashMap<String, Entry<T>>>` を保持。`Entry { value: T, fetched_at: Instant }`。
  - 構築時に `CacheTtl` を受け取る。`lookup(repo_path)` は entry が存在し `ttl.is_fresh(fetched_at, Instant::now())` のとき `Some(value.clone())`、さもなくば `None`。`store(repo_path, value)` は stale entry を `retain` で除去してから insert（現 `*_with_cache` の `retain` 挙動を移送）。
  - 1 つの generic 実装を `PrStatus` 用と `Vec<IssueInfo>` 用にインスタンス化する（仮定 D2）。

### 3.4 controller — Tauri command wrapper

- 5 本の command を `State<AppState>` 経由で `state.git_host_usecase` を取得し、`run_blocking`（既存 `command/mod.rs` の共通ヘルパ）で usecase を `spawn_blocking` 上に載せて呼ぶ。
- command は引数（`repo_path: String`）と戻り値の型変換のみ。business behavior は持たせない（R4）。
- 戻り値型は `Result<T, AppError>`。`AppError` の serialize 表現は bare JSON 文字列であり（`repository` migration で実証済み）、現行 `Result<T, String>` の `Err`（join error 文字列）と wire 形状が等価。成功時の wire DTO（`PrStatusDto` / `Vec<IssueInfoDto>` / `ProviderStatusDto`）は usecase 側で定義し、既存 DTO 形状を不変に保つ。
- usecase メソッドは infallible のため、`run_blocking` の closure は DTO 変換済みの値を返す（唯一の error 経路は join error）。

## 4. データモデルまたは型

### 4.1 value object と転送 model の境界

domain value object は転送形式から独立させ、serde に依存しない。

```rust
// value_objects/pr.rs
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrInfo { pub number: u64, pub url: String }

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrStatus {
    pub open_prs: HashMap<String, PrInfo>,
    pub merged_branches: Vec<String>,
}

// value_objects/provider_status.rs
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderStatus {
    Available,
    CliNotFound { cli: String },
    NotAuthenticated,
    UnsupportedPlatform,
    NoRemote,
}

// value_objects/issue.rs — IssueInfo / IssueLabel / Milestone / PrAuthor
// domain 型は serde derive / serde 属性を持たない。
```

gh JSON の parse では gateway 内部の service model が外部入力形式を吸収する。

```rust
// adaptor/gateway/git_host/github.rs
#[derive(Debug, Deserialize)]
struct GhIssueInfo {
    number: u64,
    title: String,
    state: String,
    url: String,
    author: GhPrAuthor,
    #[serde(alias = "createdAt")]
    created_at: String,
    #[serde(alias = "updatedAt")]
    updated_at: String,
    #[serde(default)]
    labels: Vec<GhIssueLabel>,
    #[serde(default)]
    assignees: Vec<GhPrAuthor>,
    #[serde(default)]
    body: String,
    #[serde(default)]
    milestone: Option<GhMilestone>,
}

impl From<GhIssueInfo> for IssueInfo { /* field mapping */ }
```

Tauri response の wire shape は usecase 側 DTO が保持する。

```rust
// usecase/git_host/dto.rs
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrStatusDto {
    pub open_prs: HashMap<String, PrInfoDto>,
    pub merged_branches: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderStatusDto { /* existing wire variants */ }

impl From<PrStatus> for PrStatusDto { /* domain -> wire mapping */ }
impl From<ProviderStatus> for ProviderStatusDto { /* domain -> wire mapping */ }
```

### 4.2 cache policy value object（domain）

```rust
// value_objects/cache.rs
#[derive(Debug, Clone, Copy)]
pub struct CacheTtl(Duration);

impl CacheTtl {
    pub const fn from_secs(secs: u64) -> Self { Self(Duration::from_secs(secs)) }
    /// hit / stale 判定ルール: 経過時間が TTL 未満なら fresh（behavior A3）。
    pub fn is_fresh(&self, fetched_at: Instant, now: Instant) -> bool {
        now.duration_since(fetched_at) < self.0
    }
}
```

`PR` / `ISSUE` ともに `CacheTtl::from_secs(30)`（現 `PR_CACHE_TTL` / `ISSUE_CACHE_TTL` 相当）。

### 4.3 port trait（domain）

```rust
// git_host.rs
pub trait GitHostProvider: Send + Sync {
    fn provider_status(&self, repo_path: &str) -> ProviderStatus;
    fn fetch_pr_status(&self, repo_path: &str) -> PrStatus;
    fn list_issues(&self, repo_path: &str) -> Vec<IssueInfo>;
}

pub trait PrStatusCache: Send + Sync {
    fn lookup(&self, repo_path: &str) -> Option<PrStatus>;
    fn store(&self, repo_path: &str, value: PrStatus);
}

pub trait IssueCache: Send + Sync {
    fn lookup(&self, repo_path: &str) -> Option<Vec<IssueInfo>>;
    fn store(&self, repo_path: &str, value: Vec<IssueInfo>);
}
```

> 旧 `GitHostProvider::get_pr_review_comments` は port に含めない（R9）。
> port は `detect_open_prs` / `detect_merged_prs` を個別に持たず `fetch_pr_status` に集約する（現 `fetch_pr_status_inner` 同様、origin discovery を 1 回に保つため）。

### 4.4 usecase 構造

```rust
pub struct GitHostUsecase {
    provider: Arc<dyn GitHostProvider>,
    pr_cache: Arc<dyn PrStatusCache>,
    issue_cache: Arc<dyn IssueCache>,
}
```

## 5. 処理フロー

### 5.1 `check_pr_provider_status`

```
command → run_blocking → uc.check_provider_status(repo)
  → provider.provider_status(repo)
      → discovery: origin URL 取得
          ├ remote 無し       → NoRemote
          ├ 非 github.com    → UnsupportedPlatform
          └ github.com       → gh --version → 無ければ CliNotFound{cli:"gh"}
                               → gh auth status → 失敗で NotAuthenticated / 成功で Available
```

### 5.2 `fetch_pr_status` / `fetch_issues`（cache 不使用）

```
command → run_blocking → uc.fetch_pr_status(repo) → provider.fetch_pr_status(repo)
  → discovery: 非 GitHub → PrStatus::default()
  → GitHub → gh pr list --state open  → parse → open_prs
            gh pr list --state merged → parse → merged_branches
  （gh 失敗 / timeout / parse 不能は空に畳み込み、Ok を返す: A4）
```

`fetch_issues` も同様（`gh issue list` → parse、非 GitHub / 失敗時は空 vec）。

### 5.3 `get_cached_pr_status` / `get_cached_issues`（cache orchestration）

```
command → run_blocking → uc.get_cached_pr_status(repo)
  → pr_cache.lookup(repo)
      ├ Some(v)  (entry 存在 & fresh)  → v を返す（provider を呼ばない = cache hit）
      └ None     (entry 無し or stale) → v = provider.fetch_pr_status(repo)
                                         pr_cache.store(repo, v.clone())
                                         v を返す
```

- TTL 判定（`is_fresh`）は `InMemoryTtlCache.lookup` 内で `CacheTtl` を用いて行う。
- `store` は現 `*_with_cache` と同じく stale entry の `retain` 除去後に insert。
- 「30 秒未満 → hit」「30 秒以上 → stale → 再 fetch + 更新」（behavior A3）を維持。

## 6. エラー処理

- **port は infallible**。`gh` の spawn 失敗・非ゼロ終了・timeout・UTF-8 不正・JSON parse 不能はすべて gateway 内で空結果（空 map / 空 vec / 既定 `PrStatus`）へ畳み込む。現行 `run_gh_with_timeout` が `None` を返し呼び出し側で空にする挙動・`parse_*` の `unwrap_or_default` を維持（behavior A4）。`eprintln!` による診断ログ出力も現状維持。
- **command の error は join error のみ**。`spawn_blocking` の `JoinError` を `AppError`（メッセージ `"task join error: {e}"`）に変換する。`AppError` は bare JSON 文字列として serialize され、現 `Result<_, String>` の error wire 形状と等価。
- `provider_status` はエラーを返さず `ProviderStatus` enum の variant（`NoRemote` / `UnsupportedPlatform` / `CliNotFound` / `NotAuthenticated` / `Available`）で結果を表現する（現状維持）。

## 7. テスト方針

`docs/architecture/TEST.md` と既存慣習に従い、各 module 内 `#[cfg(test)] mod tests` に配置する。

### 7.1 usecase test（`usecase/git_host/git_host_usecase.rs`）— R7

fake `GitHostProvider` / fake `PrStatusCache` / `IssueCache`（呼び出し回数と返り値を制御）を用いる。

- provider 不在: fake provider が `default` / 空を返す → `fetch_pr_status` が空、`fetch_issues` が空、`Ok` 相当（値が空）。
- GitHub available: fake provider が `Available` → `check_provider_status` が `Available`。
- GitHub unavailable: fake provider が `NoRemote` / `UnsupportedPlatform` / `CliNotFound` / `NotAuthenticated` を返す各ケース。
- cache hit: fake cache `lookup` が `Some` → provider が**呼ばれない**ことを検証（呼び出しカウンタ）し、cache 値が返る。
- cache miss: fake cache `lookup` が `None` → provider が呼ばれ、`store` が呼ばれ、fetch 値が返る。
- cache stale: usecase からは miss と同経路（`lookup` が `None`）として観測。TTL 境界そのものは 7.3 でカバーし、stale → 再 fetch の経路を miss test が担保する。

### 7.2 gateway test — R7

- parse test（`github.rs`、現 `github.rs` のテストを移送）:
  - `parse_gh_pr_list_output`: valid / empty array / invalid JSON / missing fields。
  - `parse_gh_merged_pr_output`: valid / empty / invalid。
  - `parse_gh_issue_list_output`: valid / empty / invalid JSON / missing optional fields / 実 `gh` 出力（milestone 有無）。
  - パイプバッファ deadlock 再現テスト（`piped_stdout_*`）は `run_gh_with_timeout` の並行読み取り設計を守るため移送する（移送先は `github.rs`）。
  - `parse_pr_review_comments` 系テストは移送しない（R9）。
- discovery test（`discovery.rs`、現 `mod.rs` のテストを移送）:
  - `get_origin_url`: remote 無し → `None` / GitHub remote → URL。
  - `is_github` 経由の `provider_status` 相当: no remote → `NoRemote` / 非 GitHub → `UnsupportedPlatform`（git2 temp repo を用いる現 `check_provider_status_*` を移送）。
- cache gateway test（`cache.rs`、任意）: `store` → `lookup` で `Some`、別 key は `None`。

### 7.3 domain test（`value_objects/cache.rs`）— TTL 境界

- `CacheTtl::is_fresh`: `fetched_at` に対し `now = fetched_at + 29s` で `true`、`+30s` / `+31s` で `false`（30 秒未満は hit、30 秒以上は stale: behavior A3）。`Instant::now()` を基準に `checked_add` で `now` を構成し決定的に検証する。

### 7.4 品質ゲート

`cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test` が通ること（受け入れ基準）。frontend は無変更のため `pnpm test` への影響なし。

## 8. リスクと代替案

- **command の error 型変更（`String` → `AppError`）**: 成功 DTO は不変、error は両者とも bare JSON 文字列で wire 等価のため観測不変（R8）。リスクは低いが、万一懸念があれば現行どおり `Result<_, String>` を維持し join error 文字列をそのまま返す代替も可能。本設計は `repository` 慣習（`run_blocking` + `AppError`）に揃える方を採る。
- **provider discovery の回数**: coarse port（`fetch_pr_status` に open/merged 合成を集約）により discovery は呼び出しあたり 1 回で、現 `fetch_pr_status_inner`（`create_provider` 1 回）と等価。代替の細粒度 port（`detect_open_prs` / `detect_merged_prs` を usecase が合成）は discovery が増えるため採らない。
- **cache を `AppState` に統合 vs 単体 `manage`**: 本設計は `git_host_usecase` を `AppState` に載せ、cache 具象は usecase が `Arc` 保持する（`repository` 慣習と一致、`State<AppState>` 単一注入）。代替は現状どおり cache を個別 `manage` し command が個別 `State` で受ける構成だが、新レイヤーの DI 一貫性のため `AppState` 統合を採る。いずれも frontend invoke 署名（`repo_path` のみ）は不変。
- **`gh` 実行を gateway に置く（infrastructure ではなく）**: requirements が「`gh` CLI プロセス実行」を `adaptor/gateway/git_host/` に明示指定（スコープ）しているため、process 実行を infrastructure へ切り出さず gateway に置く。
- **テストの time 依存**: TTL 境界を usecase 統合テストで実時間待ちすると flaky になるため、判定ロジックを `CacheTtl::is_fresh(fetched_at, now)`（pure）へ切り出し、`now` を `checked_add` で構成して決定的に検証する。usecase 層は fake cache の `Some`/`None` で hit/miss を制御する。

## 9. 仮定

requirements A1〜A4 / behavior A1〜A6 を所与とする。加えて本設計で置いた仮定:

- **D1**: command のファイル分割（`pr.rs` / `issue.rs`）は責務凝集に基づく提案。1 ファイル化も許容（観測挙動に無影響）。
- **D2**: in-memory cache は generic `InMemoryTtlCache<T>` を `PrStatus` / `Vec<IssueInfo>` の 2 インスタンスで使う。型ごとに別 struct を置く構成でも可。
- **D3**: TTL 判定ルール（`elapsed < TTL`）を domain value object `CacheTtl::is_fresh` に置き、in-memory 保持を gateway、lookup-or-fetch-store を usecase に置く（R3 の「domain decision なら value object 化」を採用）。
- **D4**: command は `Result<_, AppError>` を返し `run_blocking` を再利用する（`repository` 慣習）。`AppError` の serialize は bare 文字列で現 `String` error と wire 等価。
- **D5**: `git_host_usecase: Arc<GitHostUsecase>` を `AppState` に追加し、`lib.rs` の個別 `manage(PrCache/IssueCache)` を廃止する。cache 具象は `GitHostUsecase` が `Arc` 保持して生存させる。
- **D6**: value object の型名（`PrInfo` / `PrAuthor` 等）は現名を維持する。型名は JSON 表現に影響せず、移行差分を最小化する。domain value object は serde 非依存にし、転送形式の属性は境界側へ厳密に移送する。`alias` / `default` は gh JSON service model（gateway）に置き、`rename_all` と Tauri response の serialize は usecase DTO に置く。
- **D7**: port trait の置き場所は `domain/git_host/git_host.rs`（`repository.rs` 同様 `module_inception` を許容）。

## 10. Open Questions

なし。requirements / behavior の Open Questions が「なし」であり、本設計の判断点（command error 型・cache の AppState 統合・ファイル分割・port 粒度・TTL value object 化）はいずれも既存 `repository` 移行の慣習と R1〜R9 の制約から確定でき、external observable behavior（R8）に影響しないため、人間確認を要する未確定事項はない。
