# Requirements

Issue: #1211 「ReviewSnapshot / ReviewFileView command を追加し diff 表示を Rust read model に寄せる」

関連: #1210（RepositoryStateService / 本 Issue の基盤・先行）/ #1212（hunk operation の id 化）/ #767 / #805 / #866 / `docs/releash-performance-architecture-audit.md` M1（正本ドキュメント, commit `b0c5e4c2`, マイルストーン: 性能・メモリ効率改善）

## Type

新機能 / リファクタリング。性能・メモリ効率改善（マイルストーン M1「Git / Diff hot path を Rust read model に寄せる」）の一部。Issue コメントの実装順「4-2: Git / Diff read model」に位置づけられ、#1210 の `RepositoryStateService` 上に **ReviewSnapshot / ReviewFileView** command を追加して、frontend が持つ direct FS read と diff orchestration を Rust read model へ寄せる。

## 背景と目的

現状、レビュー（diff 表示）の read model 生成と file IO が frontend に分散している。

- ファイル一覧（file list）が `useGitStatus`（status 取得 + watcher）、`useDiffFileTree`（`get_status_diff_stats` + `build_diff_file_tree`）など複数の hook / command 呼び出しに分かれている。
- ファイルを開く（file open）処理で、frontend が diff 基準（`DiffBase` = `branch-base` | `head`）と区画（`DiffSection` = `changes` | `staged`）の選択、original / modified の組み立て準備を持つ。`useFileDiffContent` が `get_review_text_diff`、`useImageDiff` が `get_review_image_diff` を呼ぶ。
- `useImageDiff` は Rust から base64 を受け取り、frontend 側で `buildDataUrl` により base64 data URL を生成している（メモリ・転送コストが大きい）。
- `useGitOriginalContent` は `get_repo_git_dir` でディレクトリを取得し、Git index path を組み立てて original 取得経路を frontend 側で持つ（direct な Git 内部経路の参照）。
- large file / hunk 数が多い場合・tokenization が重い場合の打ち切り判断が frontend 側に依存しており、UI が固まる懸念がある。

`docs/releash-performance-architecture-audit.md` M1 の結論にあるとおり、問題は個別機能の遅さではなく「frontend が working tree を直接読み、diff 基準選択 / tree 化 / image base64 化 / patch generation 準備まで持っている」設計の広がりにある。

本 Issue では、`RepositoryStateService`（#1210）の versioned snapshot を土台に、**ファイル一覧 API（ReviewSnapshot）** と **ファイル表示 API（ReviewFileView）** を分離して追加し、diff 表示に必要な判断（original / modified / source の決定、large-file fallback、image/binary の参照方式、threshold 適用）を Rust read model 側へ集約する。これにより frontend は read model を表示するだけになり、Git orchestration と file IO を持たない構成にする（`.claude/rules/rust-first-logic.md` 準拠）。

## 合意済みの前提

- 本 Issue は #1210 の `RepositoryStateService`（versioned snapshot / cache 基盤）が先行して存在することを前提とする（Issue 本文・コメント「順序 4-1 → 4-2」）。
- 対象は **デスクトップアプリのレビュー（diff 表示）経路**であり、ロジックは Rust（Tauri コマンド）に実装し frontend は表示に徹する（プロジェクト方針）。
- 既存の diff 基準語彙 `DiffBase`（`branch-base` | `head`）と区画語彙 `DiffSection`（`changes` | `staged`）を踏襲する（語彙そのものの変更はしない）。

## 仮定

以下は task とリポジトリ内の情報から置いた仮定であり、本文中で「仮定」と明示する。Open Questions で確定すべきものは別途記載する。

