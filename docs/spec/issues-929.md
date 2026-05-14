## 要求

**種別**: バグ修正

**ゴール**:
- Workflow 実行時、各 step に紐づく Agent runtime（Claude bridge / LSP を含む）が step 完了後も残り続ける挙動を解消する。
- 完了した step の Agent runtime を解放し、step 数に比例して backend process / LSP が起動したまま残るメモリ圧迫を防ぐ。
- step の履歴（メッセージ・agent session 識別子）は cleanup 対象とせず保持する。解放対象は Agent runtime（Claude bridge / LSP を含む）と tab 状態に限定する。
- 完了済み step の履歴は引き続き閲覧可能で、**ユーザーがメッセージ送信した時のみ** Agent runtime を start/resume して継続できる状態にする。閲覧（tab open / reopen）だけでは Agent runtime を起動しない。
- 本対応の対象は **workflow step session のみ** とする（通常チャット等の非ワークフロー session は対象外）。session が workflow step session か否かは `ChatSession.workflow_step_session` フラグで判定し、追加の所属検証（execution_id / worktree_path 一致確認）は行わない。
- **逐次 step と並列ブロックの子 step は、step session lifecycle（runtime release / tab close / output 永続化 / contract 検証）を単一の経路で扱う**。並列ブロックの「全子完了で親 step を advance する」という遷移判定だけが本質的な差分で、それ以外（turn_complete 受信から runtime release までの不変条件）は完全に共通化する。これにより、片方のパスだけ修正されてもう片方が古いままになり挙動が分岐する状況を構造的に排除する。

**背景**:
- Issue #929 で「ワークフロー実行時、step ごとに AgentProcess / Claude bridge / LSP が起動されるが、step 完了後も runtime が残り続ける」と報告されている。
- 結果として、step 数に比例して backend process / LSP が起動したままになり、メモリを圧迫している。
- 現状の `ChatSession` は履歴・UI 表示単位として残すべきものであり、cleanup 対象ではない。一方で、実行用の `AgentProcess` / Claude bridge / LSP は step 完了後に release されるべきだが、release されていない。
- 「完了済み session を消す」ではなく、「完了した step に紐づく Agent runtime を release する」という整理が必要。
- 複数タブで履歴を開いた場合に、閲覧だけで backend process が起動してしまう状況も避けたい。

**対応方針（合意済み）**:

Rust 側で次の 2 つの状態を **別管理** として持つ。

1. **起動中 Session 一覧**（既存 `AgentProcessMap`）: `chat_session_id -> AgentProcess` の runtime マップ。
2. **Open 中 Tab 一覧**（新規 `OpenTabRegistry`）: UI 上で開かれている step tab の `chat_session_id` 集合。フロントから open/close/reopen を Tauri コマンドで通知して維持する。

フロントエンドは Rust から両方の状態を取得して表示を切り替える（runtime 起動中か否か、tab open 中か否か、を区別して表示）。

既存 `SessionStore` には `SessionState::Closed` トグルがあり、`AgentChatPanel` の tab 一覧は `list_sessions` / `close_session` / `restore_session` 経由でこの state を観測している。本対応では step `ChatSession`（`workflow_step_session: true`）の tab close/reopen 時に **`SessionState::Closed` トグルと `OpenTabRegistry` の双方** を更新する。さらに `restore_session` 経路は対象 session の `workflow_step_session` フラグで分岐し、step session を `restore_session` した場合は `start_agent_session` / `start_agent_turn` を呼ばずに `OpenTabRegistry.add` のみを行うこと（閲覧のみで runtime を起動しない不変条件と整合させる）。

状態遷移ルール:

- **Step Done 時**（`on_turn_complete` 後に step が `Done` 状態へ確定したタイミング）:
  - step の output / token usage を回収・永続化したうえで、当該 step の step session（`current_session_id` / `StepHistoryEntry.session_id`、これらは同一の step `chat_session_id`）に対して `AgentProcess`（runtime）を release する。
  - あわせて当該 `chat_session_id` を `OpenTabRegistry` から remove する。release 対象は `AgentProcess`（Claude bridge / LSP を含む）と `OpenTabRegistry` の該当 entry に限定する。
  - `ChatSession` / messages / agent_session_id は履歴として残す。
  - tab が既に閉じている（`OpenTabRegistry` に存在しない）状態で Done が確定した場合でも、`AgentProcess` は release する。

