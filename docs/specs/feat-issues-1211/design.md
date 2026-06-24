# Design

Issue: #1211 「ReviewSnapshot / ReviewFileView command を追加し diff 表示を Rust read model に寄せる」

本書は requirements.md / behavior.md を実装方針へ落とし込む設計文書である。語彙は requirements に従う:
`DiffBase` = `branch-base` | `head`、`DiffSection` = `changes` | `staged`。

関連先行実装（#1210, commit `46dd0962`）の調査結果を前提に、既存の `RepositoryStateService`（versioned snapshot）
と既存 review command（`get_review_text_diff` / `get_review_image_diff` 等）を土台として設計する。

---

## 1. 概要

レビュー（diff 表示）の read model 生成と file IO を Rust 側へ集約する。具体的には次の 2 つの Tauri コマンドを追加する。

- **ReviewSnapshot**: `get_review_snapshot(worktree_path, base)` — ファイル一覧 read model を 1 回で返す（R1, R3）。
- **ReviewFileView**: `get_review_file_view(target, section, base, viewport?)` — ファイル表示種別を Rust 側で判定し、種別別 read model を返す（R2, R5, R6）。

これに伴い、frontend の direct FS read / Git orchestration（diff 基準選択・tree 化・base64→data URL 変換・Git index path 組み立て）を撤去し、frontend は read model の表示に徹する（R4）。

既存資産の活用方針:
- ファイル一覧 read model は既存 `RepositorySnapshot`（`status` / `diff_stats` / `*_diff_file_tree` / `version` / `flags`）をほぼそのまま再利用する。
- text diff の original / modified 生成は既存 `get_review_text_diff` の生成ロジック（`code/file_content.rs`）を ReviewFileView の内部実装として再利用する。
- image / binary は既存 base64 経路（`get_review_image_diff`）を廃し、Tauri asset / custom URI scheme による URL 参照へ置き換える。

---

## 2. 変更対象

### 2.1 Rust（追加）

| 区分 | パス | 内容 |
| --- | --- | --- |
| controller | `src-tauri/src/adaptor/controller/command/code/review.rs`（新規） | `get_review_snapshot` / `get_review_file_view` の Tauri コマンド |
| protocol | `src-tauri/src/adaptor/protocol/code.rs`（追記） | ReviewSnapshot / ReviewFileView の入出力 DTO、`ReviewTarget` / `Viewport` 入力型 |
| usecase | `src-tauri/src/usecase/code_usecase.rs`（追記） | ReviewFileView の表示種別判定・read model 組み立てユースケース |
| usecase/dto | `src-tauri/src/usecase/code_dto.rs`（追記） | `ReviewSnapshotDto` / `ReviewFileViewDto`（tagged enum） |
| domain | `src-tauri/src/domain/code/`（追記） | 表示種別判定・threshold 値オブジェクト（`ReviewKind`, `ReviewThresholds`, `ReviewLimit`） |
| gateway | `src-tauri/src/adaptor/gateway/code/`（追記） | image/binary コンテンツの URL 供給（custom URI scheme / temp file。§9 Open Question 依存） |
| registration | `src-tauri/src/adaptor/controller/command/mod.rs` | `generate_handler!` へ 2 コマンド追加 |

### 2.2 Rust（再利用・最小改修）

- `RepositoryStateService::get_snapshot`（`usecase/repository_state/service.rs`）— ReviewSnapshot の土台。`base` への対応のみ追加（§3.2）。
- `code/file_content.rs` の text 生成ヘルパー（`get_file_at_branch_base` / `get_file_at_ref` / `get_staged_content` / working tree read）— ReviewFileView 内部から再利用。
- 既存 `get_review_text_diff` / `get_review_image_diff` / `get_status_diff_stats` / `build_diff_file_tree` は当面残置し、frontend からの呼び出しを撤去する（A7。即時削除は本 Issue の必須要件としない）。

### 2.3 frontend（改修）

