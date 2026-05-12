## 要求

**種別**: 改善
**ゴール**: Spec修正やコード修正などの各種 Fix ステップに入る際、レビュー指摘をそのまま自動修正するのではなく、ユーザーが修正方針を確認・指示できる承認待ちを挟めるようにする。完了時には、Fix ステップがレビュー結果だけでなく人間が Approve した最新方針を踏まえて修正を進められる状態になっている。また、対話を伴う step は `approval` に統一し、`interactive` mode は廃止する。
**背景**: 現在の `plan_fix` や `fix` はレビュー指摘に基づいて自動で修正を進めるため、修正方法に選択肢がある場合や、指摘の扱いに人間の判断が必要な場合でも、エージェントが保守的な判断で進めてしまう。修正前に方針確認の対話を挟むことで、意図しない仕様変更や過剰修正を避け、ユーザーの判断を修正サイクルに反映できるようにする。
**対象ユーザー**: spec-driven-development ワークフローで Spec レビュー修正や実装レビュー修正を行うユーザー
**利用シーン**: plan review や code review の結果が `NEEDS_FIX` となり、`plan_fix` または `fix` に遷移する場面
**制約**: フロントエンドは表示・入力受付に徹し、方針対話の扱い、ステップ遷移、後続 Fix ステップへの方針伝播、`approval` step の表示判定は Rust 側の workflow 実行ロジックで管理する。
**影響範囲**: spec-driven-development の Fix 系ステップ、レビュー結果から Fix へ戻る遷移、approval step の出力伝播、workflow step mode、workflow trace 表示、承認待ちUI

### 方向性

- `plan_fix` の前に Plan 修正方針確認の `approval` step を置き、レビュー指摘に対して「どの方針で Spec を直すか」をユーザーがチャットで調整し、Approve できるようにする
- `fix` の前に実装修正方針確認の `approval` step を置き、コードレビュー指摘に対して「どの方針でコードを直すか」をユーザーがチャットで調整し、Approve できるようにする
- 修正方針確認 step は workflow 上の明示的な承認境界として扱い、後続の `plan_fix` / `fix` は Approve された最新の修正方針を `approved-fix-policy` output contract の structured output として受け取る
- `interactive` mode は廃止し、既存・新規を問わず workflow step mode として無効にする。ユーザーとの対話・成果物確認・承認待ちは `approval` mode に統一し、`mode: interactive` を含む workflow 定義は `validation_error` にする
- `approval` step の `rules` は Reject 操作用の差し戻し遷移だけを表す。`approval` step は最大1件の `match: reject` rule を持て、1件ある場合のみ Reject 可能、0件の場合は Reject 不可とする
- Reject 可否は Rust 側の `WorkflowEngine` が評価し、`WorkflowState` に同期された `canReject=true/false` 相当の解釈済み approval 操作可否だけを React が表示に使う。React は workflow definition や `rules` を解釈しない
- Reject はチャット調整ではなく、approval step の reject rule に従う明示的な差し戻し遷移として扱う
- workflow approval step 専用の AutoApprove オプションを置き、明示的な承認境界は残したまま、必要に応じて人間の Approve 操作を自動通過できるようにする。この AutoApprove は agent 実行権限向けの `agentAutoApprove` とは独立した設定で、既定値は無効とする
- AutoApprove が有効な場合でも、承認境界の出力は後続 Fix ステップに渡され、workflow trace 上でどの方針が採用されたかを追跡できるようにする
- 採用済み方針として保存・同期・後続 Fix へ注入する値は Fix に必要な方針情報に限定し、固定の機密値パターンに該当する値は `[REDACTED]` に置換した形だけを workflow log、workflow variables、remote sync payload、trace 表示、後続 Fix 入力に含める

## 振る舞い定義

