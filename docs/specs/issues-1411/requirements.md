# Requirements

関連: #1411（milestone 84「Agentチャット安定化」／ Phase 0 ／ L10）

正本参照:
- 問題インベントリ: `specs/milestone-84-agent-chat-stabilization/agent-chat-instability-audit.md` の **ST-5**（session lock の二段取得と prune skip）
- ライフサイクル理想形: `specs/milestone-84-agent-chat-stabilization/agent-chat-ideal-lifecycle.md` の **I13**（排他とロックの規約）

## Type

構造的安定化（backend／Rust 内部）。session runtime lock の保持規約を明文化し、既存経路の是正、prune の確実化、テストビルドでの再入検出を行う。外部から観測可能な UI/CLI の振る舞いは追加しない。

## 背景と目的

Agent session の runtime 処理は、session ごとの排他を `acquire_session_runtime_lock`（`src-tauri/src/usecase/agent_session/runtime/usecase.rs:2153`）で行う。この実装には次の構造的問題がある（ST-5、重大度 medium）。

1. **二段ロックの規約未整備**: locks map の `Mutex<HashMap<..>>`（`SessionRuntimeLocks`）を取得して per-session の `Arc<Mutex<()>>` を取り出し、その `lock_owned()` を取る二段構造になっている。lock 保持中に許される await の範囲（別 session の lock 取得可否、backend I/O の await 範囲、emit のタイミング）が文書化も検査もされておらず、将来 lock 保持中に別 session の操作を await する経路が追加されると deadlock し得る温床になっている（現時点で deadlock の実績確認は無し）。

2. **prune skip による lock エントリ蓄積**: `SessionRuntimeLockGuard` の `Drop` 実装が、解放時のエントリ掃除を `tokio::runtime::Handle::try_current()` に依存して `handle.spawn(...)` で行う（`usecase.rs:2172-2196`）。runtime handle を取得できない文脈（テスト・非 tokio スレッド上の Drop 等）では prune を skip し、`session_locks` HashMap にエントリが蓄積し得る。

本変更の目的は、I13「session runtime lock の保持中に、別 session の lock・長時間 await（backend I/O）を行わない。lock の取得順序と保持範囲を規約として明文化し、prune はランタイムハンドル取得に依存しない方式にする」を満たすことである。これにより「チャットが応答しなくなる」系の将来 deadlock を作り込みにくくし、lock エントリの無限蓄積を無くす。

## スコープ

対象は `src-tauri/src/usecase/agent_session/runtime/usecase.rs` の session runtime lock 機構（`SessionRuntimeLocks` / `SessionRuntimeLock` / `acquire_session_runtime_lock` / `SessionRuntimeLockGuard`）と、その保持中に処理を行う既存呼び出し経路である。

1. **lock 規約の明文化（rustdoc）**: `acquire_session_runtime_lock`（および公開 API の `acquire_session_lock`）に、lock 保持中の規約を rustdoc として記述する。少なくとも次を明記する。
   - (a) lock 保持中に別 session の runtime lock を取得しない（取得順序・再入の禁止）。
   - (b) backend I/O（backend への stdin write、process spawn 等）の await は最小範囲に留める。
   - (c) UI/event への emit（`emit_session_state_change` 等の通知）は lock 外で行う。

2. **既存違反の是正または列挙**: lock を保持する既存経路（`usecase.rs` 内の `acquire_session_lock` / `acquire_session_runtime_lock` 呼び出し箇所。現状 `279` / `284` / `438` / `484` / `1826` / `1880` / `2087` / `2259` 付近）を棚卸しし、上記規約 (a)〜(c) の違反を是正する。是正範囲が本 ISSUE のスコープとして過大になる場合は、違反箇所の一覧（ファイル・行・違反種別）を本 ISSUE に追記し、分割 ISSUE として切り出す判断を行う。

3. **prune の確実化（ランタイム非依存化）**: `SessionRuntimeLockGuard` の `Drop` における `Handle::try_current()` 依存を廃止する。解放時に prune 候補を（同期的に取得可能な）キュー等へ登録し、次回 `acquire_session_runtime_lock` 時に未参照（`Arc::strong_count == 1` 相当）のエントリを掃除する方式へ変更する。これにより tokio runtime の有無に関わらず、lock エントリが `session_locks` に無期限蓄積しない。

