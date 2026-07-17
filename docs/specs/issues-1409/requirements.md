# L8: persist 失敗の可視化と event log 自己修復（#1409）

## 背景と目的

Agent チャット（Claude / Codex セッション）の永続化経路には、失敗が無言で握りつぶされ、in-memory 状態と durable 状態が乖離したまま処理が進行する問題がある。milestone 84「Agent チャット安定化」の Phase 0（依存なし）として、監査で確定した以下 3 件を解消する。

- **ST-4**: 永続化失敗が `let _ =` で握りつぶされ、in-memory と durable が乖離したまま進行する。RT-8 等の症状の残存パターン。
- **RT-4**: event log への append がクラッシュで欠けた `]` を、append 側が自己修復しない。以後そのセッションの全 append（`TurnStarted` / `SessionClosed` / `PermissionResolved` 等）が恒久的に失敗し、当該チャットではメッセージ送信が二度と成功しなくなる。
- **RT-8**: `FinalPartsRecorded` の append 失敗時に、projection 由来の（Text/Thinking を欠いた）tool-only parts で persist 済みの本文を上書きし得る。turn 完了の瞬間に本文がツール履歴だけのメッセージへ置き換わり、reload しても戻らない。

目的は、これらの経路で発生する永続化失敗を **無言にせず**（リトライ → 継続失敗は可視化＋エラー伝搬）、破損した event log を **自己修復して送信可能な状態へ戻し**、失敗時に **persist 済みの本文を失わない** ことである。

### 正本ドキュメント

- 問題詳細: `specs/milestone-84-agent-chat-stabilization/agent-chat-instability-audit.md`（ST-4: L1120〜, RT-4: L701〜, RT-8: L765〜）
- ライフサイクル: `specs/milestone-84-agent-chat-stabilization/agent-chat-ideal-lifecycle.md` I8（状態変更の ack 駆動と失敗の可視化、`PersistFailure` の transient 例外）

## スコープ

1. **`let _ =` による persist 失敗の握りつぶし全廃（ST-4）**
   - `src/usecase/agent_session/` 配下を `rg '^\s*let _ = '` で棚卸しし、**永続化（session_store / event append / state 保存）の失敗を握りつぶしている production 経路**を対象に、次の統一挙動へ置換する。
     - 短い backoff で N 回リトライする。
     - 継続失敗時は呼び出し元へエラーを伝搬し、ユーザーへ可視化する。
   - 現行コードで対象となる production 経路（監査記載の行は現状コードで移動済み）:
     - `runtime/usecase.rs:3378` — queued turn の runtime 再オープン失敗時の `set_session_state(Error)`。
     - `runtime/usecase.rs:3534` — `start_turn` 失敗時の `TurnInterrupted` append（`append_session_event_and_project_state`）。
   - 可視化は、本 ISSUE 時点では **session-scoped バナー**（durable-first 原則の唯一の例外として transient 表示を許容）＋ **構造化ログ** で実装する。S5（#1393）で `Notice(PersistFailure)` に置換される前提で、そのための接合点を残す。

2. **event log の append 側自己修復（RT-4）**
   - event log 読み込み／追記経路（`adaptor/gateway/agent_session/session_storage/event_store.rs`）で、末尾破損（欠け `]`・中途行）を検出し修復する。
   - 読み取り側の `recover_unclosed_session_events`（既存）に対し、**append 側でも修復してから追記を継続できる**ようにし、修復後は通常どおり append 可能に戻ること。
   - 修復した事実を構造化ログ＋可視化（session バナー、将来は Notice）に残す。

3. **`FinalPartsRecorded` append 失敗時の上書き防止（RT-8）**
   - `complete_turn` の最終永続化経路で、`FinalPartsRecorded` の append に失敗した場合に、projection 由来の tool-only parts（Text/Thinking 欠落）で persist 済み本文を完全置換する経路を止める。
   - durable 追記に失敗した場合は **persist 済み本文を保持したまま再試行** する挙動へ変更する。

4. **テスト**
   - 破損 event log fixture、および persist 失敗注入（既存の `test_support` のエラー注入を利用）で、無言乖離・本文喪失が起きないことを固定する。

## 非スコープ