```gherkin
Feature: Fix ステップ前の修正方針確認

  spec-driven-development ワークフローでレビュー結果が NEEDS_FIX になったとき、
  ユーザーが承認待ち中に修正方針を確認・指示し、Approve した最新方針で Fix ステップへ進められるようにする。

  Rule: レビュー指摘は Fix 前の修正方針確認へ遷移する

    Scenario: Plan レビューが NEEDS_FIX の場合に Plan 修正方針確認へ進む
      Given plan review が Spec の修正を必要としている
      When plan review の判定が NEEDS_FIX になる
      Then ワークフローは plan_fix を実行せず Plan 修正方針確認に遷移する

    Scenario: コードレビューが NEEDS_FIX の場合に実装修正方針確認へ進む
      Given code review が実装の修正を必要としている
      When code review の判定が NEEDS_FIX になる
      Then ワークフローは fix を実行せず実装修正方針確認に遷移する

    Scenario: 並列レビューの一部が NEEDS_FIX の場合に全レビュー結果を方針確認へ渡す
      Given 並列レビューの結果に LGTM と NEEDS_FIX が混在している
      When aggregate result が NEEDS_FIX になる
      Then 修正方針確認は全レビュー結果と NEEDS_FIX findings を入力として提示する

  Rule: 承認待ち中のチャット指示は同じ修正方針確認で方針案を更新する

    Scenario: Plan 修正方針案をチャットで調整する
      Given Plan 修正方針確認が execution_id E、current_step S、current_session_id C で承認待ちである
      When ユーザーがチャットで Plan 修正方針への指示を追加する
      Then WorkflowState の execution_id、current_step、current_session_id は E、S、C のまま変わらない
      And S に属する完了済み最新 assistant output が更新済み Plan 修正方針案に置き換わる

    Scenario: 実装修正方針案をチャットで調整する
      Given 実装修正方針確認が execution_id E、current_step S、current_session_id C で承認待ちである
      When ユーザーがチャットで実装修正方針への指示を追加する
      Then WorkflowState の execution_id、current_step、current_session_id は E、S、C のまま変わらない
      And S に属する完了済み最新 assistant output が更新済み実装修正方針案に置き換わる

    Scenario: 承認待ちではない step にチャット調整を送信できない
      Given workflow が waiting_approval ではない
      When ユーザーが修正方針へのチャット指示を送信する
      Then ワークフロー状態は変わらず `invalid_state` error kind が表示用状態として返される

    Scenario: 上限を超えるチャット指示は採用されない
      Given 修正方針確認が承認待ちである
      When ユーザーが 8192 文字を超えるチャット指示を送信する
      Then 修正方針確認は更新されず `validation_error` error kind が表示用状態として返される

  Rule: Approve された最新の修正方針だけが Fix ステップに伝播される

    Scenario: 最新の Plan 修正方針を Approve して plan_fix へ進む
      Given Plan 修正方針確認が更新済み方針案を提示している
      When ユーザーが Approve する
      Then 最新の Plan 修正方針が `approved-fix-policy` の structured output として1件だけ記録される
      And plan_fix はレビュー結果と採用済み Plan 修正方針を入力として実行される

    Scenario: 最新の実装修正方針を Approve して fix へ進む
      Given 実装修正方針確認が更新済み方針案を提示している
      When ユーザーが Approve する
      Then 最新の実装修正方針が `approved-fix-policy` の structured output として1件だけ記録される
      And fix はレビュー結果と採用済み実装修正方針を入力として実行される

    Scenario: 承認待ちではない step を Approve できない
      Given workflow が waiting_approval ではない
      When ユーザーが Approve する
      Then ワークフロー状態は変わらず `invalid_state` error kind が表示用状態として返される

    Scenario: 空の修正方針案は採用されない
      Given 修正方針確認が空または空白のみの方針案を提示している
      When ユーザーが Approve する
      Then plan_fix または fix は実行されず `validation_error` error kind として承認待ちに留まる

    Scenario: 未完了の方針案は採用されない
      Given 修正方針確認の assistant output が更新中で完了していない
      When ユーザーが Approve する
      Then plan_fix または fix は実行されず `validation_error` error kind として承認待ちに留まる

    Scenario: output contract 不一致の方針案は採用されない
      Given 修正方針確認が `approved-fix-policy` を満たさない方針案を提示している
      When ユーザーが Approve する
      Then plan_fix または fix は実行されず `validation_error` error kind として承認待ちに留まる

    Scenario: 最新方針案を取得できない場合は採用されない
      Given 修正方針確認が承認待ちである
      And 現在の step に属する完了済み assistant output が存在しない
      When ユーザーが Approve する
      Then plan_fix または fix は実行されず `validation_error` error kind として承認待ちに留まる

    Scenario: Approve の重複送信は同じ採用済み方針を一度だけ記録する
      Given Plan 修正方針確認が承認待ちである
      When 同じ承認要求が2回届く
      Then 採用済み Plan 修正方針は1件だけ記録される
      And plan_fix は1回だけ開始される

  Rule: AutoApprove は承認待ちの修正方針案を自動採用する

    Scenario: AutoApprove 有効時に Plan 修正方針案を自動採用する
      Given AutoApprove が有効である
      And Plan 修正方針確認が修正方針案を提示している
      When Plan 修正方針確認が承認待ちになる
      Then ワークフローは Plan 修正方針案を採用済み方針として記録する
      And plan_fix は採用済み Plan 修正方針を入力として実行される

    Scenario: AutoApprove 有効時に実装修正方針案を自動採用する
      Given AutoApprove が有効である
      And 実装修正方針確認が実装修正方針案を提示している
      When 実装修正方針確認が承認待ちになる
      Then ワークフローは実装修正方針案を採用済み方針として記録する
      And fix は採用済み実装修正方針を入力として実行される

    Scenario: workflow approval AutoApprove が無効なら agentAutoApprove が有効でも自動採用しない
      Given workflow approval AutoApprove が未設定または無効である
      And agentAutoApprove が有効である
      And 修正方針確認が有効な方針案を提示している
      When 修正方針確認が承認待ちになる
      Then ワークフローは方針案を採用済み方針として記録しない
      And 修正方針確認は承認待ちに留まる
      And plan_fix または fix は実行されない

    Scenario: AutoApprove は無効な方針案を自動採用しない
      Given AutoApprove が有効である
      And 修正方針確認が空、上限サイズ超過、または output contract 不一致の方針案を提示している
      When 修正方針確認が承認待ちになる
      Then ワークフローは方針案を採用せず承認待ちに留まる
      And plan_fix または fix は実行されない

    Scenario: AutoApprove と手動 Approve が競合しても Fix は一度だけ開始される
      Given AutoApprove が有効である
      And 修正方針確認が有効な方針案を提示している
      When AutoApprove とユーザーの Approve が同じ承認待ちに対して届く
      Then 採用済み方針は1件だけ記録される
      And plan_fix または fix は1回だけ開始される

  Rule: 対話を伴う step は approval に統一される

    Scenario: approval mode の step は実行完了後に承認待ちになる
      Given workflow 定義に mode: approval の step S が存在する
      When S の Agent 実行が完了する
      Then WorkflowState.state.type は waiting_approval になる
      And WorkflowState.currentStepName は S になる

    Scenario: interactive mode は workflow step mode として利用されない
      Given workflow step mode を定義する
      When workflow 定義に mode: interactive が含まれる
      Then `validation_error` error kind が返され workflow は開始されない

    Scenario: approval step に reject 以外または複数の reject rule を定義できない
      Given approval step の rules に match: reject 以外の rule または2件以上の match: reject rule が含まれる
      When workflow 定義を検証する
      Then `validation_error` error kind が返され workflow は開始されない

  Rule: approval step の操作UIは Rust から同期された操作可否で決まる

    Scenario: Reject 可能な場合に Reject が表示される
      Given WorkflowState に `canReject=true` 相当の approval 操作可否が含まれている
      When ユーザーが承認待ちUIを表示する
      Then Approve と Reject が表示される

    Scenario: Reject 不可の場合に Reject は表示されない
      Given WorkflowState に `canReject=false` 相当の approval 操作可否が含まれている
      When ユーザーが承認待ちUIを表示する
      Then Approve が表示される
      And Reject は表示されない

    Scenario: reject rule がない step に Reject command が届いても遷移しない
      Given approval step が承認待ちで Reject 不可である
      When Reject command が届く
      Then ワークフロー状態は変わらず `invalid_state` error kind が表示用状態として返される

    Scenario: Reject は採用済み方針を作らず reject rule の遷移先へ進む
      Given approval step が承認待ちで Reject 可能である
      When ユーザーが Reject comment を入力して Reject する
      Then 採用済み方針は記録されない
      And plan_fix または fix は実行されない
      And workflow は reject rule の遷移先へ進む

    Scenario: 空または空白のみの Reject comment は送信できない
      Given approval step が承認待ちで Reject 可能である
      When ユーザーが空または空白のみの Reject comment で Reject する
      Then ワークフロー状態は変わらず `validation_error` error kind が表示用状態として返される

    Scenario: 上限を超える Reject comment は送信できない
      Given approval step が承認待ちで Reject 可能である
      When ユーザーが 8192 文字を超える Reject comment を送信する
      Then ワークフロー状態は変わらず `validation_error` error kind が表示用状態として返される

  Rule: 修正方針確認の状態は workflow trace に表示される

    Scenario: 承認待ちの修正方針確認が表示される
      Given 修正方針確認が承認待ちである
      When ユーザーが workflow trace を表示する
      Then workflow trace に修正方針確認が承認待ちとして表示される

    Scenario: 採用済み修正方針が表示される
      Given 修正方針が採用済みである
      When ユーザーが workflow trace を表示する
      Then workflow trace に採用済み修正方針が表示される

    Scenario: 機密値を含む方針案を採用しても同期と永続ログに平文を残さない
      Given 修正方針案の policy に `password=secret123`、`ghp_abcdefghijklmnopqrstuvwxyz1234567890`、`-----BEGIN PRIVATE KEY-----abc-----END PRIVATE KEY-----`、`MY_TOKEN_VALUE_123456` が含まれている
      And `MY_TOKEN_VALUE_123456` はマスク対象として登録された環境変数値である
      When ユーザーが Approve する
      Then workflow variables、workflow log、remote sync payload、trace 表示では各機密値が `[REDACTED]` に置換される
      And plan_fix または fix には `[REDACTED]` に置換済みの採用済み方針だけが渡される

  Rule: 承認操作は現在の承認待ち step だけを対象にできる

    Scenario: 別 worktree の承認操作は拒否される
      Given worktree A の workflow が承認待ちである
      When worktree B を対象に Approve または Reject command が届く
      Then worktree A のワークフロー状態は変わらず `unauthorized_worktree` error kind が表示用状態として返される

    Scenario: 同じ worktree の別 execution または別 step の承認操作は拒否される
      Given worktree A の workflow が execution_id E、current_step S で承認待ちである
      When worktree A の execution_id E2 または step T を対象に Approve または Reject command が届く
      Then worktree A のワークフロー状態は変わらず `unauthorized_approval_target` error kind が表示用状態として返される

    Scenario: 終了済み状態の workflow へ Reject できない
      Given workflow が completed、failed、または aborted である
      When Reject command が届く
      Then ワークフロー状態は変わらず `invalid_state` error kind が表示用状態として返される

    Scenario: 過去 run または別 session の方針案は採用されない
      Given 修正方針確認が承認待ちである
      And 最新方針案が別 session または過去 run に属している
      When ユーザーが Approve する
      Then plan_fix または fix は実行されず `validation_error` error kind として承認待ちに留まる
```

