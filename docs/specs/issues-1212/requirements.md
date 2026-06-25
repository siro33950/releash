# Requirements

Issue: #1212 「Hunk operation を id-based にし frontend patch 再生成を削除する」

関連: #1211（ReviewSnapshot / ReviewFileView の追加・本 Issue の直接の前提・マージ済み commit `efdb90cf`）/ #1210（RepositoryStateService / versioned snapshot 基盤）/ #767 / #805 / `docs/releash-performance-architecture-audit.md` M1（正本ドキュメント, commit `b0c5e4c2`, マイルストーン: 性能・メモリ効率改善, 実装順 4-3）

## Type

リファクタリング / 性能・メモリ効率改善。マイルストーン M1「Git / Diff hot path を Rust read model に寄せる」の項目 4（`stage_hunk_by_id` / `unstage_hunk_by_id` を追加し、frontend から full content と patch generation を外す）。Issue コメントの実装順「4-3」に位置づけられ、#1211 で stable な file view / hunk が返るようになったことを前提に、stage / unstage を **id-based operation** へ変える仕上げ。

## 背景と目的

`docs/releash-performance-architecture-audit.md` M1 の結論にあるとおり、レビュー（diff 表示）経路の問題は「frontend が working tree を直接読み、diff 基準選択 / tree 化 / image base64 化 / patch generation 準備まで持っている」設計の広がりにある。本 Issue は、その最後に残る **hunk 単位 stage / unstage の patch 生成経路** を id-based に整理する。

#1211 のマージにより、現状は既に次の状態にある（実装確認済み）。

- frontend の full content based な patch 文字列生成（`generatePatch` 等）は撤去済みで、`src/lib/` に patch 生成関数は存在しない。
- hunk / group 単位の stage / unstage は `useDiffOperations` 経由で Rust コマンド `git_stage_review_group` / `git_unstage_review_group` を呼ぶ。frontend が渡すのは `worktreePath` / `path` / `section` / `base` / `groupIndex` / `snapshotVersion` のみ。
- Rust 側（`review_usecase::generate_review_group_patch`）が versioned snapshot を基に original / modified を select し、`compute_diff_hunks` で hunk / change group を再計算して patch を組み立て、`git apply --cached`（`git_stage_hunk` / `git_unstage_hunk`）で適用する。

しかし、対象 hunk / group の指定が **位置依存の `group_index`（`ChangeGroupDto.group_index`）** であり、整合性は **`snapshot_version` ガード**（`ensure_current_review_snapshot_version`）に依存している。このため次の課題が残る。

- `group_index` は snapshot を再計算するたびに意味が変わりうる位置 index であり、file refresh をまたいで安定しない。
- frontend が version を持ち回り、snapshot が更新されると in-flight / 連続操作が version 不一致で弾かれる（Issue 受け入れ基準「file refresh 後も stable id の扱いが明確」を満たさない）。

本 Issue の目的は、ReviewFileView / ReviewSnapshot が **安定した `hunk_id` / `group_id`** を返すようにし、stage / unstage を **id + intent だけ**を渡す id-based operation（`stage_hunk_by_id` / `unstage_hunk_by_id`）に置き換え、Rust 側が staged をベースに patch を再構築する構成へ収束させること。これにより file refresh をまたいだ stable id の扱いを明確にし、frontend から patch generation の痕跡（位置・version 持ち回り）を完全に外す。

## 合意済みの前提

- 本 Issue は #1211（ReviewSnapshot / ReviewFileView）と #1210（RepositoryStateService / versioned snapshot）が先行して存在することを前提とする（Issue コメント「順序 4-1 → 4-2 → 4-3」、#1211 はマージ済み）。
- 対象は **デスクトップアプリのレビュー（diff 表示）経路**であり、ロジックは Rust（Tauri コマンド）に実装し frontend は表示・入力受付・invoke に徹する（`.claude/rules/rust-first-logic.md`）。
- 既存の diff 基準語彙 `DiffBase`（`branch-base` | `head`）と区画語彙 `DiffSection`（`changes` | `staged`）を踏襲し、語彙そのものは変更しない。
- group / hunk 単位の stage / unstage は `branch-base` では提供しない（現状 `generate_review_group_patch` が branch-base を拒否しているのを踏襲）。`head` ベースの `changes` / `staged` 区画のみ対象。
- patch の適用は `git apply --cached` を用い、patch のベースは Staged 状態とする（`CLAUDE.md` の既知制約・現行 `git_stage_hunk` 実装を踏襲）。