| パス | 改修内容 |
| --- | --- |
| `src/hooks/useReviewSnapshot.ts`（新規） | `get_review_snapshot` を invoke し一覧 read model を保持。`useGitStatus` + `useDiffFileTree` のレビュー経路を置換 |
| `src/hooks/useReviewFileView.ts`（新規） | `get_review_file_view` を invoke し表示 read model を保持。`useFileDiffContent` + `useImageDiff` + `useGitOriginalContent` を置換 |
| `src/hooks/useImageDiff.ts` | 削除または URL 参照を返すだけの薄い形へ縮退（base64→data URL 変換を撤去） |
| `src/hooks/useGitOriginalContent.ts` | 削除（Git index path 組み立て・direct read を撤去） |
| `src/lib/imageUtils.ts` の `buildDataUrl` | レビュー経路からの参照を撤去（他用途が無ければ削除） |
| `src/components/panels/ReviewPanel.tsx` / `DiffFileTree.tsx` / `DiffViewerSection.tsx` | 新 hook の read model を表示する形へ再構成 |

> 注: `useGitStatus` はソースコントロールパネル等レビュー以外でも使われている可能性があるため、**レビュー経路のみ**を置換し、他経路は維持する（スコープ外への波及防止）。

---

## 3. アーキテクチャと責務分割

レイヤ責務は既存のクリーンアーキテクチャ構成（controller → usecase → domain / gateway）に従う。

```
frontend (表示のみ)
  └─ invoke: get_review_snapshot / get_review_file_view
        │
controller/command/code/review.rs       … 入出力 DTO 変換、AppError 変換
        │
usecase/code_usecase.rs                 … 表示種別判定の orchestration、threshold 適用
        │   ├─ RepositoryStateService.get_snapshot()   (一覧 read model)
        │   ├─ CodeQueryService (text original/modified 生成)
        │   └─ gateway/code (image/binary URL 供給)
        │
domain/code                             … ReviewKind 判定規則、ReviewThresholds 値
```

### 3.1 ReviewSnapshot の責務

- `RepositoryStateService::get_snapshot(worktree_path)` から versioned snapshot を取得する。
- `base` に応じて返す変更ファイル集合・tree を選択する（§3.2）。
- 各ファイルに安定 `file_id` を付与する（§5.2）。
- snapshot の `version` / `stale` / `loading` / `limited` を read model へ伝播する。

ReviewSnapshot は**新たな走査を行わず**、既存 snapshot を read model へ整形するだけの薄い層とする（R1「個別走査での都度生成に依存しない」）。

### 3.2 base ごとのファイル集合（設計判断）

既存 `RepositorySnapshot` は head 系の read model（`status` / `diff_stats` / `staged_diff_file_tree` / `changes_diff_file_tree`）を保持する。一方 `base=branch-base` は merge-base からの差分集合であり、現状の snapshot には含まれていない（frontend は `get_branch_diff_summary` で別取得していた）。

**D1（確定）**: `base=head` の ReviewSnapshot は snapshot の head 系 read model（`status` / `diff_stats` / `staged_diff_file_tree` / `changes_diff_file_tree`）からそのまま構成する。`base=branch-base` の ReviewSnapshot は、snapshot の `version`（一貫性の基準）に紐付けつつ、branch-base 差分集合を ReviewSnapshot ユースケース内で**補完取得**して read model 化する。branch-base 集合を versioned snapshot 本体へ取り込むのは `RepositoryStateService` 本体の拡張（#1210 スコープ）に当たるため、本 Issue では snapshot version を参照キーとした補完取得に留める（人間レビューで確定済み）。

### 3.3 ReviewFileView の責務

1. `target`（`file_id` または `path`）から対象ファイルを特定する（§5.2）。
2. 対象ファイルの**表示種別**を Rust 側で判定する: `text-diff` / `image` / `binary` / `fallback`（large file 等）。
   - 種別判定は拡張子・git attributes・内容のバイナリ検出・サイズの順で評価する（§6.1）。
3. 種別に応じた read model を組み立てる:
   - `text-diff`: `section` / `base` から original / modified を決定（既存 `get_review_text_diff` ロジック再利用）、`viewport` 適用、threshold 評価。
   - `image` / `binary`: original / modified の URL 参照を供給（§9）。
   - `fallback`: 全量を返さず、超過理由と最小メタ（行数・サイズ等）を返す。
4. threshold 超過を評価し `limited` フラグと超過理由を read model に反映する（§6）。

---

## 4. データモデルまたは型

### 4.1 入力型（protocol/code.rs）

