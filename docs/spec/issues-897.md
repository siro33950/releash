## 要求

**種別**: 新機能
**ゴール**: Workflow の approval step で Reject する際に、人間が差し戻し理由・修正指示をコメントとして入力でき、そのコメントが後続の fix step へ渡されることで、自動修正サイクルが人間の指摘を起点に再開できるようにする。
**背景**: 現状の approval は approve / reject / abort の判定のみで、Reject 理由を Workflow Engine に渡せない。Spec 承認や実装結果承認で人間が Reject しても、次の auto step が何を修正すべきかを構造的に受け取れない。
**利用シーン**: Spec 承認・実装結果承認等の approval step で Reject → fix step への差し戻しフロー
**制約**:
- #896（Step Output / Collect / Reduce）の output 永続化・prompt 注入基盤が前提
- Approve / Abort の既存挙動を維持すること
**影響範囲**: ApprovalDecision 型、Tauri コマンド、Frontend UI、StepOutput、Trace view

**受け入れ基準**:
- approval step で Reject する際、空コメントでは Reject できない
- Reject comment 付きで Reject すると、workflow は reject rule の遷移先へ進む
- 遷移先 step の prompt に Reject comment が注入される
- Step history / Trace view で Reject comment を確認できる
- NDJSON ログ再構築後も Reject comment が復元される
- 既存の Approve / Abort フローが壊れていない

**非スコープ**:
- ビルトイン Workflow の追加
- レビュー観点ごとの並列実行
- approval 以外の human feedback 汎用化

## 振る舞い定義

```gherkin
Feature: Approval Rejectコメント付き差し戻し
  Workflow の approval step で人間が Reject する際にコメントを入力し、
  後続の fix step が人間の指摘を起点に修正サイクルを再開できる。

  Rule: Reject時にコメントが必須である
    Scenario: コメント付きでRejectする
      Given approval step が WaitingApproval 状態である
      When ユーザーがコメント付きで Reject する
      Then ワークフローは reject ルールの遷移先へ進む

    Scenario: 空コメントではRejectできない
      Given approval step が WaitingApproval 状態である
      When ユーザーが空コメントで Reject しようとする
      Then Reject は拒否される

  Rule: RejectコメントはStepOutputとして保存される
    Scenario: Rejectコメントがstep outputに記録される
      Given approval step が WaitingApproval 状態である
      When ユーザーがコメント付きで Reject する
      Then step の output_text にRejectコメントが保存される
      And step の result が "reject" として保存される

  Rule: 遷移先stepがRejectコメントをprompt contextとして受け取る
    Scenario: pass_previous_responseで前stepのRejectコメントを受け取る
      Given approval step が Reject コメント付きで完了している
      And 遷移先 step に pass_previous_response が true で設定されている
      When 遷移先 step が起動される
      Then 遷移先 step の prompt に Reject コメントが注入される

    Scenario: pass_output_fromで任意のapproval stepのRejectコメントを受け取る
      Given approval step が Reject コメント付きで完了している
      And 遷移先 step に pass_output_from で approval step 名が指定されている
      When 遷移先 step が起動される
      Then 遷移先 step の prompt に Reject コメントが注入される

  Rule: Rejectコメントは追跡可能である
    Scenario: Trace viewでRejectコメントを確認する
      Given approval step が Reject コメント付きで完了している
      When ユーザーが Trace view を表示する
      Then Reject コメントが表示される

    Scenario: NDJSONログからRejectコメントを復元する
      Given Reject コメント付きで完了した approval step のログが存在する
      When ログから WorkflowState を再構築する
      Then 復元された StepOutput に Reject コメントが含まれる

  Rule: Approve/Abortの既存挙動は維持される
    Scenario: Approveは従来通りコメントなしで動作する
      Given approval step が WaitingApproval 状態である
      When ユーザーが Approve する
      Then ワークフローは次のステップへ進む

    Scenario: Abortは従来通りコメントなしで動作する
      Given approval step が WaitingApproval 状態である
      When ユーザーが Abort する
      Then ワークフローは中止される
```