## スコープ

### S1. ReviewFileView / ReviewSnapshot の read model に安定 id を追加する

- text diff の read model（`ReviewTextDiffDto` の `hunks` / `change_groups`）に、対象 hunk / change group を一意に指す安定 id（`hunk_id` / `group_id`）を含める。
- id は、同一内容の hunk / group を指す限り file refresh（snapshot 再計算）をまたいで安定する値とする。生成規則の詳細（内容ハッシュか、ファイル内オフセット由来か等）は design で確定する（仮定 A1）。
- 既存の位置 index（`HunkDto.index` / `ChangeGroupDto.group_index` / `hunk_index`）は表示・整列用途として残してよいが、stage / unstage の対象特定には id を用いる。

### S2. id-based の stage / unstage コマンドを追加する

- 対象 hunk / change group を **id + intent（stage / unstage）** で指定する id-based operation を Rust の Tauri コマンドとして提供する。Issue 記載の `stage_hunk_by_id` / `unstage_hunk_by_id` に相当する。
- 既存の `git_stage_review_group` / `git_unstage_review_group`（`group_index` + `snapshot_version` ベース）からの移行方法（置き換えか追加か、コマンド命名）は design で確定する（仮定 A2）。
- frontend からは worktree / path / section / base / 対象 id / intent のみを渡す。位置 index と `snapshot_version` の持ち回りをやめる（決定事項 D1）。

### S3. Rust 側で staged をベースに patch を再構築する

- 対象 id から、現在の snapshot 上の change group を解決し、original / modified を select して patch を再構築する。
- patch の適用ベースは Staged 状態とし、`git apply --cached` のベース不一致を起こさない（既知制約の踏襲）。
- `snapshot_version` ガード（`ensure_current_review_snapshot_version`）は廃止し、整合性は対象 id を現在の snapshot 上の change group に解決できるか否かで担保する（決定事項 D1）。
- 対象 id が現在の snapshot 上に解決できない（内容が変わって該当 group が消失した等）場合は、誤った範囲を stage / unstage せず **エラーを返す**。frontend はそれを捕捉し snapshot を refresh する（または再取得を促す）（決定事項 D2）。

### S4. frontend から patch generation の痕跡を外す

- frontend は id-based コマンドを invoke するだけにし、位置 index / `snapshot_version` の持ち回りを `useDiffOperations` から除去する。
- id 解決失敗のエラーを `useDiffOperations`（または呼び出し元）で捕捉し、snapshot を refresh する（または再取得を促す）ハンドリングを追加する。現状の `catch (e) { console.error }` による握りつぶしのままにしない（決定事項 D2）。
- `useGitActions` に残る patch 文字列を受け取る旧経路（`git_stage_hunk` / `git_unstage_hunk` を patch 引数で叩く `stageHunk` / `unstageHunk`）が未使用であれば撤去対象とする（design で死コード確認の上、本 Issue で除去するかを確定）。

## 非スコープ

- ReviewSnapshot / ReviewFileView 本体の追加（#1211 の範囲。本 Issue は既存 read model への id 付与と stage / unstage 経路の変更に限定）。
- RepositoryStateService 本体（watcher / scan 集約 / versioned snapshot / debounce / cancel-supersede）の変更（#1210 の範囲）。
- `DiffBase` / `DiffSection` の意味・選択肢そのものの変更。
- `branch-base` での hunk / group 単位 stage / unstage の新規提供。
- ファイル単位の stage / unstage（`git_stage` / `git_unstage`）の変更。
- image / binary / large-file fallback の表示方式（#1211 の範囲）。
- レビューコメント機能、検索、tokenization 表示そのものの仕様変更。
- WebSocket（remote_access）経路への id-based stage / unstage の展開。
- 性能数値の目標値設定・計測（#1209 / M0 の範囲）。

