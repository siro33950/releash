# Design

Issue: #1212 「Hunk operation を id-based にし frontend patch 再生成を削除する」

本書は `requirements.md`（R1〜R5 / D1〜D3 / A1〜A2）と `behavior.md` の Gherkin を、実装方針・責務分割・データ構造・エラー処理・テスト方針として具体化する。requirements の仮定 A1（id 生成規則）/ A2（コマンド命名・移行）を本書で確定する。

## 概要

stage / unstage の対象指定を、位置依存の `group_index` + `snapshot_version` ガードから、**内容に紐づく安定 id（`hunk_id` / `group_id`）+ intent** へ置き換える。Rust 側は受け取った id を「現在の snapshot 上で再計算した change group 群」に解決し、解決できた場合のみ staged をベースに patch を再構築して `git apply --cached` で適用する。解決できなければ何も適用せずエラーを返す。frontend はそのエラーを捕捉して snapshot を refresh する。

これにより、

- read model（`ReviewTextDiffDto`）は file refresh をまたいで安定する id を返す（R1）。
- frontend は `group_index` / `snapshot_version` を一切持ち回らず、id + intent + 座標語彙のみを渡す（R2 / R3 / D1）。
- 整合性は version ガードではなく「id を現在 snapshot に解決できるか」で担保する（R4 / D1）。
- 解決失敗は安全な no-op + エラー（R5 / D2）。

## 変更対象

### Rust（`src-tauri/src/`）

| ファイル | 変更内容 |
|---|---|
| `domain/code/value_objects/hunk.rs` / `domain/code/services/hunk.rs` | `Hunk` に `hunk_id`、`ChangeGroup` に `group_id` を追加。id 算出純粋関数（`compute_hunk_id` / `compute_group_id`）と、review 操作で変化しない side の全文に対する occurrence で `group_id` を再付与する helper を新設。 |
| `usecase/code_dto.rs` | `HunkDto` に `hunk_id`、`ChangeGroupDto` に `group_id` を追加。`hunk_to_dto` / `change_group_to_dto` / `hunk_dto_to_domain` / `change_group_dto_to_domain` の往復で id を維持。 |
| `usecase/review_usecase.rs` | `generate_review_group_patch` を `group_index: u32` 引数から `group_id: &str` 引数へ変更し、`change_groups.iter().find(|g| g.group_id == group_id)` で解決。解決失敗時は専用エラー。`git_stage_review_group` / `git_unstage_review_group` のシグネチャを id ベースへ変更し `snapshot_version` 引数と `ensure_current_review_snapshot_version` 呼び出しを除去。 |
| `usecase/review_usecase.rs` | `ensure_current_review_snapshot_version` を削除（D1）。 |
| `adaptor/protocol/code.rs` | `ReviewGroupActionInput` から `group_index` / `snapshot_version` を除去し `group_id: String` を追加。 |
| `adaptor/controller/command/code/review.rs` | `git_stage_review_group` / `git_unstage_review_group` コマンドの input マッピングを id ベースへ変更。コマンド名は据え置き（後述）。 |
| `adaptor/controller/command/code/staging.rs` | patch 文字列を受け取る Tauri コマンド `git_stage_hunk` / `git_unstage_hunk` を**削除**（死コード確認済み。後述）。コマンド登録（`invoke_handler`）からも除去。 |

### Frontend（`src/`）

| ファイル | 変更内容 |
|---|---|
| `components/panels/useDiffOperations.ts` | `snapshotVersion` パラメータを除去。`applyGroupAction(command, groupIndex)` を `applyGroupAction(command, groupId)` へ変更し、input から `groupIndex` / `snapshotVersion` を除き `groupId` を渡す。`catch` の握りつぶしを解消し、解決失敗エラー時に snapshot refresh をトリガーする（後述）。 |
| `components/panels/ReviewPanel.tsx` | `useDiffOperations` への `snapshotVersion` 受け渡しを除去。stage / unstage ハンドラへ渡す引数を `group_id` 由来へ変更。 |
| `hooks/useGitActions.ts` | 死コードの `stageHunk` / `unstageHunk` を**削除**。 |
| `types/`（該当 diff 型） | `ChangeGroup` / `Hunk` の TS 型に `groupId` / `hunkId` を追加（read model 表示用）。 |