- **UI で Tab を Close した時**:
  - 当該 step session の `AgentProcess` も close する。
  - ただし Rust 側でガードを行い、**step 動作中の Session は close しない**（runtime はそのまま残し、tab だけ閉じる扱い）。
  - 「step 動作中」とは、当該 step session の AgentProcess が次のいずれかの状態を満たすことと定義する（grace period なし）。
    - `BridgeState::Streaming` である
    - `TurnPhase::WaitingPermission` である
    - `pending_message.is_some()` である

- **UI で Tab を ReOpen（再表示）した時**:
  - Open 中 Tab 一覧に当該 `chat_session_id` を追加するのみ。
  - runtime は起動しない（閲覧だけで backend process を起動しない）。

- **Message 送信時**:
  - 完了済み step の `ChatSession` に対してユーザーがメッセージを送信した場合、`AgentProcess` が存在しなければ `agent_session_id` を用いて start/resume してから turn を開始する。
  - すでに起動中であればそのまま turn を開始する。

- **逐次 step と並列子 step の共通化**:
  - `on_turn_complete` 時のディスパッチは、session が workflow step session であるか否かのみで分岐させる。**逐次 step か並列子 step かによる経路分岐は行わない**。
  - 並列子 step に固有の処理（複数子の完了同期、aggregate 評価、親 step への昇格）は、共通経路の中で `WorkflowExecution.parallel_run` の有無に応じて分岐させる。`SessionRefKind` に `ParallelChild { parent_step_name }` / `SequentialStep` の別を持たせず、step session を表す単一のバリアントに統合する。`parent_step_name` の情報が必要な場合は `exec.parallel_run.parent_step_name` から取得する（情報の重複保持を排除する）。
  - 結果として、runtime release / tab close / output 永続化 / contract 検証 / 「step 動作中」判定 / SessionState の更新 / OpenTabRegistry の更新は、逐次 step と並列子 step で同じヘルパ呼び出しを通る。

**期待する挙動**:
- ワークフローの各 step 履歴は閲覧可能なまま残る。
- step 完了後は Agent runtime（Claude bridge / LSP を含む）が残らず、tab 状態も同時に解放される。
- UI で Tab を閉じた時、Rust 側のガードにより動作中 step の Agent runtime は残り、非動作中の step の Agent runtime のみ解放される。
- 完了済み step の Tab を ReOpen（再表示）しても Agent runtime は起動しない（履歴表示のみ）。
- メッセージ送信時に限り、Agent runtime がなければ start/resume して turn を開始できる。
- 重複する tab open / close 操作は冪等に処理され、状態を変更せずエラーにもしない。
- 完了確定と tab close、および同一 step への並行メッセージ送信が競合しても、Agent runtime の release / 起動は当該 step に対して高々 1 回のみ実行され、二重解放・二重起動が発生しない。
- **逐次 workflow と並列 workflow の step session は、runtime release / tab close / output 永続化までの観測可能な挙動が同一**。並列ブロックでも、各子 step は他の子と独立に上記不変条件を満たし、全子完了時点で親 step（並列ブロック自身）が advance する。

**バグ詳細**:
- 現在の挙動: workflow を実行すると step ごとに Agent runtime（Claude bridge / LSP を含む）が起動され、step が完了状態に遷移した後も Agent runtime が残り続ける。step 数に比例して backend process / LSP が増え続け、メモリを圧迫する。
- 期待する挙動: step が完了状態になったタイミングで当該 step の Agent runtime を解放し、履歴のみを残す。完了済み step に対してメッセージ送信した時のみ start/resume で Agent runtime を再起動する。UI 上で tab を閉じた場合は、Rust 側のガード判定（step 動作中でないこと）を経て Agent runtime を解放する。
- 再現手順: 複数 step を持つ workflow を実行し、step を順次完了させながら起動中の Agent runtime / Claude bridge / LSP の数を観察する。step 完了後も Agent runtime が残ったままになることを確認する。

## 振る舞い定義