## 要求事項

### R1. 安定 id の付与

- text diff の read model が、各 hunk / change group に対し file refresh をまたいで安定する id（`hunk_id` / `group_id`）を含むこと。
- id は同一内容の hunk / group を指す限り、snapshot の再計算後も同じ値を返すこと。

### R2. id-based stage / unstage コマンド

- 対象を id + intent で指定して stage / unstage を行う Tauri コマンドを提供すること（`stage_hunk_by_id` / `unstage_hunk_by_id` 相当）。
- frontend が渡す対象指定は安定 id とし、位置 index および `snapshot_version` による指定に依存しないこと（D1）。
- 操作粒度は現行 UI の change group 単位を維持すること。hunk 全体単位の新規 stage 操作は追加しないこと（D3）。

### R3. frontend の patch generation 排除

- frontend が full content based の patch 文字列を生成しないこと（#1211 で既に達成済みの状態を維持し、本 Issue で id-based へ移行後も退行させないこと）。
- frontend が stage / unstage のために渡すのは id + intent（および worktree / path / section / base）に限られること。

### R4. staged ベースでの patch 再構築

- Rust 側が対象 id から現在の staged 状態をベースに patch を再構築し、`git apply --cached` のベース不一致を起こさないこと。
- 対象 id が現在の snapshot 上に存在しない場合に、誤った範囲を stage / unstage しないこと。

### R5. file refresh 後の stable id の扱いの明確化

- file refresh（snapshot 更新）後に、過去に払い出した id がどう扱われるか（同一内容なら有効、消失なら安全に失敗）が定義され、テストで担保されていること。

## 受け入れ基準の概要

- frontend の full content based patch generation が存在せず、stage / unstage が id + intent のみで実行できる（Issue 受け入れ基準）。
- `git apply --cached` のベース不一致を避けることを確認するテストがある（Issue 受け入れ基準）。
- file refresh 後の stable id の扱い（同一内容なら有効 / 消失時はエラー）がテストで担保されている（Issue 受け入れ基準、D2）。
- `snapshot_version` ガードが廃止され、frontend が version を渡さずに stage / unstage できる（D1）。
- id 解決失敗時に Rust がエラーを返し、frontend が refresh して回復できる（D2）。
- 既存の group 単位 stage / unstage の振る舞い（`changes` で stage、`staged` で unstage、`branch-base` 非対応、操作粒度は change group 単位）が id-based でも維持される（D3）。

## 仮定

task とリポジトリ内の情報から置いた仮定。design で確定すべきものは Open Questions に分離する。

- **A1**: 安定 id は、内容ハッシュ等によりファイル内の hunk / group 内容に紐づく値とし、snapshot 再計算後も同一内容なら同じ id を返す。生成規則の詳細（内容ハッシュかオフセット由来か等）は design で確定する。
- **A2**: 新コマンドは Issue 記載の `stage_hunk_by_id` / `unstage_hunk_by_id` を基本名とし、既存の `git_stage_review_group` / `git_unstage_review_group` を id-based に置き換える（並存させない）。コマンド命名・移行手順の詳細は design で確定する。

## 決定事項

Open Questions は人間との確認により以下のとおり解消済み。

- **D1**: id-based 化に伴い `snapshot_version` ガード（`ensure_current_review_snapshot_version`）を**完全に廃止**する。整合性は対象 id を現在の snapshot 上の change group に解決できるか否かで担保し、frontend は version を一切持ち回らない（id + intent のみ）。
- **D2**: 対象 id が現在の snapshot 上に解決できない場合は、誤った範囲を適用せず **Rust がエラーを返す**。frontend はそのエラーを捕捉し、snapshot を refresh する（または再取得を促す）ハンドリングを本 Issue のスコープに含める。現状の握りつぶし（`catch` で `console.error` のみ）のままにしない。
- **D3**: stage / unstage の操作粒度は現行 UI の **change group 単位を維持**する。`hunk_id` と `group_id` の双方に安定 id を払い出すが、hunk 全体単位の新規 stage 操作は追加しない。

## Open Questions

なし（D1〜D3 で解消済み）。