- **A1**: `get_review_snapshot(worktree_path, base)` は、現状 `useGitStatus` + `useDiffFileTree` + `get_status_diff_stats` が分散して生成しているファイル一覧 read model（status / diff stats / diff file tree）を、`RepositoryStateService` の同一 versioned snapshot から 1 回の呼び出しで返す API とする。
- **A2**: ReviewSnapshot は各ファイルに対し、ReviewFileView が参照する**安定した `file_id`** を払い出す。`get_review_file_view` はこの `file_id` または `path` のどちらでも対象を特定できる（Issue 記載の `file_id | path`）。
- **A3**: `get_review_file_view(file_id | path, section, base, viewport?)` は、対象ファイルの表示種別（text diff / image / binary / large-file fallback / source-only）を Rust 側で判定し、種別に応じた read model（text の original/modified、image/binary の参照、fallback メタ情報）を返す。
- **A4**: `viewport?` は任意指定で、large file 等で全量を返さず必要範囲のみを返すための表示範囲ヒント（行範囲等）とする。未指定時は Rust 側の threshold に従い全量または fallback を返す。
- **A5**: image / binary は base64 data URL をやめ、frontend が `<img src>` 等にそのまま使える参照として返す。返却方式は **Tauri の asset / resource URL**（`convertFileSrc` 等の asset protocol 経由）を正とする（合意済み）。frontend 側の base64→data URL 変換（`buildDataUrl` 経路）は削除する。HEAD / staged 由来など実ファイルが存在しない側の扱い（一時ファイル化の要否）と URL ライフサイクルは design で詰める。
- **A6**: threshold（large file サイズ / hunk 数 / tokenization 上限）は requirements に**概値**を明記し、design で最終調整する（合意済み）。概値は次のとおり: large file = ファイルサイズ > 約 1MB または > 約 5,000 行 / hunk 数 > 約 300 / tokenization = 内容 > 約 10 万文字（または約 5,000 行）でトークン化をスキップし fallback。閾値超過の判定・打ち切りは Rust 側で行い、snapshot / file view に `limited`（または相当）フラグとして反映する（#1210 の `limited` フラグに倣う）。
- **A7**: 既存 command（`get_review_text_diff` / `get_review_image_diff` / `get_status_diff_stats` / `build_diff_file_tree` 等）の最終的な統廃合範囲は design で確定する。本 Issue の受け入れ基準は「frontend が direct FS read と Git orchestration を持たない」ことを満たすことを必須とし、旧 command の即時削除そのものは必須要件としない。

## スコープ

### ReviewSnapshot command の追加（ファイル一覧 API）

- `get_review_snapshot(worktree_path, base)` を追加する。
- `RepositoryStateService`（#1210）の versioned snapshot から、レビューに必要なファイル一覧 read model（変更ファイル集合 / status / diff stats / diff file tree、および各ファイルの表示種別判定に要する最小メタ情報）を 1 回の呼び出しで返す。
- 各ファイルに対し ReviewFileView が参照可能な安定 `file_id` を払い出す（仮定 A2）。
- snapshot は version / `stale` / `loading` / `limited` 相当のフラグを保持・伝播する（#1210 の versioned snapshot に整合）。

### ReviewFileView command の追加（ファイル表示 API）

- `get_review_file_view(file_id | path, section, base, viewport?)` を追加する。
- Rust 側で対象ファイルの表示種別（text diff / image / binary / large-file fallback）を決定し、対応する read model を返す。
- text diff は original / modified / source（および差分情報）を Rust が決定して返す。frontend は受け取った内容を表示するだけにする。
- large file / 多数 hunk / tokenization 超過時は、UI を固めない fallback 表示用の read model（全量を返さず、fallback である旨と必要なら部分内容 / viewport 範囲）を返す。

### image / binary の参照方式の変更

- image / binary は base64 data URL ではなく、**Tauri asset / resource URL**（asset protocol 経由の URL 参照）として返す。
- frontend の base64→data URL 変換経路（`useImageDiff` / `buildDataUrl`）を、参照をそのまま表示する形へ置き換える。

### threshold の定義

- large file（サイズ） / hunk 数 / tokenization の各 threshold を定義する。概値は large file > 約 1MB または > 約 5,000 行 / hunk 数 > 約 300 / tokenization > 約 10 万文字（または約 5,000 行）とし、design で最終調整する。
- 各 threshold 超過時の挙動（fallback 表示、`limited` フラグ）を定義する。

### frontend の責務縮小

- frontend の direct FS read（working tree / Git index 直接読み）を削除する。
- diff 基準選択 / tree 化 / image base64 化 / patch generation 準備など、現状 frontend が持つ orchestration を Rust read model 側へ移す。
- ファイル一覧表示（ReviewSnapshot）とファイル表示（ReviewFileView）の呼び出しを分離する。
- 対象 hook（`useFileDiffContent` / `useImageDiff` / `useGitOriginalContent` / `useDiffFileTree` / `useGitStatus` のレビュー経路）は新 API を表示用途で利用する形に再構成する。

## 非スコープ

