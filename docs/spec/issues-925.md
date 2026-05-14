## 要求

**種別**: バグ修正

**ゴール**:
- ワークフローパネルにおいて、Approval ステップの承認系アクション（Approve / Reject）を、現在のようにパネル最上部の固定アクションバーではなく、対応するステップ行内（`WorkflowTrace` の該当 `TraceItem`）に表示する。
- 並列ブロック実行中の承認（親ブロック単位・子ステップ単位の双方）は本Issueの対象外とし、後続Issueで扱う。現行バックエンドでは並列ブロック親ステップが `waiting_approval` 状態に遷移しないため、フロントエンドの行内承認UIだけを変更しても並列ブロックには適用できない。本Issueにバックエンド変更を含めるとスコープを大きく超えるため切り離す。
- 同一ステップが cycle_guard やループ遷移により複数回実行されるケースでは、`occurrence` 番号で識別される「現在実行中の occurrence」の行にのみアクションボタンを表示し、過去 occurrence の完了行には表示しない。
- ワークフロー全体に対するアクション（Stop = `abort_workflow`）は、これまで通りワークフロー単位のため、ステップ行内ではなくパネル上部に残す。Stop はステップに紐づかないので今回の移行対象には含めない。
- 結果として「いまどのステップに対して操作しているのか」が画面上のボタン位置から一意に判別でき、複数 occurrence ステップとの紐づけもボタンの配置位置のみで識別可能となる。

**スコープ確定（対象外の明示）**:
- 本Issueの実装・テスト対象は **通常（非並列）Approval ステップの Approve / Reject の行内配置のみ**。
- Interactive ステップの Complete は、Interactive モード自体が #944 (c6dcfa0) でビルトインから廃止済みであり、TypeScript の `StepMode` も `auto | approval` のみ、Rust 側 validation も `mode: interactive` を拒否する。よって Interactive Complete は **本Issueの実装・テスト対象外**（背景説明としてのみ登場する）。
- 並列ブロック内の承認（親ブロック行内承認・子ステップ個別承認の双方）は **本Issueの実装・テスト対象外**。現行バックエンド (`src-tauri/src/workflow/validation.rs`) は並列ブロック内子ステップを `mode: auto` のみに制限しており、並列親ステップに対する `mode: approval` 指定も受け付けない。そのため並列ブロックが `waiting_approval` 状態に遷移するケースは現行仕様では発生せず、本Issueは並列ブロック行内に承認UIを描画しない（境界 Scenario で回帰防止を担保）。並列ブロック承認はバックエンド拡張を伴うため後続Issueで扱う。
- Stop（ワークフロー全体中断）は移行対象ではなく現状維持。

**背景**:
- Issue #925 で「Interactive ステップの Complete ボタンがステップではなくワークフローの位置に表示されている」と報告されている。Interactive モード自体は #944 (c6dcfa0) でビルトインから廃止されたが、Approval ステップの Approve/Reject ボタンも全く同じ構造（パネル上部のアクションバー内に配置）で実装されており、根本的には同一の UI 設計上の不具合が残存している。
- 現状 `WorkflowActivePanel`（`src/components/panels/AgentChatPanel/WorkflowPanel/WorkflowPanel.tsx` L406-584）はステップ単位の Approve/Reject をパネル最上部のアクションバー（L534-572）に置いており、`WorkflowTrace` で描画される「どのステップ行に対するアクションか」が視覚的に紐付かない。これが以下の具体的な課題を生んでいる:
  - **どのステップに対するアクションか不明瞭**: トレースを下にスクロールしてステップ詳細を確認している際、ボタンは画面上部に固定されているため、操作対象のステップとボタンの空間的距離が遠く、関連付けが直感的に取れない。
  - **複数回実行されるステップとの紐づけが不明**: cycle_guard やループにより同名ステップが複数 occurrence 実行される際、どの occurrence に対する承認なのかボタン位置からは判別できない。