## アーキテクチャと責務分割

レイヤー責務（`src-tauri/CLAUDE.md` の clean architecture）を踏襲する。

- **domain（`hunk.rs`）**: id の算出は純粋計算。snapshot / git / I/O に依存しない。`Hunk` / `ChangeGroup` 値オブジェクトに id フィールドを持たせる。通常の `compute_change_groups` は diff 内容 hash を付与し、review 操作用の stable id は呼び出し元が渡す original / modified と stable side に基づいて再付与する。
- **usecase（`review_usecase.rs`）**: 「id → 現在 snapshot 上の change group 解決 → patch 再構築 → 適用」の業務手順。version ガードは持たない。`changes` は modified/working tree 側、`staged` は original/HEAD 側を stable side として group id を生成する。
- **adaptor/controller（`review.rs` / `protocol/code.rs`）**: frontend 境界の DTO（`ReviewGroupActionInput`）を id ベースへ。
- **frontend（`useDiffOperations` / `ReviewPanel`）**: invoke と、解決失敗時の refresh ハンドリングのみ。patch 生成・座標/version 持ち回りを持たない。

## データモデルまたは型

### 安定 id の生成規則（A1 の確定）

**決定: id は「対象の diff 内容」に紐づく content hash とし、同一内容が同一ファイル内に複数ある場合は出現順 ordinal で曖昧性を排除する。位置（行番号 / オフセット / `group_index`）は id に含めない。**

理由:

- behavior「同一内容なら同じ id・別内容なら別 id」を満たす最小条件は内容由来であること。
- 位置を含めると、connected な連続 stage（behavior「連続した stage 操作が staged の進行に追随する」）で、先行操作により後続対象の行位置がずれた際に id が不一致になり解決失敗する。位置非依存にすることで、内容が変わらない限り refresh をまたいでも解決できる。
- 解決時の patch 再構築に必要な位置情報は、解決先の「現在 snapshot 上で再計算した change group / hunk」から取得する。id は「どの group か」を指すだけで、位置は持たない。

具体規則（domain の純粋関数）:

- `compute_group_id(hunk, group, occurrence) -> String`:
  - 対象 change group の diff 行（`hunk.lines[line_offset_start..=line_offset_end]`、prefix `+` / `-` / ` ` を含む生の行）に加え、境界 context（`GROUP_CONTEXT_RADIUS=1` の前後 context 行）を identity hash 入力へ含める。
  - 境界 context を含める理由は、純削除 group は Modified/working 側に自身の行が射影されず、純挿入 group は Original 側に自身の行が射影されないため。context なしでは stable side occurrence の needle が空になり、現在残っている集合に依存する fallback occurrence に落ちてしまう。前後 context を identity に含めることで、挿入/削除だけの group も stable side の全文に anchor できる。
  - 同一ファイル内で content が衝突する場合に備え、同一 content の出現順 `occurrence`（0 始まり）を加える。review 操作では、この occurrence は「現在残っている change group 集合」ではなく、操作で変化しない side の全文に現れる side-projected identity の出現順とする。`changes` は modified/working tree 側、`staged` は original/HEAD 側を使う。
  - 形式（仮）: `g:{hex(hash(content))}:{occurrence}`。ハッシュは標準ライブラリの `std::hash`/`DefaultHasher` ではなく、安定性のため SHA-256 等の決定的ハッシュを用いる（プロセス間・version 間で安定する必要があるため。`DefaultHasher` は seed が安定する保証がないので不可）。依存追加可否は実装時に既存 `Cargo.toml` を確認し、既存クレートで賄えない場合は最小の決定的ハッシュ実装を `domain` 内に閉じる（仮定 A3）。