```gherkin
Feature: ワークフロー step ごとの Agent runtime ライフサイクル管理
  workflow 実行時の各 step に紐づく Agent runtime（Claude bridge / LSP を含む）を
  step 完了や tab 状態、メッセージ送信に応じて release / start し、メモリ圧迫を防ぐ。
  step の履歴（ChatSession / messages / agent_session_id）は cleanup 対象とせず保持する。

  Background:
    Given 各 workflow step は { runtime_active: bool, tab_open: bool } の状態を持つ
    And runtime_active は当該 step の Agent runtime が起動している場合に true となる
    And tab_open は当該 step の tab が UI 上で開かれている場合に true となる
    And 「step 動作中」とは当該 step の runtime が
        応答ストリーミング中、権限承認待ち、送信予約あり のいずれかを満たす状態と定義する（grace period なし）
    And エラー応答および構造化ログには redacted な error code と汎用 message のみが含まれ
        agent_session_id / message 本文 / step output / worktree path は含まれない

  # --- 操作 → 状態変化 ---

  Rule: step が完了すると runtime は解放され履歴のみ残る
    Scenario: tab 表示中の step が完了すると runtime も tab も解放される
      Given workflow の step が実行中で runtime が起動している
      And 当該 step の tab は開かれている
      When step が完了状態へ確定する
      Then 当該 step の output と token usage は永続化される
      And 取得した step 状態は runtime_active=false かつ tab_open=false となる
      And 当該 step の履歴（ChatSession / messages / agent_session_id）は閲覧可能なまま残る

    Scenario: tab を閉じた状態で step が完了しても runtime は解放される
      Given workflow の step が実行中で runtime が起動している
      And 当該 step の tab は閉じられている
      When step が完了状態へ確定する
      Then 取得した step 状態は runtime_active=false かつ tab_open=false となる

    Scenario: 動作中 step の tab を閉じた後に完了が確定すると残っていた runtime が解放される
      Given 未完了 step の tab は開かれており runtime が動作中である
      When ユーザーが当該 tab を閉じる
      Then 取得した step 状態は runtime_active=true かつ tab_open=false となる
      When 後続の turn 完了で step が完了状態へ確定する
      Then 取得した step 状態は runtime_active=false かつ tab_open=false となる

  Rule: tab close は動作中でない runtime のみを解放する
    Scenario: 非動作中 step の tab を閉じると runtime も解放される
      Given 完了済み step の tab は開かれており runtime が起動している
      And 当該 step は動作中ではない
      When ユーザーが当該 tab を閉じる
      Then 取得した step 状態は runtime_active=false かつ tab_open=false となる
      And 当該 step の履歴は閲覧可能なまま残る

    Scenario: 動作中 step の tab を閉じても runtime は残る
      Given 未完了 step の tab は開かれており runtime が動作中である
      When ユーザーが当該 tab を閉じる
      Then 取得した step 状態は runtime_active=true かつ tab_open=false となる

  Rule: tab の再表示では runtime を起動しない（閲覧のみ）
    Scenario: 完了済み step の tab を再オープンしても runtime は起動しない
      Given 完了済み step の履歴が残っている
      And 当該 step の runtime は停止している
      When ユーザーが当該 step の tab を再オープンする
      Then 取得した step 状態は runtime_active=false かつ tab_open=true となる
      And 当該 step の履歴がそのまま閲覧できる

    Scenario: runtime 起動中の step を再オープンしても runtime 状態は変化しない
      Given step の runtime は起動中で tab は閉じられている
      When ユーザーが当該 step の tab を再オープンする
      Then 取得した step 状態は runtime_active=true かつ tab_open=true となる

  Rule: メッセージ送信時のみ runtime を必要に応じて再開する
    Scenario: 完了済み step へのメッセージ送信で runtime が再開される
      Given 完了済み step の履歴が残っている
      And 当該 step の runtime は停止している
      When ユーザーが当該 step にメッセージを送信する
      Then runtime が当該 step の agent_session_id を用いて resume 起動される
      And 当該メッセージで新たな turn が開始される
      And 取得した step 状態は runtime_active=true となる

    Scenario: runtime 起動中の step へのメッセージ送信では既存 runtime が再利用される
      Given step の runtime は起動中である
      When ユーザーが当該 step にメッセージを送信する
      Then runtime は二重起動されず既存の runtime で turn が開始される

  # --- 状態 → 表示 ---

  Rule: フロントは Rust が公開する step 状態フィールドで実行状況を観測する
    Scenario: runtime 起動中かつ tab open 中の step は「実行中」として観測される
      Given step の runtime は起動中で tab は開かれている
      When フロントが workflow step 一覧を取得する
      Then 当該 step の状態は runtime_active=true かつ tab_open=true となる

    Scenario: 完了済みで履歴のみの step は「履歴のみ」として観測される
      Given step の履歴は残っており runtime は停止している
      And 当該 step の tab は閉じられている
      When フロントが workflow step 一覧を取得する
      Then 当該 step の状態は runtime_active=false かつ tab_open=false となる

    Scenario: 完了済み step の tab を再オープンすると「履歴閲覧中」として観測される
      Given 完了済み step の履歴が残っており runtime は停止している
      And ユーザーが当該 step の tab を再オープンしている
      When フロントが workflow step 一覧を取得する
      Then 当該 step の状態は runtime_active=false かつ tab_open=true となる

  # --- スコープ ---

  Rule: 非 workflow session は workflow step の状態管理に影響しない
    Scenario: 非 workflow session は workflow step 一覧に現れない
      Given 通常チャット（非 workflow）の ChatSession が存在する
      When フロントが workflow step 一覧を取得する
      Then 当該 chat_session_id は workflow step 一覧に含まれない

    Scenario: 非 workflow session への tab 操作は workflow step の状態を変化させない
      Given 通常チャット（非 workflow）の ChatSession が存在する
      And 任意の workflow step が runtime_active と tab_open の組合せ状態を持っている
      When ユーザーが当該の非 workflow session に対して tab open / close / reopen のいずれかを行う
      Then 任意の workflow step の runtime_active と tab_open は変化しない

    Scenario: 非 workflow session へのメッセージ送信は workflow step の runtime_active を変化させない
      Given 通常チャット（非 workflow）の ChatSession が存在する
      And 任意の workflow step が runtime_active の値を持っている
      When ユーザーが当該の非 workflow session にメッセージを送信する
      Then 任意の workflow step の runtime_active は変化しない

  # --- 冪等性・並行制御 ---

  Rule: 重複操作は冪等に処理される
    Scenario: 既に開いている tab への重複 open は状態を変えない
      Given step の tab は既に開かれている
      When 同じ step に対して tab open が再度呼ばれる
      Then tab 状態は変化せずエラーにもならない

    Scenario: 既に閉じている tab への重複 close は状態を変えない
      Given step の tab は既に閉じられており runtime も停止している
      When 同じ step に対して tab close が呼ばれる
      Then runtime と tab の状態は変化せずエラーにもならない

  Rule: 並行操作でも runtime は二重に作られず二重に解放されない
    Scenario: 完了確定と tab close が競合しても runtime は二重解放されない
      Given step が完了確定処理中である
      When 同時に同じ step の tab close が呼ばれる
      Then runtime の release 操作は当該 step に対して高々 1 回のみ実行される
      And 最終状態は runtime_active=false かつ tab_open=false となる

    Scenario: 同一 step への同時メッセージ送信で runtime は二重起動されない
      Given 完了済み step の runtime は停止している
      When 同じ step に対して並行して 2 件のメッセージ送信が発生する
      Then runtime の起動は当該 step に対して高々 1 回のみ実行される

  # --- 異常系 ---

  Rule: 異常系では状態を一貫させ後続処理を妨げない
    Scenario: step 完了時の output / token usage 永続化に失敗しても runtime と tab は解放される
      Given step 完了確定処理中である
      When output / token usage の永続化が失敗する
      Then 当該 step の runtime 解放と tab 解放は引き続き試行される
      And 構造化ログには redacted な error code と汎用 message のみが記録される

    Scenario: runtime の解放処理に失敗した場合でも最終状態は履歴のみへ収束する
      Given step 完了確定処理または tab close 処理中である
      When runtime の解放処理が失敗する
      Then 操作はエラー（redacted な error code と汎用 message）として返される
      And 最終的に当該 step の runtime entry は削除され runtime_active=false に固定される
      And tab_open は false に固定される
      And 構造化ログには redacted な error code と汎用 message のみが記録される

    Scenario: メッセージ送信時の runtime 起動に失敗した場合は半端な runtime を残さない
      Given 完了済み step の runtime は停止している
      When メッセージ送信時の runtime 起動が失敗する
      Then 当該メッセージの turn は開始されない
      And 操作はエラー（redacted な error code と汎用 message）として返される
      And 当該 step の runtime_active は false に固定される
      And 構造化ログには redacted な error code と汎用 message のみが記録される

    Scenario: tab open / reopen 時の状態更新に失敗しても runtime 状態は変更されない
      Given tab open / reopen 処理中である
      When tab 状態の更新が失敗する
      Then 操作はエラー（redacted な error code と汎用 message）として返される
      And runtime 状態は処理前のまま変更されない

    Scenario: tab close で runtime 解放成功後の tab 状態更新に失敗した場合は runtime を rollback しない
      Given tab close 処理中で runtime の解放は既に成功している
      When 続く tab 状態更新が失敗する
      Then 操作はエラー（redacted な error code と汎用 message）として返される
      And runtime は rollback されず runtime_active=false が維持される
      And tab_open=true が残ることを許容する
```