```rust
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewSnapshotInput {
    pub worktree_path: String,
    pub base: String, // "branch-base" | "head"
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewFileViewInput {
    pub worktree_path: String,
    pub target: ReviewTargetInput,
    pub section: String, // "changes" | "staged"
    pub base: String,    // "branch-base" | "head"
    pub viewport: Option<ViewportInput>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", tag = "by", content = "value")]
pub enum ReviewTargetInput {
    FileId(String),
    Path(String),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewportInput {
    pub start_line: u32, // 1-origin、両端含む
    pub end_line: u32,
}
```

> `DiffBase` / `DiffSection` は requirements の合意どおり既存の文字列語彙を踏襲する（enum 化は本 Issue のスコープ外。内部では `is_branch_base` / `is_staged_section` 相当で判定）。

### 4.2 ReviewSnapshot 出力 DTO（code_dto.rs）

既存 `RepositorySnapshot` / `FileStatusDto` / `FileDiffStatDto` / `DiffTreeNodeDto` を再利用しつつ、各ファイルへ `file_id` を付与する。

```rust
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewSnapshotDto {
    pub version: u64,
    pub stale: bool,
    pub loading: bool,
    pub limited: bool,
    pub base: String,
    pub files: Vec<ReviewFileEntryDto>,   // 変更ファイル集合（file_id 付き）
    pub diff_stats: Vec<FileDiffStatDto>,
    pub tree: Vec<DiffTreeNodeDto>,        // base に対応する tree
    pub staged_tree: Vec<DiffTreeNodeDto>, // base=head 時のみ意味を持つ
    pub changes_tree: Vec<DiffTreeNodeDto>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewFileEntryDto {
    pub file_id: String,
    pub path: String,
    pub index_status: String,
    pub worktree_status: String,
    pub additions: u32,
    pub deletions: u32,
}
```

### 4.3 ReviewFileView 出力 DTO（tagged enum）

表示種別を `kind` タグで判別する。frontend はタグで viewer を分岐する。

```rust
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum ReviewFileViewDto {
    TextDiff(ReviewTextDiffDto),
    Image(ReviewImageDto),
    Binary(ReviewBinaryDto),
    Fallback(ReviewFallbackDto),
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewTextDiffDto {
    pub file_id: String,
    pub path: String,
    pub original: String,
    pub modified: String,
    pub source: ReviewTextSource, // 両側 diff か片側のみか
    pub limited: bool,
    pub viewport: Option<ViewportDto>, // 適用された範囲（未指定なら None=全量）
    pub total_lines: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ReviewTextSource {
    Diff,     // original/modified 双方あり（通常の変更）
    Added,    // 新規追加: original 空、modified のみ
    Deleted,  // 削除: original のみ、modified 空
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewImageDto {
    pub file_id: String,
    pub path: String,
    pub original_url: Option<String>, // 参照（base64 ではない）
    pub modified_url: Option<String>,
    pub mime: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewBinaryDto {
    pub file_id: String,
    pub path: String,
    pub original_url: Option<String>,
    pub modified_url: Option<String>,
    pub original_size: Option<u64>,
    pub modified_size: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewFallbackDto {
    pub file_id: String,
    pub path: String,
    pub reason: ReviewLimitReason, // 超過理由
    pub total_lines: Option<u32>,
    pub size_bytes: Option<u64>,
    pub hunk_count: Option<u32>,
    pub limited: bool, // 常に true
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ReviewLimitReason {
    FileSize,
    LineCount,
    HunkCount,
    Tokenization,
}
```

### 4.4 frontend 型（types/）

上記 DTO に対応する TypeScript 型を追加し、`useReviewSnapshot` / `useReviewFileView` で利用する。`kind` による discriminated union として表現する。

---

## 5. 処理フロー

### 5.1 ReviewSnapshot

```
get_review_snapshot(worktree_path, base)
 1. RepositoryStateService.get_snapshot(worktree_path) → Arc<RepositorySnapshot>
 2. base 分岐:
      head        → snapshot.status / diff_stats / staged_tree / changes_tree を採用
      branch-base → snapshot.version をキーに branch-base 差分集合を補完取得（§3.2 / Open Question）
 3. 各ファイルへ file_id を付与（§5.2）
 4. ReviewSnapshotDto { version, stale, loading, limited, base, files, ... } を返す
```