## アーキテクチャ概要

### 責務配置
- `src-tauri/src/workflow/schema.rs`: workflow YAML の構造、step mode、transition rule、collect/aggregate、output contract 参照の表現を担当する / 承認操作の実行判断や UI 表示条件は担当しない
- `src-tauri/src/workflow/validation.rs`: workflow 定義の整合性、`mode: interactive` の `validation_error`、approval step の `match: reject` rule 最大1件制約、承認 step と遷移先の接続妥当性を担当する / 実行中の状態遷移、ユーザー操作、output contract 固有の structured output 検証は担当しない
- `src-tauri/src/workflow/builtin.rs`: `approved-fix-policy` output contract facet を組み込み facet として登録し、workflow 定義から参照できるようにする / contract 固有の値検証や Agent 向け instruction は担当しない
- `src-tauri/src/workflow/builtin/spec-driven-development.yml`: spec-driven-development のレビュー結果から修正方針確認、Fix、再レビューへ戻る経路、修正方針確認 step の `approval` mode、reject rule、`approved-fix-policy` output contract、`plan_fix` / `fix` の必須入力を定義する / 実行時に採用済み方針を保持することは担当しない
- `src-tauri/src/workflow/builtin_facets/output_contracts/approved-fix-policy.md`: `approved-fix-policy` の structured output が持つ `policy` と `review_step` の意味を output contract facet として定義する / Agent 会話や状態遷移は担当しない
- `src-tauri/src/workflow/builtin_facets/instructions/`: Plan 修正方針確認、実装修正方針確認、Fix step に渡す入力の意味を Agent 向け指示として定義する / output contract の登録・検証、分岐判定、状態保存は担当しない
- `src-tauri/src/workflow/contract.rs`: `validate_approved_fix_policy` で `approved-fix-policy` の必須フィールド、型、空文字、サイズ上限を contract-specific に検証する / workflow 定義の step 接続や UI 表示条件は担当しない
- `src-tauri/src/workflow/engine.rs`: workflow 実行状態、step 遷移、approval 待ち、Approve/Reject、AutoApprove、`StepOutput.structured_output` への採用済み方針の記録、後続 step への出力注入、入力検証、機密値マスク、冪等制御を担当する / React 表示や YAML 永続化の詳細は担当しない
- `src-tauri/src/workflow/state.rs` と `src-tauri/src/workflow/log.rs`: workflow trace に必要な実行スナップショット、履歴、採用済み方針、ログイベントの表現を担当する / 状態遷移ルールの評価は担当しない
- `src-tauri/src/workflow/commands.rs`: React から workflow 実行・承認・差し戻し・中断・履歴取得を呼ぶ Tauri command 境界と worktree / execution / step の認可を担当する / 承認可否や次 step 判定のロジックは担当しない
- `src/types/workflow.ts`: Rust からシリアライズされる workflow 型の同期を担当する / TypeScript 側で workflow ロジックを再実装しない
- `src/components/panels/AgentChatPanel/WorkflowPanel/WorkflowPanel.tsx`: approval 待ちの操作 UI、Rust から同期された approval 操作可否に基づく Reject 表示、AutoApprove による通過後の状態反映を担当する / reject rule の有無や採用済み方針の判定は担当しない
- `src/components/panels/AgentChatPanel/WorkflowPanel/WorkflowTrace.tsx`: 修正方針確認 step の承認待ち状態、採用済み方針、後続 Fix への入力関係を表示する / 表示用整形を超える状態計算や遷移判断は担当しない
- `src/remote/hooks/useRemoteWorkflowState.ts` と remote 表示層: WebSocket 経由で同期された `WorkflowState` の表示を担当する / desktop 側と別の workflow 実行ロジックは持たない