- `compute_hunk_id(hunk, occurrence) -> String`:
  - hunk の全 `lines` を改行連結したものをハッシュ入力とし、同一 content 出現順 `occurrence` を付す。
  - 形式（仮）: `h:{hex(hash(content))}:{occurrence}`。
- 通常の diff 計算では `compute_change_groups` / hunk 列挙の走査中に同一 content の出現回数をカウントして割り当てる。review 操作の read model / id 解決では、先に stage/unstage された同一局所 pattern が change group 集合から消えても後続 group の id が変わらないよう、stable side の全文に対する occurrence で再付与する。

### 型定義の変更

`domain/code/value_objects/hunk.rs`:

```rust
pub struct Hunk {
    pub index: u32,
    pub hunk_id: String,   // 追加
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    pub lines: Vec<String>,
}

pub struct ChangeGroup {
    pub group_index: u32,  // 表示・整列用に残置（requirements S1）
    pub group_id: String,  // 追加
    pub hunk_index: u32,
    pub new_start: u32,
    pub new_end: u32,
    pub line_offset_start: u32,
    pub line_offset_end: u32,
    pub is_staged: Option<bool>,
}
```

`usecase/code_dto.rs`（serde camelCase）:

```rust
pub struct HunkDto {
    pub index: u32,
    pub hunk_id: String,        // camelCase: hunkId
    // ...既存...
}

pub struct ChangeGroupDto {
    pub group_index: u32,       // 残置
    pub group_id: String,       // camelCase: groupId
    pub hunk_index: u32,
    // ...既存...
}
```

`adaptor/protocol/code.rs`（frontend 境界）:

```rust
pub struct ReviewGroupActionInput {
    pub worktree_path: String,
    pub path: String,
    pub section: String,
    pub base: String,
    pub group_id: String,   // group_index / snapshot_version を置換
}
```

### コマンド命名と移行（A2 の確定）

**決定: Tauri コマンド名は既存の `git_stage_review_group` / `git_unstage_review_group` を据え置き、引数（input DTO）のみを id ベースへ置き換える（並存させない）。**

理由:

- Issue 記載の `stage_hunk_by_id` / `unstage_hunk_by_id` は「id ベースにする」という意図の表現であり、コマンド名そのものの指定ではない。既存コマンドは既に review group 単位（= change group 単位、D3）で動作しており、命名 `*_review_group` は粒度として正確。
- 改名は frontend invoke 文字列・コマンド登録・テストの広範な置換を生み、振る舞い不変のリネームは本 Issue の本質（id 化）と無関係なノイズになる。
- 「id ベース operation」という意味は input の `group_id` で表現され、外部観測上 version / index を渡さないことで満たされる。

> 別案として新コマンド `stage_review_group_by_id` を追加し旧コマンドを deprecated にする案もあるが、requirements D1（version ガード完全廃止・並存させない）に反するため採らない。

### 死コードの扱い（S4 の確定）

調査により以下が判明:

- frontend `useGitActions.stageHunk` / `unstageHunk`（patch 文字列を `git_stage_hunk` / `git_unstage_hunk` へ渡す）は **呼び出し元なし（テスト除く）= 死コード**。→ 本 Issue で削除する。
- Tauri コマンド `git_stage_hunk` / `git_unstage_hunk`（`staging.rs`、patch 文字列引数）は frontend からの上記経路のみが呼び元 = **死コード**。→ コマンド定義と `invoke_handler` 登録から削除する。
- ただし usecase / gateway の `code.git_stage_hunk(worktree_path, &patch)`（`adaptor/gateway/code/staging.rs` の `git apply --cached`）は **id ベースの新経路が内部で利用し続ける**ため残置する。

削除対象は「patch 文字列を frontend から直接受ける Tauri コマンドと frontend ラッパ」に限定し、内部 gateway の patch 適用は維持する。

## 処理フロー