4. **テストビルドでの再入検出**: lock を保持したまま同一実行フローで別の（または同一 session の）runtime lock を取得する「再入」を、テストビルド限定で検出する仕掛け（task owner keyed な保持 registry + `assert!` 等）を入れる。production ビルドの挙動・性能には影響を与えない。

5. **テスト**: prune がランタイム非依存で確実に行われること（エントリが蓄積しないこと）と、再入検出が働くことを Rust のユニットテストで検証する。

## 非スコープ

- runtime 状態機械そのものの module 分解・大規模リファクタ（ST-3 / L11 #1412 の対象）。本変更は lock 機構の規約化・prune 是正・再入検出に限定する。
- lock 機構を跨いだ排他モデルの再設計（別 session 間で共有する新たなロック導入等）。既存の per-session 排他モデルは維持する。
- `MessagePart` 二重定義など ST-6 以降の別問題。
- frontend への変更。本変更は backend 内部完結とする（rust-first-logic に従う）。
- lock 保持中の await 範囲を静的解析で恒久的に強制する仕組み（本変更は rustdoc 規約 + テストビルド限定の再入検出まで）。

## 要求事項

- R1: `acquire_session_runtime_lock` / `acquire_session_lock` に、lock 保持中の規約 (a) 別 session lock 取得禁止、(b) backend I/O await 最小化、(c) emit はロック外、を rustdoc で明記する。
- R2: lock を保持する既存経路を棚卸しし、R1 の規約違反を是正する。是正が過大な場合は違反一覧を ISSUE に追記し分割判断する。
- R3: `SessionRuntimeLockGuard` の `Drop` から `Handle::try_current()` 依存を除去し、次回 acquire 時に未参照エントリを掃除する方式へ変更する。prune を skip する経路を無くす。
- R4: prune 変更後、`session_locks` HashMap にエントリが無期限蓄積しない（解放済み session の未参照エントリは次回 acquire で除去される）。
- R5: テストビルド限定で lock 再入を検出する仕掛け（task owner keyed 共有 registry + `assert!` 等）を導入する。production ビルドの挙動・性能を変えない。
- R6: R3・R4・R5 を検証する Rust ユニットテストを該当 module に追加する。
- R7: 本変更は backend（Rust）内で完結し、frontend の変更を伴わない。

## 実装時の lock 保持経路棚卸し