## 実装仕様

**対応方針**: 振る舞い定義を実現するために、ApprovalDecision型・Tauriコマンド・エンジン・フロントエンドUIに対して、Reject時のcommentパラメータ追加で対応する。Rejectコメントは既存の `StepOutput.output_text` に格納し、`pass_previous_response` / `pass_output_from` の既存基盤をそのまま活用する。

**対象コンポーネント**:

### Rust側

1. **`engine.rs` — ApprovalDecision enum変更**
   - `Reject` variant に `comment: String` フィールドを追加
   - `Reject { comment: String }` （tagged enum、serde rename_all = "snake_case"）

2. **`engine.rs` — handle_approval()変更**
   - Reject時、`fetch_current_output()` および `decide_approval_action()` の前に `comment.trim().is_empty()` をチェックし、空の場合は `WorkflowEngineError` を返して処理を中断する（不要な副作用を避けるため最初にバリデーション）
   - バリデーション通過後、`output_text` を `fetch_current_output()` の結果ではなく、`decision` から取り出した `comment`（trim前の原文）に差し替える
   - Approve は既存どおり `fetch_current_output()` の結果を `output_text` として履歴に保存する
   - Abort は既存どおり履歴エントリを追加せず、状態を Aborted に遷移するのみ（`output_text` の保存なし）

3. **`engine.rs` — decide_approval_action()変更**
   - 変更不要。`ApprovalDecision::Reject { .. }` のパターンマッチ更新のみ

4. **`commands.rs` — approve_workflow_step コマンド**
   - パラメータ変更不要。`ApprovalDecision` のserde deserializeで `{ "reject": { "comment": "..." } }` を受け取る

### フロントエンド側

5. **`WorkflowPanel.tsx` — Reject UI**
   - Reject ボタン押下でコメント入力UIを表示（テキストエリア + 送信/キャンセル）
   - 空コメントでは送信ボタンを disabled にする
   - `invoke("approve_workflow_step", { worktreePath, decision: { reject: { comment } } })` で送信

6. **`WorkflowTrace.tsx` — 表示**
   - 変更不要。既存の `outputText` 表示で Reject コメントが表示される（`result: "reject"` で区別可能）

### ログ・永続化

7. **`log.rs`**
   - 変更不要。`StepCompleted` の `output_text` と `result` フィールドにRejectコメントと "reject" がそのまま記録される

8. **`state.rs`**
   - 変更不要。`StepOutput` / `StepHistoryEntry` の既存フィールドで対応可能

**検討した代替案**:
- StepOutput / StepHistoryEntry に `reject_comment` フィールドを新設する案 → `output_text` に格納すれば `pass_previous_response` / `pass_output_from` / inject_step_outputs / NDJSON復元がすべて既存実装のまま動作するため却下。専用フィールドを追加すると全層（型定義・ログ・復元・表示）に変更が波及する

**影響するテスト**:
- Rust: `engine.rs` の既存テスト（approval判定テスト）を `Reject { comment }` に更新
- Rust: Reject時の `output_text` がコメントになることのテスト追加
- Rust: 空コメントRejectのバリデーションテスト追加
- Rust: `pass_previous_response` でRejectコメントが次stepのpromptに注入されるテスト追加
- Rust: `pass_output_from` で任意stepのRejectコメントが注入されるテスト追加
- Rust: `StepCompleted` ログからRejectコメント（`output_text` + `result: "reject"`）が復元されるテスト追加
- Rust: Approve時に `handle_approval()` が `output_text` を履歴に保存しadvanceするテスト追加
- Rust: Abort時に `handle_approval()` が `step_history` を増やさず Aborted に遷移するテスト追加
- Frontend: WorkflowPanel のReject UIテスト（コメント入力 → invoke呼び出し）
- Frontend: 空コメント時のdisabled状態テスト
- Frontend: Trace viewでRejectコメント（`result: "reject"` + `outputText`）が表示されるテスト