## アーキテクチャ概要

### 責務配置

- **WorkflowEngine (`src-tauri/src/workflow/engine.rs`)**
  - 担当: workflow execution の進行管理、step start、`on_turn_complete` での step Done 検出、step Done 確定時の output / token usage 回収・永続化、`session_workflow_refs` による実行中 step と session の紐付け保持、workflow state snapshot の提供。
  - step Done 確定後の runtime/tab lifecycle は `WorkflowStepLifecycle` に委譲する。WorkflowEngine は step session の lifecycle 操作（tab open / close / restore、message send、generic session command 分岐）を直接 orchestration しない。
  - `on_turn_complete` は **session が workflow step session か否か** だけで分岐させ、逐次 step か並列子 step かでは分岐させない。並列子 step に固有の処理（複数子の完了同期、aggregate 評価、親 step への昇格）は、共通経路の中で `WorkflowExecution.parallel_run` の有無で判定して分岐する。`session_workflow_refs` の値は step session を表す単一のバリアント（`Parent` 以外は `Step` 等の単一種別）に統合し、`ParallelChild { parent_step_name }` のような種別固有データを `session_workflow_refs` に重複保持しない。
  - 担当しない: AgentProcess の起動/停止の実体処理、Open 中 Tab 一覧の保持、Tauri command の公開、フロント描画、workflow step session lifecycle の状態遷移判断。