### データ/通信フロー
- Plan review が `NEEDS_FIX` の場合: review child steps → aggregate result → `WorkflowEngine` の遷移判定 → Plan 修正方針確認 approval step → `workflow_state_sync` / session state persist → UI が承認待ちを表示
- Code review が `NEEDS_FIX` の場合: code review aggregate result → `WorkflowEngine` の遷移判定 → 実装修正方針確認 approval step → `workflow_state_sync` / session state persist → UI が承認待ちを表示
- 並列レビューが `NEEDS_FIX` の場合: collect/aggregate 済みの全レビュー結果と NEEDS_FIX findings → 修正方針確認 approval step → Agent が方針案を提示
- 承認待ち中のチャット調整: UI の既存 Agent chat 入力 → 現在の approval step session → Agent が更新済み方針案を返す → `WorkflowEngine` は同じ worktree / execution / step / session に属する完了済み最新 assistant output だけを承認対象として扱う
- Approve: UI → approval Tauri command → `commands.rs` が worktree / execution / step を認可 → `WorkflowEngine` → 最新方針案を `validate_approved_fix_policy` で検証して機密値をマスク → `approved-fix-policy` の `StepOutput.structured_output` と history に採用済み方針として1回だけ記録 → `plan_fix` または `fix` の prompt に review result と採用済み方針を注入 → state persist と broadcast
- Reject: UI → reject Tauri command → `commands.rs` が worktree / execution / step を認可 → `WorkflowEngine` → Reject comment を検証 → 現在 step が Reject 可能な場合だけ reject rule の `next` へ遷移 → 採用済み方針は記録しない → state persist と broadcast
- AutoApprove: approval step が承認待ちになったタイミング → `WorkflowEngine` が workflow approval 専用 AutoApprove 設定を確認 → 方針案を Approve と同じ検証・マスク・冪等処理で採用済み方針として記録 → 人間の操作なしに後続 Fix step へ進める → trace には approval 境界と採用済み方針を残す
- Trace 表示: `WorkflowState` / `WorkflowLogEvent` → Tauri command または WebSocket sync → desktop / remote の WorkflowTrace → approval step の状態、result、structured output、採用済み方針、approval 操作可否を表示

