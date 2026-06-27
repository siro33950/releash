# Requirements

対象 Issue: #1248「Agent context epoch と instruction resolution policy を導入する」

関連: #1190（session resume / reinject）/ #1213 / #1217 / #767（フロントロジックの Rust 移行）

マイルストーン: 性能・メモリ効率改善（Workbench State / Read Model）

## Type

基盤導入（Agent へ投入する context の鮮度管理モデルと instruction 解決方針の確立、およびその Rust 実装）。

## 背景と目的

### 背景

- #1190 で session resume / reinject による「会話コンテキストの復帰」は扱っているが、Agent に投入する **repo / diff / open editor / workflow / instructions** といった周辺文脈（system context）の**鮮度管理**はまだ独立した方針になっていない。
- 現状調査（仮定を含む。詳細は behavior / design で確定）:
  - `context_restore.rs` の `ContextRestorePlan`（Resume / Reinject / NoContext）は、**会話メッセージ**の復帰戦略を判定するもので、repo / diff / instruction 等の system context の鮮度は対象外。
  - `WorkflowStepContextDto` は型として存在するが、ChatSession への値は実質未設定（None）で、workflow run / step state が Agent context に伝播していない。
  - instruction（`AGENTS.md` / `CLAUDE.md` 相当）の探索・解決・重複回避ルールは明文化されておらず、workflow facet 側で instruction を user_message に直結する実装に留まる。
  - context source（repo summary / diff / mentions / terminal log / workflow state / instructions）を「どの単位で、どの版（revision）として保持し、いつ差し替えるか」という共通の保持単位が存在しない。
- OpenCode は `SystemContext.Source<A>`（`key` / `codec` / `load` / `baseline` / `update` / `removed`）と `SessionContextEpoch`（`prepare` / `initialize` / `requestReplacement` / `current`）で context baseline / snapshot / revision / replacement_seq を管理し、instruction 解決（`AGENTS.md` / `CLAUDE.md` / `CONTEXT.md` と read file 近傍 instruction）を行っている。

### 改善する failure mode

本要求は以下の失敗パターンの解消を目的とする。

1. **復帰後の stale 文脈**: 復帰後、UI 上は続きに見えるが、Agent が古い repo 状態や古い system prompt で応答する。
2. **backend/model 切替後の残留 instruction**: model / backend 切替後も前 backend 向けの instruction が system context に残る。
3. **instruction の重複投入による context bloat**: `AGENTS.md` / `CLAUDE.md` / workflow facet instruction が重複して投入され、context が肥大化する。
4. **局所ルールの欠落**: read した file 近傍の project instruction が投入されず、ディレクトリ局所のルールを外す。

### 目的

Releash の Agent context について、

- **context source の列挙**（何が文脈になるか）、
- **保持単位（snapshot / epoch / revision）の定義**（どの単位で保持・差し替えるか）、
- **context replacement ルールの定義**（backend / model / worktree / instruction file 変更時に何を破棄・再構築するか）、
- **instruction 解決と重複回避ルールの Rust 側集約**（`AGENTS.md` 相当の探索範囲と重複防止）

を確立し、上記 4 つの failure mode を構造的に防げる状態にする。

## スコープ

本 Issue の受け入れ基準は「`ContextSource` / `ContextSnapshot` / `ContextEpoch` 相当の**型または設計 doc** がある」「`AGENTS.md` 相当の探索範囲と重複回避ルールが**明文化**されている」と定義されている。これを踏まえ、本要求は **設計の確立に加え、その Rust 実装まで** をスコープとする（ユーザー確認済み）。

### 設計成果物

- **D1. context source の列挙と定義**: Agent に投入しうる context source を列挙し、それぞれの意味・取得元・更新契機を定義する。少なくとも以下を含む。
  - repo summary
  - diff / review snapshot
  - open editor / selection
  - mentions
  - terminal log summary
  - workflow run / step state
  - `AGENTS.md` 相当の project instructions
- **D2. 保持単位の定義**: context snapshot / epoch / revision の保持単位（何を 1 単位とし、どの粒度で版を持つか）を定義する。`ContextSource` / `ContextSnapshot` / `ContextEpoch` 相当の **Rust 型** として表現する。
- **D3. replacement ルールの定義**: agent backend / model / worktree / instruction file の変更時に、どの context を破棄・再構築・据え置くかの replacement ルールを定義する。
- **D4. instruction 解決方針の定義**: `AGENTS.md` 相当（`AGENTS.md` / `CLAUDE.md` 等）の **探索範囲**（リポジトリ階層・read file 近傍）と **重複回避ルール** を明文化する。
- **D5. #1190 との整合**: 上記モデルが #1190 の native resume / reinject fallback と矛盾しないことを設計上担保する（会話コンテキスト復帰と system context 鮮度管理の責務境界を明確化する）。

### 実装成果物

- **D6. 型・解決ロジックの Rust 実装**: D1〜D4 で定義した型・解決ロジックの Rust 実装（context source の保持、snapshot / epoch 保持、replacement 適用、instruction 解決の実コード）。
  - 各 context source の取得処理（repo summary 生成・diff 取得・terminal log 要約等）のアルゴリズム自体の新規実装・高度化は本要求の対象外とし、既存の取得結果を上記モデルの保持単位・replacement ルールに載せることを実装範囲とする。
- **D7. instruction 解決ロジックの Rust 集約**: D4 の探索・重複回避ルールを Rust usecase 側に実装する。
- **D8. frontend 入力受け渡しの整合**: frontend は context 構築ロジックを持たず、必要な生入力（編集中ファイル・選択範囲・mentions 等）を Rust usecase に渡すだけに留める構成へ調整する。