- **WorkflowStepLifecycle (`src-tauri/src/workflow/step_lifecycle.rs`)**
  - 担当: tab open / close / restore、Step Done cleanup、generic command / WS 経路から `workflow_step_session: true` の session を受けた場合の lifecycle 分岐、`runtime_active` / `tab_open` へ反映される状態更新。
  - `AgentProcessMap` / `OpenTabRegistry` / `SessionStore` / `WorkflowEngine` の低レベル API を呼び分ける唯一の lifecycle 経路とする。Tauri command、session command、WS handler は session の `workflow_step_session` フラグでのみ判定し、フラグが立っている session に対する tab 操作はこの lifecycle 経路へ委譲する。execution_id / worktree_path に基づく所属検証は行わない。
  - 担当しない: workflow の次 step 判定、step prompt 生成、AgentProcess の内部状態再現、フロント描画、メッセージ送信処理（generic `send_agent_message` 経路に委ねる）。

- **AgentProcessMap (`src-tauri/src/backends/bridge_common.rs`)**
  - 担当: `chat_session_id -> AgentProcess` の runtime マップ管理、Claude bridge / LSP プロセスの起動・停止、turn 実行状態（`BridgeState::Streaming` / `TurnPhase::WaitingPermission` / `pending_message`）の保持、「step 動作中」判定（上記いずれかを満たす場合に動作中、grace period なし）の入力提供および判定関数の公開。
  - 既存の **非 Tauri な internal API**（`close_agent_session_internal` / `start_agent_session_internal`）を、Tauri command（UI 経由の close）と WorkflowEngine（step Done 経由の close）の双方から呼び出せる形で活用する。Tauri command（`close_agent_session` / `start_agent_session`）は本 internal API への薄いラッパーとして整理する。`start_agent_session_internal` は `start_agent_turn` の spawn-if-needed 経路からも呼ばれる。
  - 担当しない: step Done 判定、Open 中 Tab 一覧の保持、UI への表示制御、Tauri 境界での `tauri::State` 取り回し（Tauri command 層の責務）。