- 規約 (a) について、`src-tauri/src/adaptor/gateway/workflow/runtime_session.rs` の `start_fanout_child_sessions` が、単一 task で fan-out の全 child session lock を先に取得して同時保持していた。child ごとの reservation task が自身の lock だけを取得し、全 child の予約後に snapshot / tab を公開して、開始対象の guard を親 activation future へ順に引き渡す構造へ変更した。`start_turn_locked` は親 future 自身が実行するため、cancel acknowledgment 後の decision 待ちでは child start も poll されず静止し、rollback 時は同じ future を安全に再開できる。commit 時は親 future の drop と共有 tracker による全 reservation task の終了確認を終えてから terminal cleanup を行う。これにより単一実行フローでの別 session lock 同時保持を解消しつつ、公開後の外部操作が workflow activation を追い越さない既存の排他順序を維持する。その他の `acquire_session_lock` / `acquire_session_runtime_lock` 呼び出し元には、lock 保持中の追加 acquire は無い。
- 規約 (b) について、lock 保持中に残す backend / workflow notifier の await は次のとおりである。行番号は本 ISSUE 実装時点の行であり、各行の「guard 取得・保持範囲」と実際の await を対応付けている。いずれも同一 session の状態遷移順を守る範囲に限られ、別 session lock は取得しない。これらをさらに lock 外へ分離するには runtime 状態機械と post-action 境界の変更が必要なため、本 ISSUE では変更せず、全件を ST-3 / #1412 と併せた分割対象とする。

  | 経路 | guard 取得・保持範囲 | lock 保持中の await（違反種別 (b)） |
  |---|---|---|
  | `send_message` | `src-tauri/src/usecase/agent_session/runtime/usecase.rs:313-318` で取得し、`:463` または早期 return まで保持 | active turn への backend steer `:334-353`、および `start_turn_for_session` の await `:438-453`（配下の process open / turn start は `:1504-1576`） |
  | `start_session` | `src-tauri/src/usecase/agent_session/runtime/usecase.rs:472-498` | `ensure_runtime` `:497` から process open `:2073-2091` |
  | `respond_permission` | `src-tauri/src/usecase/agent_session/runtime/usecase.rs:518-656` | backend permission response `:531-534`、stall-clear workflow notifier `:587-590`、streaming delta retry `:617-627` |
  | `start_turn_locked`（workflow 共通） | API 本体は `src-tauri/src/usecase/agent_session/runtime/usecase.rs:1146-1215`。production の guard 呼び出し元は `src-tauri/src/adaptor/gateway/workflow/node_session_boundary.rs:220-233`、`src-tauri/src/adaptor/gateway/workflow/runtime_session.rs:623-628,686-713`、`src-tauri/src/adaptor/gateway/workflow/runtime_engine_impl.rs:3644-3654` | `start_turn_for_session` `src-tauri/src/usecase/agent_session/runtime/usecase.rs:1199-1215`（配下の process open / turn start は `:1504-1576`） |
  | stale watchdog | `src-tauri/src/usecase/agent_session/runtime/usecase.rs:1930-2011`（`:1876-1923` の先行 guard 範囲には backend / notifier await 無し） | stall-observed workflow notifier `:2005-2009`。observe/clear の逆転防止のため保持中に直列化する既存契約がある |
  | event pump | `src-tauri/src/usecase/agent_session/runtime/usecase.rs:2137-2140` | `apply_runtime_event` `:2139`（本体 `:2500-2745`）と、その配下の stall-clear workflow notifier `:2665-2669,2864-2868`、stream flush `:2879-2880`、turn completion `:2672-2698,3281-3444` |
  | queue drain | `src-tauri/src/usecase/agent_session/runtime/usecase.rs:2388-2391` | `start_next_queued_turn` `:2390`（本体 `:3447-3734`）配下の process open `:3502-3533` と backend turn start `:3635-3654` |
  | idle runtime close | `src-tauri/src/adaptor/gateway/workflow/node_lifecycle_adapters.rs:142-149,229-236` | idle 判定 `:32-52` と backend close `src-tauri/src/usecase/agent_session/runtime/usecase.rs:873-880` |