### 状態Owner
- workflow 実行状態 (`running`, `waiting_approval`, `completed`, `failed`, `aborted`): `WorkflowEngine` / `WorkflowExecution`
- 現在 step、step 実行回数、cycle guard、次 step 遷移: `WorkflowEngine`
- workflow 定義と mode 解釈: Rust の `schema.rs` / `validation.rs`
- 修正方針確認 step の承認待ち状態: `WorkflowEngine` の `WorkflowExecutionState::WaitingApproval`
- approval step の最新方針案: 現在の worktree / execution / step / Agent session に属する完了済み最新 assistant output を正本とし、採用前は `WorkflowEngine` が session 参照で扱う
- 採用済み Plan 修正方針 / 採用済み実装修正方針: `WorkflowEngine` が `approved-fix-policy` の `StepOutput.structured_output` と `step_history` に保存する
- review result と aggregate result: `WorkflowEngine` の output contract 検証、collect/aggregate、StepOutput
- Reject 可否と reject 遷移先: approval step の rules を `WorkflowEngine` が評価した結果。`WorkflowState` は React が `canReject=true/false` 相当の解釈済み操作可否として使える形で同期する
- AutoApprove 設定: Rust 側の workflow 実行ロジックが読む workflow approval step 専用設定を Owner とし、UI は設定変更と現在値表示だけを担当する。`agentAutoApprove` は対象外とする
- Trace 表示用状態: Rust の `WorkflowState` / `WorkflowLogEvent` が正本、React は派生表示のみを持つ
- remote 側 workflow 状態: desktop 側 `WorkflowState` の WebSocket 同期結果が正本で、remote はローカル実行状態を持たない