- 根本解決として、ステップに紐づくアクション（Approve / Reject / 旧 Complete）はステップ行内に配置し、ワークフローに紐づくアクション（Stop）のみ上部に残す、という責務分離を行う。これにより本 Issue（Interactive 由来の構造的不具合）と Approval ステップが現在抱える同種の問題、および複数 occurrence ステップへの紐づけ不明を同時に解消する。並列ブロック親・子ステップに対する承認はバックエンドAPIの拡張を伴うため別Issueで扱う。

## 振る舞い定義

```gherkin
Feature: ワークフロー承認系アクションのステップ行内配置

  対象ステップに紐づく承認系アクション（Approve / Reject）を、
  パネル最上部の固定アクションバーではなく、操作対象のステップ行内に配置する。
  ワークフロー全体に紐づく Stop アクションのみパネル上部に残す。
  注: Interactive ステップの Complete は #944 (c6dcfa0) でビルトインから廃止済みであり、
  TypeScript の StepMode は auto|approval のみ、Rust 側 validation も mode: interactive を
  拒否するため、本 Feature では実装・テスト対象に含めず背景説明としてのみ参照する。

  Rule: ステップ行内アクションによるワークフロー進行
    Scenario: 承認待ちステップ行内の Approve でステップが承認される
      Given ワークフローの currentStepName が "review" であり state.type が "waiting_approval" である
      When ユーザーが "review" ステップ行内の Approve ボタンを押す
      Then approve_workflow_step が decision: "approve" で呼び出される
      And 状態遷移後、currentStepName が次ステップ "implement" となる
      And 行内に表示されていた Approve / Reject ボタンは消える

    Scenario: 承認待ちステップ行内の Reject でステップが拒否される
      Given ワークフローの currentStepName が "review" であり state.type が "waiting_approval" である
      And "review" ステップの rules に match: "reject" / next: "fix" が定義されている
      When ユーザーが "review" 行内の Reject ボタンから "needs fix" というコメントを添えて送信する
      Then approve_workflow_step が decision: { reject: { comment: "needs fix" } } で呼び出される
      And stepHistory に追加された "review" エントリの result が "reject" となる
      And currentStepName が match: reject の遷移先 "fix" となる
      And 行内に表示されていた Approve / Reject ボタンは消える

    Scenario: Approve が API エラーで失敗した場合に行内へエラーが表示される
      Given ワークフローの currentStepName が "review" で state.type が "waiting_approval" である
      When ユーザーが "review" 行内の Approve ボタンを押し、approve_workflow_step が拒否エラー（例: 承認競合・既に他クライアントが承認済み）を返す
      Then useStepApprovalAction の approvalError が更新される
      And "review" 行内に approvalError のメッセージが表示される
      And 当該ステップ行は承認待ち表示のまま維持され、サイレントに失敗しない

    Scenario: Reject が API エラーで失敗した場合に行内へエラーが表示される
      Given ワークフローの currentStepName が "review" で state.type が "waiting_approval" である
      When ユーザーが "review" 行内の Reject にコメントを添えて送信し、approve_workflow_step がエラーを返す
      Then useStepApprovalAction の approvalError にエラーメッセージが保持される
      And "review" 行内にエラーメッセージが表示され、Reject 入力欄は維持される（再送信が可能である）

    Scenario: Reject コメントが空または空白のみの場合は送信不可
      Given ワークフローの currentStepName が "review" で state.type が "waiting_approval" である
      And ユーザーが "review" 行内の Reject 入力欄を開き、コメントが空または空白のみ（例: "", "   ", "\n"）である
      When ユーザーが Reject 送信を試みる
      Then 送信ボタンは disabled である
      And approve_workflow_step は呼び出されない

    Scenario: Reject コメントが上限（8192 文字）を超える場合は行内にエラーが表示される
      Given ワークフローの currentStepName が "review" で state.type が "waiting_approval" である
      And ユーザーが "review" 行内の Reject 入力欄に 8192 文字を超えるコメントを入力している
      When ユーザーが Reject 送信を実行する
      Then approve_workflow_step がバックエンド側 validation により拒否エラーを返す
      And useStepApprovalAction の approvalError にエラーメッセージが保持される
      And "review" 行内にエラーメッセージが表示される
      And Reject 入力欄は維持され、入力内容も保持され、再送信が可能である

  Rule: ワークフロー全体アクションによる中断
    Scenario: パネル上部の Stop でワークフロー全体が中断される
      Given ワークフローが実行中または承認待ち状態である
      When ユーザーがパネル上部の Stop ボタンを押す
      Then abort_workflow が呼び出され、実行中のワークフロー全体が中断される

    Scenario: Stop が API エラーで失敗した場合にパネル上部にエラーが表示される
      Given ワークフローが実行中または承認待ち状態である
      When ユーザーがパネル上部の Stop ボタンを押し、abort_workflow がエラーを返す
      Then パネル上部にエラーメッセージが表示される
      And ワークフローは中断されないまま継続表示され、サイレントに失敗しない

  Rule: ステップに紐づく承認系アクションの表示位置
    Scenario: 承認待ちの通常ステップ行にのみ Approve/Reject が表示される
      Given ワークフローの currentStepName が "review"、state.type が "waiting_approval"、approvalOperations.canReject が true である
      And "review" の TraceItem が kind: "current" として描画されている
      When ユーザーがワークフローパネルを閲覧する
      Then "review" の TraceItem 行内にのみ Approve / Reject ボタンが表示される
      And パネル最上部のアクションバーには承認系ボタンが表示されない

    Scenario: 並列ブロック行内には承認系ボタンを描画しない（本Issueでは対象外）
      Given ワークフローパネルに kind: "parallel" の TraceItem（親並列ブロック行）が描画されている
      When ユーザーがワークフローパネルを閲覧する
      Then 当該並列ブロック行（kind: "parallel" の親行）内には承認系ボタンが表示されない
      And 並列ブロック内の子ステップ行にも承認系ボタンが表示されない

    Scenario: 現在実行中の occurrence 行にのみ承認系ボタンが表示される
      Given workflowState.currentStepName が "review"、state.type が "waiting_approval" である
      And workflowState.stepExecutionCounts["review"] が 2 である
      And buildTraceItems が "review" について 2 行を返し、run 1 は kind: "completed" / occurrence: 1、run 2 は kind: "current" / occurrence: 2 である
      When ユーザーがワークフローパネルを閲覧する
      Then "review" の run 2（kind: "current" / occurrence: 2）の行にのみ承認系ボタンが表示される
      And "review" の run 1（kind: "completed" / occurrence: 1）の行には承認系ボタンが表示されない

    Scenario: 拒否が許可されていない場合は Reject ボタンが表示されない
      Given ワークフローの currentStepName が "review"、state.type が "waiting_approval"、approvalOperations.canReject が false である
      And "review" の TraceItem が kind: "current" として描画されている
      When ユーザーがワークフローパネルを閲覧する
      Then "review" の TraceItem 行内には Approve ボタンのみが表示され、Reject ボタンは表示されない

  Rule: ワークフローに紐づくアクションの表示位置
    Scenario: ワークフロー実行中はパネル上部に Stop ボタンが表示される
      Given ワークフローが実行中または承認待ち状態である
      When ユーザーがワークフローパネルを閲覧する
      Then パネル上部に Stop ボタンが表示される
      And Stop ボタンはステップ行内には表示されない
```