- **RT-7 のキュー停止問題そのものの解消**（再オープン失敗後の自動再試行トリガー欠如、キュー未 pop）。本 ISSUE では `runtime/usecase.rs:3378` の **persist 失敗の握りつぶし解消（可視化・伝搬）** のみを扱い、キュー回収挙動の再設計は行わない。
- **`Notice(PersistFailure)` 語彙の正式導入**。これは S5（#1393）の担当であり、本 ISSUE は暫定の session バナー＋構造化ログに留める。
- **projection 内の非 persist な `let _ =`**（例: `event_log/projector.rs:632` の `apply_tool_result_update` の戻り値無視）。永続化失敗ではないため対象外。
- **テストコード内の `let _ =`**（例: `runtime/usecase.rs:4792` の test 用 `ReentrantWorkflowNotifier`）。production 経路ではないため対象外。
- RT-5 / RT-6 など、milestone 84 の別 ISSUE が担当する監査項目。
- frontend の恒久的なエラー表示・チャット内 Error block 描画の追加（FE-2 等、別 ISSUE 担当）。本 ISSUE の可視化は session-scoped バナー＋ログに限定する。

## 要求事項

### R1: persist 失敗を無言にしない（ST-4）

- 永続化失敗が発生した production 経路で、失敗を握りつぶさず、リトライ（短い backoff で N 回）を行う。
- リトライ後も失敗が継続する場合、失敗を呼び出し元へエラーとして伝搬し、当該操作をエラー化する。
- 失敗はユーザーに可視化する（session-scoped バナー）とともに、構造化ログへ記録する。
- 棚卸しの結果、`src/usecase/agent_session/` 配下の production 永続化経路に `let _ =` による persist 失敗の握りつぶしが 0 件になる。

### R2: event log の自己修復（RT-4）

- 末尾破損（欠け `]`・中途行）した event log を持つセッションでも、読み込み・追記を再開できる。
- append 側で修復を行い、修復後は以降の append（送信を含む）が通常どおり成功する。
- 修復が行われた事実をログ＋可視化に残す。

### R3: 本文の上書き防止（RT-8）

- `FinalPartsRecorded` の append に失敗した場合でも、persist 済みの本文（Text/Thinking を含む parts）を tool-only parts で上書きしない。
- durable 追記に失敗したときは persist 済み本文を保持し、再試行する。

### R4: テストによる固定

- 破損 event log fixture で、当該セッションが自己修復され送信可能になることを固定する。
- persist 失敗注入で、失敗が無言にならず（可視化＋リトライ）、本文が tool 履歴だけのメッセージへ置換されないことを固定する。

## 受け入れ基準の概要

- [ ] 破損した event log のセッションが自己修復され、メッセージ送信が再び成功する。
- [ ] persist 失敗が無言にならない（session バナーでの可視化＋リトライ＋継続失敗時のエラー伝搬）。
- [ ] turn 完了時に本文が tool 履歴だけのメッセージへ置き換わる事象が再現しない。
- [ ] `src/usecase/agent_session/` 配下 production 経路で `let _ =` による永続化失敗の握りつぶしが 0 件。
- [ ] 上記を固定する unit / 統合テストが追加され、CI（`cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test`）が通る。

## 仮定

- **A1**: spec-id は既存命名慣習に合わせ `docs/specs/issues-1409/` とする。
- **A2**: 可視化は本 ISSUE 時点では session-scoped バナー（transient 許容の唯一の例外）＋構造化ログで実装し、`Notice(PersistFailure)` への置換は S5（#1393）で行う。バナーは backend-owned state から供給し、frontend は表示に徹する（Rust-first 原則）。
- **A3**: 監査文書が参照する行番号（usecase.rs:3366 / :3522 / :4755 等）は現行コードで移動しており、実装時は行番号ではなく `rg '^\s*let _ = '` による棚卸し結果を正とする。現時点の production 対象は `runtime/usecase.rs:3378` と `:3534` の 2 箇所。
- **A4**: リトライ回数 N と backoff 間隔の具体値は design node で確定する。要求段階では「短い backoff で有限回リトライ後にエラー伝搬」という挙動のみを固定する。
- **A5**: RT-8 の対象は tool 使用等の durable part を含む turn で、`FinalPartsRecorded` の append だけが mid-turn 失敗するケース（監査記載の発生条件）を主とする。純テキスト turn は live parts フォールバックで無傷のため対象外。
- **A6**: エラー注入はテスト専用の既存 `test_support` 機構を利用し、外部プロセス・実 I/O 障害はテストで発生させない（プロジェクトのテスト方針に従う）。

## Open Questions

なし