- `RepositoryStateService` 本体（watcher / scan 集約 / versioned snapshot / debounce / cancel-supersede / ignored 扱い）の新規実装。これは #1210 の範囲であり、本 Issue は既存基盤の利用に徹する。
- hunk 単位の stage / unstage 操作の id 化（`stage_hunk_by_id` / `unstage_hunk_by_id`）。これは #1212（M1 の別項目）の範囲。
- diff 基準語彙 `DiffBase` / 区画語彙 `DiffSection` の意味・選択肢そのものの変更。
- レビューコメント機能（`useDiffComments` 等）、検索（`useDiffSearch`）、トークン化表示そのものの仕様変更（tokenization は threshold 適用の対象としてのみ扱う）。
- WebSocket（remote_access）経路への ReviewSnapshot / ReviewFileView の展開。
- Monaco Editor の diff 表示方式・再マウント方針（`key={filePath}`）などレンダリング基盤の変更。
- 性能数値の目標値設定そのもの（計測は #1209 / M0 の範囲）。

## 要求事項

### R1. ファイル一覧 API（ReviewSnapshot）の追加

- `get_review_snapshot(worktree_path, base)` を Rust の Tauri コマンドとして追加すること。
- 戻り値は、現状 `useGitStatus` + `useDiffFileTree` + `get_status_diff_stats` が分散生成しているファイル一覧 read model（変更ファイル集合 / status / diff stats / diff file tree）を含み、1 回の呼び出しで取得できること。
- read model は `RepositoryStateService` の同一 versioned snapshot 由来とし、個別 command / 個別走査での都度生成に依存しないこと。
- 各ファイルに ReviewFileView から参照可能な安定 `file_id` を含めること。
- version および `stale` / `loading` / `limited` 相当の状態を表現できること。

### R2. ファイル表示 API（ReviewFileView）の追加

- `get_review_file_view(file_id | path, section, base, viewport?)` を Rust の Tauri コマンドとして追加すること。
- `file_id` と `path` のいずれでも対象ファイルを特定できること。
- Rust 側で表示種別（text diff / image / binary / large-file fallback）を決定し、種別に応じた read model を返すこと。
- text diff では original / modified / source を Rust が決定し、frontend がそのまま表示できる形で返すこと。
- `viewport?` を任意に受け取り、指定時は必要範囲のみを返せること。

### R3. ファイル一覧と表示の分離

- ファイル一覧の取得（R1）とファイルを開く処理（R2）が、別々の API として分離されていること（Issue 受け入れ基準）。

### R4. frontend から Git orchestration / file IO を排除する

- frontend は ReviewSnapshot / ReviewFileView を表示するだけになり、Git orchestration と file IO（working tree / Git index の direct read、diff 基準選択、tree 化、image base64 化、patch generation 準備）を持たないこと（Issue 受け入れ基準・rust-first-logic 準拠）。
- 対象の direct FS read 経路（`useFileDiffContent` / `useImageDiff` / `useGitOriginalContent` / `useDiffFileTree` のレビュー経路）が、新 API の表示利用へ置き換わっていること。

### R5. image / binary を非 base64 の参照で返す

- image / binary は base64 data URL ではなく、Tauri asset / resource URL（asset protocol 経由の URL 参照）として返すこと。
- frontend 側の base64→data URL 変換経路を廃し、返された URL をそのまま表示に用いること。

### R6. large file / 多数 hunk / tokenization の threshold 定義と fallback

- large file（サイズ） / hunk 数 / tokenization の各 threshold を定義すること。
- threshold 超過時は UI を固めず、fallback 表示になること（Issue 受け入れ基準）。
- 超過判定・打ち切りは Rust 側で行い、その状態を read model（`limited` 等）で frontend に伝えること。

## 受け入れ基準の概要

- file list 表示（ReviewSnapshot）と file open（ReviewFileView）の API が分離されている。
- frontend は ReviewSnapshot / ReviewFileView を表示するだけで、Git orchestration と file IO を持たない（direct FS read が削除されている）。
- large file は UI を固めず fallback 表示になる。
- image / binary が base64 data URL ではなく blob ref / temp URL / resource URL で返される。
- large file / many hunks / tokenization の threshold が定義され、超過時に fallback として扱われる。
- ファイル一覧 read model（status / diff stats / diff file tree）が `RepositoryStateService` の同一 versioned snapshot から ReviewSnapshot 経由で取得される。

## Open Questions

なし（image / binary 返却方式は Tauri asset / resource URL に確定、threshold 概値も確定済み。残る詳細は design で詰める）。