## アーキテクチャ概要

### 責務配置
- `WorkflowPanel.tsx` / `WorkflowActivePanel`: タブ管理・パネル上部アクションバーの描画・ワークフロー全体アクション（Stop）の発火、および Stop API 呼び出し失敗時のエラー（`abortError`）の保持・パネル上部での表示を担当。ステップ単位の承認系UI状態・API呼び出しは担当しない。
- `WorkflowTrace.tsx`（`TraceItemRow` / `ParallelBlockRow`）: ステップ行のレイアウト描画と「この行が現在の承認対象か」の判定を担当。承認アクションUI状態の保持は担当しない。判定条件は既存 `TraceItem` 型に整合させ、以下のとおりとする:
  - 共通条件: `workflowState.state.type === "waiting_approval"` かつ `item.stepName === workflowState.currentStepName`。
  - 通常ステップ行: 上記共通条件に加えて `item.kind === "current"` の行が現在の承認対象であり、その行内に承認系UIを描画する。
  - 並列ブロック行（`item.kind === "parallel"`）: 本Issueでは承認対象外とし、行内に承認系UIを一切描画しない。現行バックエンドでは並列親ステップが `waiting_approval` 状態に遷移しないため、フロントエンド単独で並列ブロック内承認を実現できない。並列ブロック承認の対応は後続Issueで扱う。
  - 過去 occurrence（`item.kind === "completed"` の行）: 承認系UIを描画しない。