- **OpenTabRegistry（新規, Rust 側）**
  - 担当: Open 中 Tab の `chat_session_id` 集合を一元保持し、`add` / `remove` / `contains` / `snapshot` 等の内部 API を提供する。
  - 担当しない: Tauri command の直接公開、AgentProcess のライフサイクル制御、step 状態判定。これらは workflow step lifecycle 経路に委譲する。

- **SessionStore (`src-tauri/src/session/`)**
  - 担当: `ChatSession` / messages / `agent_session_id` / `workflow_state` の永続化、`SessionState::Closed` トグル（`list_sessions` / `close_session` / `restore_session`）、`chat_session_id` の存在確認、step session 判定（`ChatSession.workflow_step_session` フラグの読み取り）。
  - 担当しない: runtime（AgentProcess / bridge / LSP）の生存管理。execution_id / worktree_path に基づく所属検証。

- **Workflow Tauri Commands (`src-tauri/src/workflow/commands.rs` ほか)**
  - 担当: UI ↔ Rust の境界。step tab open / close / reopen、step 状態取得の Tauri command を提供し、payload の形式検証と状態変化イベントの通知を担う。メッセージ送信は generic `send_agent_message` 経路を使用し、workflow step session 専用のメッセージ送信 Tauri command は設けない。
  - command は `WorkflowStepLifecycle` を呼び出す薄い境界とし、AgentProcessMap / OpenTabRegistry / SessionStore の状態遷移を command 内で別々に再実装しない。
  - 既存の generic session command（`close_session` / `restore_session`）から step `ChatSession`（`workflow_step_session: true`）を扱う場合は、`WorkflowStepLifecycle` に委譲し、閲覧だけで runtime を起動しない、動作中 step の runtime を close しない、という不変条件を維持する。execution_id / worktree_path に基づく所属検証は行わない。
  - 担当しない: workflow step 進行判断、AgentProcess の実体操作、Open 中 Tab 集合の保持。

- **AgentChatPanel / step tab UI (`src/components/panels/AgentChatPanel/`, `src/hooks/useAgentChat.ts`)**
  - 担当: step `ChatSession` の tab 一覧表示・tab 操作（open / close / reopen）。Rust から取得した step 一覧（`runtime_active` / `tab_open` を含む）を表示状態にマッピングし、tab 操作・メッセージ送信 UI を提供する。
  - 担当しない: Open 中 Tab 一覧・runtime 起動状態をフロントローカルで独自保持すること、runtime 起動可否の独自判定。

- **WorkflowPanel / useWorkflowState (`src/components/panels/AgentChatPanel/WorkflowPanel/`, `src/hooks/useWorkflowState.ts`)**
  - 担当: workflow execution history（過去 workflow 実行）の一覧表示、`openPastIds` による execution history の開閉状態管理、`WorkflowTrace` の View 導線。
  - 担当しない: step `ChatSession` の tab 管理（`AgentChatPanel` / `useAgentChat` の責務）。`WorkflowPanel.openPastIds` は execution history 用であり、本対応の OpenTabRegistry 置換対象に**含めない**。

### 状態フィールド契約

Rust が公開する workflow step 状態には次のフィールドを含む。フロントおよびテストはこのフィールドを観測点とする。

- `runtime_active: bool` — 当該 `chat_session_id` が AgentProcessMap に存在する場合に `true`。
- `tab_open: bool` — 当該 `chat_session_id` が OpenTabRegistry に存在する場合に `true`。

「step 動作中」は AgentProcessMap が提供する判定関数の結果で表現し、`BridgeState::Streaming` / `TurnPhase::WaitingPermission` / `pending_message.is_some()` のいずれかを満たす場合に動作中とする（grace period なし）。

### データ/通信フロー