- 変更なし（空集合）でもエラーにせず空 `files` を返す（behavior「変更が無い場合は空のファイル一覧」）。

### 5.2 file_id の定義（仮定 D2）

**D2（確定）**: `file_id` は対象 worktree 内のファイルの**正規化済み相対パス**そのものとする（既存 `DiffTreeNodeDto.id` が path と同値である慣行に合わせる）。これにより `get_review_file_view` は `file_id`（=相対パス）でも `path`（相対パス）でも同一経路で解決でき、snapshot をまたいでも安定する。rename は rename 後 path を識別子とし、rename 前後を同一ファイルとして独立追跡はしない（人間レビューで確定済み）。

### 5.3 ReviewFileView

```
get_review_file_view(worktree_path, target, section, base, viewport?)
 1. target を解決 → 絶対 path（file_id=相対path / path から worktree_path 基準で解決）
 2. 表示種別判定（§6.1）:
      a. large file 判定（size / 行数）→ 超過なら Fallback(FileSize|LineCount)
      b. image 判定（拡張子 + バイナリ検出）→ Image
      c. binary 判定（git attributes / NUL 検出）→ Binary
      d. それ以外 → text-diff へ
 3. text-diff:
      - base/section から original/modified を決定（既存 file_content.rs ロジック再利用）
      - hunk 数算出 → 超過なら Fallback(HunkCount)
      - tokenization 量（文字数/行数）評価 → 超過なら Fallback(Tokenization)
      - viewport 指定時は該当行範囲のみ original/modified を切り出して返す
      - ReviewTextSource（Diff/Added/Deleted）を決定
 4. image/binary:
      - original(HEAD/staged 由来) / modified(working tree 由来) の URL を供給（§9）
 5. ReviewFileViewDto を返す
```

---

## 6. threshold とエラー処理

### 6.1 表示種別・threshold の判定順と確定値

requirements A6 の概値を確定値として採用する（design で最終調整 → 下記で確定）。

| 種別/閾値 | 確定値 | 超過時の挙動 |
| --- | --- | --- |
| large file（サイズ） | modified 側ファイルサイズ > **1 MiB（1,048,576 bytes）** | `Fallback(FileSize)` |
| large file（行数） | 行数 > **5,000 行** | `Fallback(LineCount)` |
| hunk 数 | hunk 数 > **300** | `Fallback(HunkCount)` |
| tokenization | 文字数 > **100,000 文字** または 行数 > **5,000 行** | `Fallback(Tokenization)` |

判定順（早期確定で重い処理を避ける）:
1. **サイズ**: ファイルサイズ（メタデータのみ、内容読込前）で large file を弾く。
2. **バイナリ/image**: 拡張子 + 先頭バイトの NUL 検出。image は asset URL 経路、それ以外 binary は binary 経路。
3. **行数**: text のとき行数で large file を弾く。
4. **hunk 数**: diff 計算後の hunk 数。
5. **tokenization**: 文字数/行数。

> 行数 threshold（large file）と tokenization の行数条件（ともに 5,000 行）は重複するが、超過理由を区別するため判定順で先に評価した方（large file=`LineCount`）を採用する。tokenization は「行数は閾値内だが文字数のみ超過」のケースで `Tokenization` として効く。

### 6.2 limited の伝播

- ファイル単位の超過は `ReviewFileViewDto::Fallback { limited: true, reason }` で表現する。
- `ReviewTextDiff` でも viewport により部分返却した場合等、全量でないことを示すため `limited` を持つ（true/false）。
- snapshot 単位の `limited`（#1210 由来）は `ReviewSnapshotDto.limited` として伝播する（本 Issue では snapshot 側 limited は基本 false。ReviewFileView 側で新たに評価する）。

### 6.3 エラー処理

- 各層の専用エラー型を踏襲する。controller は `AppError` へ変換して返す（既存 review command と同様）。
- 対象ファイルが snapshot に存在しない（target 解決失敗）→ ルール違反系エラー（`AppError`）。frontend はエラー表示にフォールバック。
- `UnbornBranch`（初回コミット前）→ 既存ロジック同様、original を空として扱い `ReviewTextSource::Added` 相当で返す。
- image/binary の original 側（HEAD/staged）が存在しない（新規追加）→ `original_url = None`。modified 側が存在しない（削除）→ `modified_url = None`。
- threshold 超過は**エラーではなく** `Fallback` read model として正常応答する（UI を固めないため）。
- viewport が範囲外/逆転 → 安全側に clamp し、空または可能な範囲を返す（エラーにしない）。