## 非スコープ

- 会話メッセージ履歴そのものの復帰仕様（resume / reinject による会話コンテキスト引き継ぎ）の再設計。これは #1190 の範囲とし、本要求はそれと整合する system context 鮮度管理を扱う。
- 会話コンテキストの要約・圧縮・トークン上限に基づくトリミング機能の新設。
- Agent SDK / Agent CLI そのものの resume / system prompt 仕様の変更。
- Claude / Codex 以外の新しい Agent backend への対応追加。
- メッセージ履歴やエディタの表示 UI そのものの仕様変更。
- context source の各取得処理（repo summary 生成・diff 取得・terminal log 要約等）のアルゴリズム自体の高度化。本要求は「どの source をどの保持単位・どの replacement ルールで扱うか」の枠組みを定義する（取得処理の新規実装有無は Q1 に従う）。

## 要求事項

### R1. context source の列挙と定義

- Agent に投入しうる context source を列挙し、各 source について「意味」「取得元」「更新（再取得）契機」を定義すること。
- 列挙には少なくとも repo summary / diff・review snapshot / open editor・selection / mentions / terminal log summary / workflow run・step state / project instructions（`AGENTS.md` 相当）を含むこと。

### R2. 保持単位（snapshot / epoch / revision）の定義

- context snapshot / epoch / revision の保持単位を定義し、`ContextSource` / `ContextSnapshot` / `ContextEpoch` 相当の **Rust 型または設計 doc** として表現すること。
- 各 context が「どの epoch / revision に属するか」を識別でき、stale な context を判別できる構造であること（failure mode 1 への対処）。

### R3. context replacement ルールの定義

- agent backend / model / worktree / instruction file の変更時に、どの context を破棄・再構築・据え置くかを定義すること。
- backend / model 切替時に、前 backend / 前 model 向けの instruction・system prompt が残留しないこと（failure mode 2 への対処）。

### R4. instruction 解決と重複回避ルール

- `AGENTS.md` 相当（`AGENTS.md` / `CLAUDE.md` 等）の **探索範囲**（リポジトリ階層を辿る範囲、および read した file 近傍の局所 instruction）を明文化すること。
- 同一 instruction が複数経路（リポジトリ instruction / workflow facet instruction / read file 近傍 instruction）から重複投入されないための **重複回避ルール** を明文化すること（failure mode 3 への対処）。
- read した file 近傍の project instruction が投入される方針であること（failure mode 4 への対処）。
- 上記 instruction 解決ロジックは Rust 側に集約する方針であること。

### R5. #1190 との整合と責務分離

- 本モデルが #1190 の native resume / reinject fallback と矛盾しないこと。
- 「会話コンテキスト（メッセージ履歴）の復帰」（#1190）と「system context（repo / diff / instruction 等）の鮮度管理」（本要求）の責務境界が明確であること。

### R6. ロジック配置

- context 構築・instruction 解決・epoch / replacement 判定のロジックは Rust（Tauri バックエンド）側に置くこと（`.claude/rules/rust-first-logic.md` に従う）。
- frontend は context 構築ロジックを持たず、必要な生入力（編集中ファイル・選択範囲・mentions 等）を Rust usecase に渡すだけであること。

### R7. Rust 実装

- R1〜R4 で定義した型・解決ルール・replacement ルールを Rust 側に実装すること。
- 各 context source の取得処理（repo summary 生成・diff 取得・terminal log 要約等）のアルゴリズム自体の新規実装・高度化は対象外とし、既存の取得結果を本モデルの保持単位・replacement ルールに載せること。
- 実装には正常系・エラー系の双方を含むテストを伴うこと。

## 受け入れ基準の概要

Issue 記載の受け入れ基準に対応する。

- **AC1**: `ContextSource` / `ContextSnapshot` / `ContextEpoch` 相当の Rust 型が存在し、実装されている（R1 / R2 / R7）。
- **AC2**: `AGENTS.md` 相当の探索範囲と重複回避ルールが明文化され、その解決ロジックが Rust 側に実装されている（R4 / R7）。
- **AC3**: 本モデルが #1190 の native resume / reinject fallback と矛盾しない（R5）。
- **AC4**: frontend は context 構築ロジックを持たず、Rust usecase に入力を渡すだけである（R6）。
- **AC5**: 列挙された context source が backend / model / worktree / instruction file 変更時の replacement ルールを持ち、stale context・残留 instruction・重複投入を構造的に防げる（R2 / R3 / R4、failure mode 1〜4 に対応）。

## 仮定

- **A1**: 本 Issue の成果物は、「context source / snapshot / epoch モデルの型・設計」と「instruction 解決ルールの明文化」に加え、それらの **Rust 実装** を含む（ユーザー確認済み）。ただし各 context source の取得アルゴリズム自体の新規実装・高度化は含まない（R7）。
- **A2**: spec ディレクトリ名は新しい命名規約に合わせ `docs/specs/issues-1248` とする。
- **A3**: 「会話コンテキストの復帰」は #1190 の責務とし、本要求はそれと整合する system context（repo / diff / instruction 等）の鮮度管理に限定する。両者の重なり（例: instruction の再注入と会話再注入の関係）は design 検討時に切り分ける。
- **A4**: instruction の対象ファイルは少なくとも `AGENTS.md` と `CLAUDE.md` を含む。`CONTEXT.md` 等の追加対象は OpenCode 参照を踏まえ design で確定する。
- **A5**: context source の列挙は Issue 記載の 7 種を最小集合とし、過不足は design 検討時に調整しうる。

## Open Questions

なし（Q1: 実装範囲は「Rust 実装まで含む」で確定）。