- **step Done 確定**: `WorkflowEngine.on_turn_complete` → step 状態を Done へ遷移 → output / token usage 永続化を試行 → 保持していた当該 step の `current_session_id`（= `StepHistoryEntry.session_id` = step `chat_session_id`）を対象に `WorkflowStepLifecycle.release_on_step_done` で runtime release と tab close を実施 → 状態変化イベントをフロントへ emit。tab が閉じていても runtime は release する。永続化が失敗しても runtime release / OpenTabRegistry remove / SessionState Closed 化は試行し、runtime_active=false / tab_open=false を最終状態として固定する。並列ブロックの子 step も同じ経路を通る（各子の turn_complete ごとに本経路で runtime release / tab close を実施し、`parallel_run.children` の completion 集計に基づいて all_done に到達した時点で親 step の advance を行う）。逐次か並列子かによって本経路の不変条件（release / tab close / 履歴保持）に差は生じない。
- **UI Tab Close**: フロント → command → 対象 session が `workflow_step_session: true` のとき `WorkflowStepLifecycle.close_tab` に委譲（フラグが立っていなければ generic 経路で SessionState Closed 化のみ）→ AgentProcessMap の「step 動作中」判定 → 非動作中なら runtime release を実施し、OpenTabRegistry から remove と SessionStore `close_session` 相当の状態更新を実施。動作中なら OpenTabRegistry.remove と SessionStore の Closed 化のみを実施し runtime は残す → 状態変化イベント emit。runtime release 成功後の tab 状態更新失敗時は rollback せず、AgentProcessMap=close 済（runtime_active=false） / OpenTabRegistry=未削除（tab_open=true）の状態を許容し Result::Err を返す。冪等性のため、すでに OpenTabRegistry に存在しない chat_session_id への close はエラーとせず no-op として扱う。
- **UI Tab ReOpen**: フロント → command → 対象 session が `workflow_step_session: true` のとき `WorkflowStepLifecycle.restore_tab` に委譲（フラグが立っていなければ generic `restore_session` 挙動）→ `OpenTabRegistry.add` と SessionStore の `restore_session` 相当の状態更新（`start_agent_session_internal` / `start_agent_turn` を呼ばない）を実施 → 状態変化イベント emit。重複 open は冪等。
- **Message 送信**: フロント / WS → generic `send_agent_message` command → `start_agent_turn` の spawn-if-needed 経路で AgentProcessMap に当該 chat_session_id の AgentProcess が不在なら `agent_session_id` を用いて `start_agent_session_internal`（resume）を実施し、起動後に turn を投入。session_id が存在しなければ既存の `send_agent_message_internal` が自然に Err を返す。並行送信時にも spawn が二重に走らないよう、runtime ensure helper（共通化箇所）に同期プリミティブを導入する。spawn 失敗時は turn を開始せず Result::Err を返し、半端な AgentProcess を残さない（runtime_active=false に固定）。
- **画面表示**: フロントは Rust が公開する step 一覧（`runtime_active` / `tab_open` を含む）を取得・購読し、両フィールドの組合せで「実行中」「履歴のみ」等の表示を決定する。
- **エラー観測**: 上記すべての異常系で、Tauri command は `Result::Err` を返し、`tracing` ベースの構造化ログには `agent_session_id` / message 本文 / step output / worktree path などのセンシティブな値を含めない。

### 状態Owner

- **起動中 Session 一覧（`chat_session_id -> AgentProcess`）**: AgentProcessMap（Rust）
- **Open 中 Tab 一覧（`chat_session_id` の集合）**: OpenTabRegistry（Rust, 新規）
- **「step 動作中」判定材料**（`BridgeState::Streaming` / `TurnPhase::WaitingPermission` / `pending_message`）: AgentProcessMap（Rust）
- **step 状態（Done を含む）と step ↔ session の紐付け（`current_session_id` / `StepHistoryEntry.session_id`）**: WorkflowEngine（Rust）
- **workflow step session lifecycle の状態遷移判断**: WorkflowStepLifecycle（Rust）
- **ChatSession / messages / agent_session_id / workflow_state**: SessionStore（Rust, JSON 永続化）
- **フロント表示状態**: Rust 由来の状態の派生表示のみ（独自 Source of Truth を持たない）

### 境界