### read model 取得（R1）

1. `review_text_view`（`review_usecase.rs`）が snapshot から original / modified を select し `compute_diff_hunks` を呼ぶ。
2. `compute_change_groups`（domain）が hunk / group 列挙時に `compute_hunk_id` / `compute_group_id` を計算し、review read model では `changes` / `staged` の stable side に基づいて `group_id` を再付与する。
3. `ReviewTextDiffDto.hunks[].hunk_id` / `.change_groups[].group_id` として frontend へ返る。

### id ベース stage / unstage（R2 / R4 / D1）

1. frontend `useDiffOperations.applyGroupAction(command, groupId)` が `invoke(command, { input: { worktreePath, path, section, base, groupId } })`。
2. controller が `ReviewGroupActionInput`（id ベース）を受け、usecase `git_stage_review_group(worktree, path, section, base, group_id)` を呼ぶ。
3. usecase: `snapshot(worktree)` を取得（version ガードなし）。`generate_review_group_patch(..., group_id, snapshot)` を呼ぶ。
4. `generate_review_group_patch`:
   - branch-base なら従来どおり Rule エラー（behavior「branch-base での id 指定 stage はエラー」）。
   - snapshot から original / modified を select し `compute_diff_hunks` で再計算。
   - `change_groups.iter().find(|g| g.group_id == group_id)` で解決。**見つからなければ「id 解決失敗」エラー**（後述の専用エラー、D2）。
   - 解決した group の `hunk_index` から hunk を引き、`generate_group_patch` で staged ベースの patch を再構築。
5. usecase が `code.git_stage_hunk` / `code.git_unstage_hunk`（`git apply --cached` / `--reverse`）で適用。
6. 成功時 frontend は `onGitChanged`（refresh）。

### 解決失敗時の回復（R5 / D2）

1. usecase が「id 解決失敗」エラーを返す。
2. controller がエラーを `AppError` として frontend へ伝播。
3. `useDiffOperations` の `catch` がエラー種別を判定し、**snapshot refresh をトリガー**（`onGitChanged` ないし専用の refresh コールバック）。握りつぶし（`console.error` のみ）はしない。

## エラー処理

- **branch-base 非対応**: 既存どおり `CodeError::Rule("review group actions are not available for branch-base diffs")`。挙動不変。
- **id 解決失敗**（対象 group が現在 snapshot 上に存在しない）: 専用バリアントを用意する。既存は `CodeError::Rule` の文字列だが、frontend が「refresh で回復すべき失敗」と「それ以外」を区別できるよう、識別可能なエラーにする。
  - 案: `CodeError` に `ReviewTargetStale`（または `ReviewGroupNotFound`）バリアントを追加し、`AppError` へのマッピングで安定した error code / kind を持たせる。frontend は code/kind で判定して refresh する。
  - これにより behavior「解決失敗を frontend が捕捉して snapshot を refresh する」を、文字列マッチに頼らず満たす（仮定 A4: error code の表現方法は既存 `AppError` 整形に合わせて実装時確定）。
- **適用失敗**（`git apply --cached` 失敗）: 既存どおり stderr を Rule エラーで返す。staged 状態は git 側で原子的に未適用（behavior「staged の状態は変化しない」）。
- いずれも patch を適用する前に解決・branch-base 判定を行うため、エラー時に部分適用は発生しない（no-op 保証）。

## テスト方針

### Rust 単体（各 module 内 `#[cfg(test)]`）

- **domain `hunk.rs`**:
  - `compute_group_id` / `compute_hunk_id`: 同一内容 → 同一 id、別内容 → 別 id。
  - 同一内容 group が複数 → occurrence で別 id になり衝突しない。
  - 位置（new_start / old_start）だけが違い内容が同じ → 同一 id（位置非依存の確認、連続 stage 追随の根拠）。