- 新規カスタムフック `useStepApprovalAction`（`src/hooks/` 配下、Rust側ロジックは触らないReactレイヤー専用）: 行内承認UIから使う承認アクションUI状態（rejectMode / rejectComment / approvalError）と、`approve_workflow_step` Tauriコマンド呼び出しを担当。承認可否そのものの判定や、ワークフロー進行ロジックは担当しない。
- Tauriバックエンド（`src-tauri/src/workflow/`）: 承認対象の妥当性検証・承認/拒否の適用・`approvalOperations.canReject` の算出・ワークフロー状態遷移・`abort_workflow`。今回スコープでは変更なし。

### データ/通信フロー
- ワークフロー状態の取得: Rust `WorkflowEngine` → イベント/Tauriコマンド → `WorkflowState` → `WorkflowActivePanel` → `WorkflowTrace` で `buildTraceItems` により行列を構築 → 現在承認待ちの行を判別。
- Approve操作: ステップ行内Approveボタン → `useStepApprovalAction` → `invoke("approve_workflow_step", { worktreePath, executionId, stepName, decision: "approve" })` → Rust側で状態遷移 → 新しい `WorkflowState` がUIへ反映。
- Reject操作: 行内Rejectボタンで reject mode を ON → コメント入力 → Submit → `useStepApprovalAction` → `invoke("approve_workflow_step", { worktreePath, executionId, stepName, decision: { reject: { comment } } })`。エラー時は `approvalError` をフックが保持し行内に表示。
- Stop操作（変更なし）: パネル上部Stopボタン → `WorkflowActivePanel` → `invoke("abort_workflow", { worktreePath })`。

### worktree-scoped command context の受け渡し契約
- `worktreePath` および `executionId` は `WorkflowActivePanel` が所有する（既存と同じ）。
- 承認系アクションを行内で発火するため、`WorkflowActivePanel` はこれらを「承認アクション用 context」として `WorkflowTrace` 経由で行コンポーネント（`TraceItemRow`）へ渡す。
- 既存 `WorkflowTraceProps` を拡張し、`approvalAction?: { worktreePath: string; executionId: string }` を **optional props** として追加する。これは行内承認UIの描画と Tauri command 発火に必要な context をひとまとめにし、`WorkflowTrace` を承認 context 不要の場面（履歴表示等）でも従来通り再利用可能にするための契約である。
- 履歴表示用途（例: `ExecutionView` のように完了済みワークフローを閲覧するだけの呼び出し元）では `approvalAction` を **指定しない**。`approvalAction` が未指定（`undefined`）の場合、`WorkflowTrace` および行コンポーネントは行内に承認系UIを一切描画しない契約とする。
- `approvalAction` が指定された場合のみ、行コンポーネントは承認対象判定（共通条件 + `kind === "current"`）を満たす行で `useStepApprovalAction` を呼び出し、`approvalAction` の `worktreePath` / `executionId` をフックに渡す。その他のワークフロー状態（`canReject` 等）は既存どおり `WorkflowState` 由来の派生値として props 経由で受け取る。
- `useStepApprovalAction` は受け取った `worktreePath` / `executionId` / `stepName` を Tauri command 引数に転送するのみで、自前で context を解決しない。

