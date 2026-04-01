## 要求

**種別**: バグ修正
**現在の挙動**: Plan mode中に `ExitPlanMode` を呼び出すと `<error>Exit plan mode?</error>` が返るのみで承認UIが表示されない。`AskUserQuestion` を呼び出すと `<error>Answer questions?</error>` が返るのみで質問UIが表示されない。
**期待する挙動**: `ExitPlanMode` 呼び出し時にPlan内容が表示されユーザーが承認/却下できるUIが表示される。`AskUserQuestion` 呼び出し時にユーザーに選択肢が表示され回答を選択できるUIが表示される。
**再現手順**:
1. Claude Codeで `EnterPlanMode` を呼び出してPlan modeに入る
2. Planファイルに計画を記述する
3. `ExitPlanMode` を呼び出す → エラーレスポンスのみ返り、承認UIが表示されない
4. `AskUserQuestion` を呼び出す → エラーレスポンスのみ返り、質問UIが表示されない
**背景**: Plan modeから正常に抜けられず実装フェーズに移行できない。ユーザーへの質問ができず計画の確認・修正ができない。Plan modeのread-only制約により、ユーザーが手動でPlan modeを解除するしかない状態。
**影響範囲**: Releash経由のClaude Code利用時のPlan mode機能全般

## 振る舞い定義

```gherkin
Feature: Plan mode対話ツールのUI表示
  Releash経由でClaude Codeを利用する際、Plan mode中のExitPlanModeおよび
  AskUserQuestionツール呼び出しに対して適切なUIが表示され、
  ユーザーが応答できる。

  Rule: ExitPlanModeのpermission_requestはPlan承認UIとして表示される
    Scenario: ExitPlanMode呼び出し時にPlan承認UIが表示される
      Given エージェントがPlan modeで実行中である
      When エージェントがExitPlanModeツールを呼び出す
      Then Plan内容と承認/却下の選択肢を含む承認UIが表示される

  Rule: AskUserQuestionのpermission_requestは質問UIとして表示される
    Scenario: AskUserQuestion呼び出し時に質問UIが表示される
      Given エージェントが実行中である
      When エージェントがAskUserQuestionツールを呼び出す
      Then 質問文と選択肢を含む質問UIが表示される

  Rule: Plan承認の応答はSDKに伝達され承認状態に遷移する
    Scenario: Planを承認するとSDKに許可が伝達される
      Given Plan承認UIが表示されている
      When ユーザーがPlanを承認する
      Then permission_responseがSDK Bridgeに送信され承認済み状態に遷移する

    Scenario: Planを却下するとSDKに拒否が伝達される
      Given Plan承認UIが表示されている
      When ユーザーがPlanを却下する
      Then permission_responseがSDK Bridgeに送信され却下済み状態に遷移する

  Rule: 質問への回答はSDKに伝達され回答済み状態に遷移する
    Scenario: 質問に回答するとSDKに回答が伝達される
      Given 質問UIが表示されている
      When ユーザーが選択肢を選んで回答する
      Then 選択した回答を含むpermission_responseがSDK Bridgeに送信され回答済み状態に遷移する
```

## 実装仕様

**対応方針**: ExitPlanMode/AskUserQuestionのpermission_request UIが表示されないバグを修正するために、`agent_sdk.rs`のストリーミングemit方式を全状態送信から差分送信に変更し、80msスロットルを撤廃する。

**対象コンポーネント**:
- `src-tauri/src/agent_sdk.rs`: `agent-streaming-updated`イベントのペイロードを`streaming_parts`全体のクローンから、新規追加された差分partsのみの送信に変更。80msスロットル制御を撤廃し、メッセージ到着ごとに即時emitする。
- `src/hooks/useAgentSdkListeners.ts`: `agent-streaming-updated`の受信処理で差分partsを`SET_STREAMING_MESSAGE`アクションとしてdispatch。
- `src/hooks/agentChatReducer.ts`: `SET_STREAMING_MESSAGE`ハンドラで`mergeDeltaParts()`により差分partsを既存partsにマージ。

**検討した代替案**:
- permission_request受信時のみスロットルをバイパス: permission以外の即時性問題に対応できず、根本解決にならない
- フロントエンド側で`SET_PENDING_PERMISSION`時にpartsへ注入: streaming_partsとの二重管理になり整合性が崩れる

**影響するテスト**:
- Rust単体テスト: 差分emit動作の検証テスト追加
- フロントエンドテスト: `SET_STREAMING_MESSAGE`のparts追記ロジックのテスト修正