- フロントエンドは「Open 中 Tab 一覧」「runtime 起動状態」のローカル独自保持を行わず、Rust から取得した `runtime_active` / `tab_open` を表示する。step `ChatSession` の tab 管理は `AgentChatPanel` / `useAgentChat` 経路に集約する。`WorkflowPanel.openPastIds` は workflow execution history の ID 管理であり、本対応の OpenTabRegistry 置換対象には含めない（既存挙動を維持する）。
- AgentProcess の起動/停止は `bridge_common` が公開する既存の非 Tauri な internal API（`close_agent_session_internal` / `start_agent_session_internal`）または同等の AgentProcessMap 内部 API を経由する。workflow step lifecycle 経路はこれらを呼び出してよいが、AgentProcessMap の内部状態を別モジュールで独自に再現しない。
- OpenTabRegistry は AgentProcess のライフサイクルを直接制御せず、Tauri command の公開も行わない。集合の保持と内部 API 提供のみを担い、状態遷移の判断は workflow step lifecycle 経路が行う。
- tab open / close / reopen は Workflow Tauri Commands を経由するか、generic `close_session` / `restore_session` 経由で `WorkflowStepLifecycle` に委譲する。`ChatSession.workflow_step_session` フラグでのみ workflow lifecycle 経路と generic 経路を分岐し、execution_id / worktree_path に基づく所属検証は行わない。
- message 送信は generic `send_agent_message` 経路を使用する。execution_id / worktree_path に基づく所属検証は行わない。session_id が不在のときは既存の `send_agent_message_internal` が自然に Err を返すのに委ねる。並行送信時の spawn 二重防止は `start_agent_turn` の spawn-if-needed 経路に同期プリミティブを導入して担保する。
- フロントエンドは tab close のために `close_agent_session` を直接呼ばない。runtime を閉じるかどうかは `WorkflowStepLifecycle.close_tab` が Rust 側で判断する。
- 「step 動作中」判定は AgentProcessMap が公開する単一関数に集約し、UI 側・WorkflowEngine 側で独自再現しない。
- ChatSession / messages / agent_session_id は本対応の cleanup 対象に含めない。release 対象は AgentProcess（および付随する Claude bridge / LSP）と Open 中 Tab 集合に限定する。
- 本対応のスコープは workflow step session のみ。通常チャット等の非ワークフロー session は OpenTabRegistry の対象外とし、既存の AgentProcessMap 挙動と既存の `SessionState::Closed` / `restore_session` 挙動を変更しない。
- 構造化ログには `agent_session_id` / message content / step output / worktree path / 内部スタックトレース等のセンシティブ情報を含めない。

### Tauri command ペイロード契約

本対応で追加・変更する Tauri command のペイロード型は、次の方針のみを Spec で固定する。具体的な検証規則・上限値・正規化処理および serde の表現詳細・命名は実装に委ねる。

- `chat_session_id` を受け取る command は形式チェックのみ行い、execution_id / worktree_path に基づく所属検証は行わない。session の存在判定は呼び出し先（`send_agent_message_internal` / SessionStore 等）に委ね、不在時の Err はそのまま伝播させる。
- step 状態取得 command の戻り値は `runtime_active: bool` / `tab_open: bool` を必須フィールドとして含む。

### 実装に委ねること

- OpenTabRegistry の具体的データ構造（`HashSet` / `Mutex` / `RwLock` 等）と配置形態（独立 `State` として登録するか、`AgentProcessMap` と同居させるか）。
- Tab open / close / reopen を表す Tauri command 名・イベント名の具体名と、上記ペイロード契約を満たす serde 表現の細部（フィールド名・camelCase/snake_case 等）。
- `WorkflowStepLifecycle` の具体的な struct 名、引数型、State の引き回し方。tab open / close / reopen と Done cleanup の不変条件を単一の Rust 経路で満たすこと。
- generic session command（`close_session` / `restore_session`）で `workflow_step_session: true` の session を受け取った場合の委譲方法（`WorkflowStepLifecycle` への呼び分けポイント）。
- AgentProcessMap における「step 動作中」判定関数の具体名と置き場所（メソッドか別 helper か）、および並行制御に用いるロックの粒度。
- 既存 `close_agent_session_internal` / `start_agent_session_internal` を workflow step lifecycle 経路から呼ぶための引数取り回し（State の引き回し、引数追加要否等）。
- フロント表示での「runtime 起動中」「履歴のみ」を示すアイコン・ラベル・色などの UI 表現。
- メッセージ送信経路における spawn-if-needed の二重防止に用いる同期プリミティブの選択（`Mutex` / `OnceCell` / `tokio::sync::Mutex` 等）。
- 内部 helper 関数（output 永続化、close 一括処理など）の名前・分割粒度。
- テストケースの具体的配置（既存 `#[cfg(test)] mod tests` への追記か新規モジュール作成か）と、Tauri command 単位テスト／統合テストの粒度。