### 状態Owner
- ワークフロー実行状態 (`WorkflowState`、承認可否を含む `approvalOperations`): Rust `WorkflowEngine`。
- 承認アクションUI状態 (rejectMode / rejectComment / approvalError): `useStepApprovalAction` フック（行コンポーネント内で呼び出して保持）。
- Stop アクションのエラー状態 (`abortError`): `WorkflowActivePanel`。パネル上部のアクションバー領域に表示する。
- パネルのタブ表示状態 (activeTab / openPastIds / historyOpen 等): `WorkflowPanel`（変更なし）。
- 「どの行が承認対象か」の派生情報: `WorkflowTrace` の `buildTraceItems`（既存ロジックを継続利用）。本Issueでは通常ステップのみが承認対象であり、`kind === "current"` の行で識別する（並列ブロック行 `kind === "parallel"` は本Issueでは承認対象外）。

### 境界
- 承認系アクションUI（Approve / Reject / Reject comment 入力 / approvalError 表示）は、ステップ行コンポーネントの内部に閉じる。`WorkflowActivePanel` 直下のアクションバーには配置しない。
- パネル上部アクションバーはワークフロー単位アクション（現状は Stop のみ）に限定する。
- 過去 occurrence の行には承認系UIを描画しない。「現在実行中の occurrence」の判定は `buildTraceItems` が返す `item.kind === "current"` で識別する（履歴由来の通常ステップ行は `kind === "completed"` となるため、`kind` 一致で一意に判別可能）。行コンポーネント内で独自の occurrence 比較ロジックや stepHistory への参照は持ち込まず、`buildTraceItems` が返す `kind` / `stepName` と上位から渡される `workflowState` のフィールド（`currentStepName` / `state.type`）の比較のみで判定する。
- 並列ブロック行（`kind === "parallel"`）は本Issueでは承認対象外とし、親行・子ステップのいずれにも承認系UIを描画しない。並列ブロック承認はバックエンドAPI拡張を伴うため後続Issueで扱う。
- `useStepApprovalAction` は Tauriコマンドを直接 invoke するが、「承認待ちか」「Rejectが許可されているか」の判定は引数で受け取り、自前で `WorkflowState` を解釈しない。
- `approvalAction` props が未指定の `WorkflowTrace` 呼び出し（履歴表示等）では、承認系UIを行内に描画しない。これにより `WorkflowTrace` を承認 context 不要の場面でも再利用可能とする。

### 実装に委ねること
- 行内承認UIの具体的なレイアウト（ボタン配置順、Reject comment 入力欄の展開方向、エラーメッセージの表示位置・色味）。
- `useStepApprovalAction` の関数シグネチャ詳細（引数のまとめ方、返り値オブジェクトのキー名、リセットトリガーの渡し方）。
- ステップ行の承認UIをサブコンポーネントに切り出すか、`TraceItemRow` / `ParallelBlockRow` 内にインライン展開するかの粒度。
- Reject comment 入力時のキーボードショートカット（Cmd+Enter で送信等）の有無。
- テストケースの具体的配置（`WorkflowPanel.test.tsx` 既存ファイルに追記するか、`useStepApprovalAction.test.ts` を新設するか）。
- 既存 `WorkflowActivePanel` 内のアクションバー関連ステート/ハンドラ（`rejectMode`, `rejectComment`, `approvalError`, `handleApprove`, `handleReject*`）の最終的な削除/移譲の細部。