---

## 7. frontend の責務縮小（R4）

- `useReviewSnapshot`: `get_review_snapshot` の結果（`files` / `tree` / `version` / フラグ）をそのまま保持・表示する。status / diff stats / tree を frontend で組み立てない。version によるレース制御（既存 `useGitStatus` / `useDiffFileTree` の acceptedVersion 方式）は踏襲する。
- `useReviewFileView`: `get_review_file_view` の `kind` で表示を分岐する。
  - `text-diff` → CodeDiffViewer に original/modified を渡す。
  - `image` → ImageDiffViewer に `original_url` / `modified_url` をそのまま渡す（`buildDataUrl` 廃止）。
  - `binary` → binary 表示（URL 参照）。
  - `fallback` → fallback 表示（「大きすぎるため表示を省略」等、`reason` に応じたメッセージ）。
- 撤去するもの: `useGitOriginalContent`（Git index path 組み立て・direct read）、`useImageDiff` の base64→data URL 変換、`diffBase`/`section` に応じた original/modified の組み立て準備。
- `gitRefreshKey` 相当の再フェッチ契機は、snapshot の `version` 変化と既存 Git イベント購読に置き換える（ReviewFileView は version をクエリに含めることで stale を判別可能にする）。

---

## 8. 既存 command の扱い（A7）

- 本 Issue では新 API を追加し、**frontend からの旧 command 呼び出しを撤去**することを必須とする（R4 受け入れ基準）。
- Rust 側の旧 command（`get_review_text_diff` / `get_review_image_diff` / `get_status_diff_stats` / `build_diff_file_tree` / `get_repo_git_dir`）は、内部ヘルパーを ReviewFileView / ReviewSnapshot から再利用しつつ、コマンド登録自体は当面残置する（即時削除は必須要件としない）。
- 完全削除は frontend 置換完了後の別 PR で行う（スコープ拡大防止）。

---

## 9. image / binary の URL 供給方式（R5）

要件: base64 data URL を廃し、Tauri asset / resource URL（asset protocol 経由の URL 参照）で返す。

論点: working tree（modified）側は実ファイルが存在するため `convertFileSrc`（`asset:` protocol）で直接 URL 化できる。しかし **original 側（HEAD / staged 由来）は実ファイルが存在しない**ため、何らかの方法でコンテンツを URL から取得可能にする必要がある。

検討した 2 案:

- **案 A（推奨）: custom URI scheme handler**
  `tauri::Builder::register_uri_scheme_protocol` で `review-blob:` 等のスキームを登録し、`?worktree=...&file_id=...&section=...&base=...&version=...` をキーに Rust が HEAD/staged/working tree のバイト列を動的に返す。ファイルシステムに書かない。version をクエリに含めるため stale 時のキャッシュ無効化が自然。working tree 側も統一的に同スキームで扱える。ライフサイクル管理（一時ファイル掃除）が不要。
- **案 B: 一時ファイル化 + convertFileSrc**
  HEAD/staged コンテンツをアプリ temp dir に書き出し `convertFileSrc` で URL 化する。`asset` scope 設定と temp ファイルの掃除（世代交代・アプリ終了時）が必要。実装は素直だが掃除責務とディスク IO が増える。

**確定: 案 A（custom URI scheme handler）を採用する。** working tree（modified）側も含め `review-blob:` スキームで統一的に扱う。

URL ライフサイクル（案 A 採用時）:
- URL は `version` を含むため snapshot 更新で別 URL になり、古い表示は自然に置き換わる。
- handler は要求都度 git2 でコンテンツを生成する（キャッシュは持たない、または LRU で短期保持）。

---

## 10. テスト方針

### 10.1 Rust（`#[cfg(test)] mod tests`）

- **ReviewSnapshot**:
  - 変更複数 → `files` に file_id 付きで全件含まれ、`diff_stats` / `tree` が同一 version 由来で整合する。
  - 変更なし → 空 `files`、エラーにならない。
  - base=head / base=branch-base それぞれで対応する集合が返る（behavior Scenario Outline 準拠）。