- 規約 (c) について、lock 保持中に残す notifier / state-change emit は次のとおりである。usecase 内の経路はいずれも上表の guard 範囲内であり、単純に guard 外へ移すと turn 開始・完了、stall observe/clear、queue drain の通知順序を崩すため、post-action の拡張を伴う ST-3 / #1412 の分割対象とする。fan-out activation の公開経路は reservation task が guard を保持する間の emit として同じく列挙する。

  | guard 保持経路 | lock 保持中の notifier / emit（違反種別 (c)） |
  |---|---|
  | `send_message` / `start_turn_locked` → `start_turn_for_session` | `src-tauri/src/usecase/agent_session/runtime/usecase.rs:428-430,1188-1190` の turn-prepared notifier、`:1440-1445` の context-carry notifier、`:1539-1555,1584-1600,1633-1649` の開始失敗・開始成功 state-change emit |
  | `respond_permission` | `src-tauri/src/usecase/agent_session/runtime/usecase.rs:587-590` の stall-clear workflow notifier、`:617-627` の streaming-delta notifier、`:638-654` の state-change emit |
  | stale watchdog | `src-tauri/src/usecase/agent_session/runtime/usecase.rs:1997-2009` の stall-observed notifier / workflow notifier |
  | event pump → `apply_runtime_event` | `src-tauri/src/usecase/agent_session/runtime/usecase.rs:2166-2193,2520,2550-2555,2569-2574,2604-2620,2633-2649,2664-2669,2722-2738,2864-2868,2962-2968,3427-3443` の runtime event、context-carry、permission / command / token、stall-clear、streaming-delta、turn-complete state-change の各 notifier / emit |
  | queue drain → `start_next_queued_turn` | `src-tauri/src/usecase/agent_session/runtime/usecase.rs:3513-3529,3597-3602,3674-3690,3702-3709,3716-3732` の reopen / turn-start state-change、context-carry、pending-message / turn-prepared notifier |
  | `activate_fanout_child_sessions` → `broadcast_state` | `src-tauri/src/adaptor/gateway/workflow/runtime_session.rs:526-529`。全 child の reservation task が session guard を保持中に workflow state / agent status を emit する（違反種別 (c)） |

  fan-out activation の `broadcast_state` で実害が無い根拠は、(1) emit する親 flow 自身は guard を保持せず、guard は各 reservation task 内で `JoinHandle` の出力として保持されること、(2) `broadcast_state` は session runtime lock を取得せず、deadlock や lock 保持時間延伸の要因にならないこと、(3) reservation を公開より先行させる順序は behavior.md:43-48 の「workflow activation は公開後の外部操作より先に予約」を満たすために意図されていることである。実 guard 保持と公開順を分離する opaque reservation 化は runtime 状態機械側の責務移動を伴うため、#1412/ST-3 の分割対象とし、本 ISSUE では既知経路の列挙に留める。

  `src-tauri/src/adaptor/gateway/workflow/runtime_engine_impl.rs:3644-3655` の contract repair turn 開始失敗経路については、`start_turn_locked` の結果取得直後に guard を解放してから失敗確定・永続化・workflow state broadcast へ進むよう是正済みである。ただし、上表の `start_turn_locked` 共通処理内にある (b)(c) はこの呼び出し元にも適用され、分割対象として残る。以上で production の `acquire_session_lock` / `acquire_session_runtime_lock` 呼び出し元と、是正を見送る (b)(c) の対象箇所を一対一で確定する。本 ISSUE は lock 機構の runtime 非依存 prune と再入検出、および局所的に分離可能な (a)(c) 違反是正に限定する。

## 受け入れ基準の概要

- AC1（ISSUE 受け入れ基準）: lock 規約が rustdoc に明記され、既存違反が是正 または 列挙されている。
- AC2（ISSUE 受け入れ基準）: prune skip 経路が無い（エントリが蓄積しない）。
- AC3（ISSUE 受け入れ基準）: テストビルドで lock 再入が検出される。
- AC4: `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test`（`src-tauri/`）が通る。

## 仮定

- 仮定 A1: spec-id は既存慣習（`docs/specs/issues-<番号>/`）に合わせ `issues-1411` とする。
- 仮定 A2: 「prune 候補キューへ登録し次回 acquire 時に掃除する」方式は、`Drop` から同期的に触れる共有状態（例: `session_locks` と同じ `Mutex` 配下に持つ pending 集合、または `Drop` 時に `try_lock` で除去し失敗時のみ pending 登録）で実現する。具体データ構造は design.md で確定する。本 requirements では「ランタイム非依存で、解放済み未参照エントリが次回 acquire までに必ず掃除される」ことのみを要求する。
- 仮定 A3: 再入検出は task owner keyed な共有「現在保持中の session lock 集合（または保持フラグ）」を持ち、`acquire` 時に (a) 既に何らかの session lock を保持している場合を規約違反として、`#[cfg(test)]` 内の `assert!` で test profile や Tokio worker 移動に依存せず検出する。検出粒度（同一 session のみか、任意 session 保持中か）は design.md で確定するが、少なくとも「別 session の lock を保持したまま acquire する」ケースを検出する。
- 仮定 A4: 既存経路の棚卸しで規約違反が見つかった場合、本 ISSUE のスコープ内で是正できる規模であれば是正し、過大なら列挙して分割する（ISSUE 記載の分割判断に従う）。現時点では deadlock の実績は無く、明確な違反が無ければ「違反なし」を棚卸し結果として記録する。
- 仮定 A5: 外部から観測可能な UI/CLI の振る舞い変更は無い（内部の排他機構の是正のみ）。

## Open Questions

なし