- **usecase `review_usecase.rs`**:
  - id ベース `generate_review_group_patch`: 既存 group_index ベーステストを id ベースへ移行し、正しい group を解決して patch を生成すること。
  - 解決失敗（存在しない group_id）→ 専用エラー、patch 未生成。
  - branch-base → Rule エラー（挙動不変）。
  - file refresh をまたいだ安定性: 同一内容で再計算後も同一 id で解決できる / 内容変更後は別 id になり旧 id は解決失敗（R5・behavior の同名 Scenario）。
  - 連続 stage: changes に複数 group があるとき、1 つ目を stage 後に 2 つ目を旧 read model の id で stage しても解決・適用成功（staged ベース追随、`git apply --cached` ベース不一致なし。R4・受け入れ基準）。
- **protocol `code.rs`**: `ReviewGroupActionInput` が `groupId` を camelCase で受理し、`group_index` / `snapshot_version` を含まないこと。

### frontend（Vitest、Tauri invoke は `vi.mock`）

- `useDiffOperations`: stage / unstage で invoke する input が `{ worktreePath, path, section, base, groupId }` のみ（`groupIndex` / `snapshotVersion` を含まない）であること（R3・behavior「frontend が渡すのは … に限られる」）。
- 解決失敗エラーを受けたとき refresh がトリガーされ、握りつぶされないこと（D2・behavior「解決失敗を frontend が捕捉して snapshot を refresh する」）。

### 回帰

- 死コード削除後に `pnpm build` / `cargo clippy -- -D warnings` が通る（未使用警告も含む）。
- 既存の group 単位 stage / unstage の振る舞い（changes で stage / staged で unstage / 粒度は change group）が id ベースで維持される（D3）。

CI と同一コマンド（`pnpm lint` / `pnpm test` / `pnpm build`、`cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test`）で検証する。

## リスクと代替案

- **R1: content hash の衝突**。異なる内容が同一ハッシュになると誤解決する。SHA-256 等の十分な空間のハッシュを用い、さらに occurrence で同一ファイル内の同一内容を区別するため、実用上の誤解決は無視できる。代替: 内容 + 旧位置を含めるとより一意だが、連続 stage で位置がずれ解決失敗するため不採用（前述）。
- **R2: 連続操作で内容が同一の別箇所がある場合の occurrence ずれ**。先行 stage / unstage で同一内容 group の集合が変化すると、現在の change group 集合だけを数えた occurrence はずれ得る。このため review 操作では `changes` は modified/working tree 側、`staged` は original/HEAD 側の全文に対する occurrence を使い、surviving group 集合に依存しないようにする。対象そのものが消失した場合は D2 の stale エラーで回復する。
- **R3: 決定的ハッシュの依存追加**。`std` の `DefaultHasher` は version/プロセス間安定性が保証されないため使えない。既存依存に SHA 系があれば再利用、なければ最小実装を domain に閉じる（A3）。
- **R4: error 種別の表現**。frontend が refresh すべき失敗を判別するためのエラー識別子は、既存 `AppError` 整形方式に依存（A4）。文字列マッチに退行させない。

## 仮定

requirements の A1 / A2 は本書で確定（id = 位置非依存の content hash + stable side occurrence、コマンド名据え置きで input のみ id 化）。本書で追加した仮定:

- **A3**: 安定ハッシュは決定的アルゴリズム（SHA-256 等）を用いる。クレート選定は実装時に既存 `Cargo.toml` を確認し、最小追加または domain 内実装に閉じる。
- **A4**: id 解決失敗エラーは frontend が判別可能な識別子（error code / kind）を持つ。具体的表現は既存 `AppError` / presenter の整形方式に合わせて実装時確定する。
- **A5**: `group_index` / `index` / `hunk_index` は表示・整列用に read model へ残す（requirements S1）。stage / unstage の対象特定にはこれらを使わない。

## Open Questions

なし（requirements D1〜D3 で前提確定、A1 / A2 を本書で確定、残差 A3〜A5 は外部観測可能な振る舞いに影響しない実装内決定のため Open Question にしない）。