- **ReviewFileView 表示種別判定**:
  - text / image / binary / fallback の各分岐。
  - file_id / path 双方で同一ファイルが解決される。
  - section=changes / staged で original/modified が切り替わる。
  - ReviewTextSource（Diff/Added/Deleted）の判定。
- **threshold**:
  - サイズ超過 / 行数超過 / hunk 数超過 / tokenization 超過の各ケースで `Fallback(reason)` が返り `limited=true`。
  - 全 threshold 内で通常 read model・`limited=false`。
  - 境界値（1 MiB ちょうど / 5,000 行ちょうど / hunk 300 ちょうど / 100,000 文字ちょうど）の上下。
- **viewport**:
  - 指定時に範囲のみ返る。範囲外/逆転を clamp。
- **image/binary URL**:
  - 返却値が base64 data URL でなく URL 参照であること。新規追加で original_url=None、削除で modified_url=None。
- `UnbornBranch` の original 空扱い。

### 10.2 frontend（Vitest）

- `useReviewSnapshot`: invoke スタブで read model 保持、version レース（古い version 破棄）。
- `useReviewFileView`: `kind` 別の状態遷移、image で URL をそのまま渡す（buildDataUrl を呼ばない）。
- viewer 分岐コンポーネント: `kind=fallback` で fallback UI、`kind=image` で URL を `<img src>` に渡す。
- Tauri API は `vi.mock` でスタブ化（CLAUDE.md 方針）。

### 10.3 非実施

- 実 `git push` 等の外部プロセスは実行しない。
- Monaco Editor の命令型 API はロジック単体テストに留める。

---

## 11. リスクと代替案

- **R-1 base=branch-base の versioned snapshot 整合**: snapshot に branch-base 集合が無い（§3.2）。補完取得（D1 確定）では「同一 snapshot 由来」の保証が head 系より弱まる。snapshot.version を参照キーに紐付け、取得時点の version 不一致を `stale` として伝播することで緩和する。
- **R-2 image/binary URL 方式**: custom scheme handler（D4 確定）は Tauri 2 の URI scheme 登録・セキュリティ scope の設定が必要。`review-blob:` のクエリ検証（worktree/path のトラバーサル防止）を handler 側で行う。
- **R-3 既存 command 残置による二重経路**: 旧 command と新 API の併存期間に挙動差が出るリスク。frontend 置換を本 Issue で完了し、Rust 側削除を別 PR にすることで縮減。
- **R-4 useGitStatus のレビュー経路のみ置換**: 他経路（ソースコントロール等）との共有による回帰。レビュー経路を分離し他経路を維持することで波及を限定。
- **R-5 threshold 値の妥当性**: 確定値はヒューリスティック。`ReviewThresholds` を 1 箇所に集約し将来調整可能にする。

---

## 12. 仮定

requirements A1〜A7 / behavior の仮定を引き継ぎ、本書で追加した設計仮定:

- **D1**: base=head は snapshot から直接構成、base=branch-base は snapshot version を参照キーに補完取得する（§3.2）。
- **D2**: `file_id` は正規化済み相対パスとする（§5.2）。
- **D3**: threshold 確定値は §6.1 のとおり（large file > 1 MiB / > 5,000 行、hunk > 300、tokenization > 100,000 文字 or > 5,000 行）。
- **D4**: image/binary の URL 供給は custom URI scheme handler（案 A）を主案とする（§9）。
- **D5**: ReviewFileView の戻り値は `kind` タグ付き enum とする（§4.3）。
- **D6**: 旧 review command は残置し、frontend 呼び出しのみ撤去する（§8）。
- **D7**: `viewport` は 1-origin・両端含みの行範囲 `{ startLine, endLine }` とする（§4.1）。

---

## 13. Open Questions

なし。当初の 3 点は人間レビューで以下のとおり確定した。

1. **image/binary の URL 供給方式**（§9 / D4）→ **案 A: custom URI scheme handler** を採用。
2. **base=branch-base の ReviewSnapshot ファイル集合の出所**（§3.2 / D1）→ **ReviewSnapshot ユースケースが snapshot version を参照キーに補完取得**（#1210 本体は変更しない）。
3. **file_id の識別子設計**（§5.2 / D2）→ **正規化済み相対パスをそのまま file_id とする**（rename 後 path を識別子とする）。