### 境界
- フロントエンドは approval / reject / abort のユーザー入力を Tauri command に渡すだけにし、`NEEDS_FIX` 判定、AutoApprove、採用済み方針の選択、Fix への入力注入を実装しない
- `interactive` は workflow step mode として既存・新規を問わず無効にし、ユーザー確認・成果物確認・チャット調整・承認待ちは `approval` に統一する
- `approval` step の `rules` は Reject 操作用の差し戻し遷移だけを表し、0件または1件の `match: reject` rule だけを定義できる。1件ある場合に Reject 可能、0件の場合に Reject 不可とし、`match: reject` 以外の rule または2件以上の rule は `validation_error` にする
- review から Fix への直接遷移は禁止し、`NEEDS_FIX` は必ず修正方針確認 approval step を経由する
- Reject はチャット調整ではなく、workflow 定義に reject rule がある step だけで使える明示的な差し戻し遷移として扱う
- Reject button の表示条件は Rust から同期された `canReject=true/false` 相当の approval 操作可否に限定し、UI は workflow definition や `rules` を解釈しない
- AutoApprove が有効でも approval 境界、StepOutput、trace 表示は省略せず、後続 Fix step には人間 Approve 時と同じ形で採用済み方針を渡す
- 採用済み方針は `approved-fix-policy` output contract の `StepOutput.structured_output` として後続 step に渡し、Fix step が review 生データだけから修正方針を推測しない
- `approved-fix-policy` の structured output は `{ "policy": string, "review_step": string }` を必須とし、`policy` は trim 後1文字以上かつ UTF-8 で 65536 bytes 以下、`review_step` は方針の元になった review / aggregate step id とする。未知の output contract として JSON 通過させず、`contract.rs` の `validate_approved_fix_policy` で検証する
- Approve / AutoApprove は現在 waiting_approval の step に紐づく最新出力だけを対象にし、空の方針案、上限サイズ超過、未完了出力、output contract 不一致、別 session または過去 run の出力は採用しない。失敗時は `invalid_state`、`validation_error`、`unauthorized_worktree`、`unauthorized_approval_target` のいずれかの安定した error kind を表示用状態に含める
- Reject comment と承認待ち中のチャット追加入力は 8192 文字以下に制限する。Reject comment は必須で、trim 後1文字未満の場合は `validation_error` にし、Reject 遷移は実行しない
- 承認系 command は指定された worktree の現在実行中 workflow が waiting_approval で、対象 execution / step が現在の承認待ちと一致する場合のみ成功する。他 worktree、完了済み execution、running / failed / aborted 状態、現在 step と異なる承認操作は拒否し、表示用状態には `unauthorized_worktree`、`unauthorized_approval_target`、`invalid_state` のいずれかを含める
- 採用済み方針として保存・同期・後続 Fix へ注入する値は Fix に必要な方針情報に限定し、マスク対象に該当する値は `[REDACTED]` に置換した値だけを workflow log、workflow variables、remote sync payload、trace 表示、`plan_fix` / `fix` 入力に含める。マスク対象は AppConfig に保存された token 類、現在プロセスの環境変数値のうち8文字以上の値、PEM 秘密鍵ブロック、`api_key` / `apikey` / `token` / `password` / `secret` 形式の key-value 値、GitHub token 形式の `ghp_` / `github_pat_` で始まる値とする
- workflow trace と remote sync は Rust の状態をそのまま可視化する境界であり、表示のために不足する情報は Rust の状態モデルに追加して同期する

### 実装に委ねること
- 修正方針確認 step の具体的な YAML step 名、instruction facet 名、表示ラベル
- AutoApprove 設定の具体的な設定キー名、保存場所、既存設定 UI への配置
- approval step の最新 assistant output を取得・検証・採用する helper 関数の名前と分割
- Reject 可否を `WorkflowState` に同期する派生フィールド名
- 機密値マスクの具体的な検出 helper 名とログイベント上の表現
- WorkflowTrace 内での採用済み方針のコンポーネント分割、折りたたみ、JSON/Markdown 表示の細部
- desktop と remote の表示差分の細かい UI 構成
- 既存テスト構成に沿った単体テスト・コンポーネントテスト・remote hook テストの具体的な配置
