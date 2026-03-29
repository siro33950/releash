## 要求

**種別**: バグ修正
**現在の挙動**: エージェントがストリーミング中にStopボタン（interrupt）を押すと、10秒以上interrupt処理が完了しない場合にプロセスがkillされ、セッションが復帰不能になる。特にTask tool（サブエージェント）やBash tool実行中に発生しやすい。
**期待する挙動**: Stopボタンはターン（現在の応答）を中断するだけで、セッション自体は維持されるべき。
**再現手順**:
1. エージェントにTask tool（サブエージェント）やBash toolを含む長い処理を実行させる
2. ストリーミング中にStopボタンを押す
3. 10秒以上エージェントがinterrupt処理を完了できない場合、プロセスがkillされる
4. セッションが応答しなくなる
**背景**: `agent_sdk.rs` の `interrupt_agent_query` (L902-942) にある10秒タイムアウトフォールバック (L931-939) が、`BridgeState::Streaming` のまま残っているプロセスをkillしてしまう。Claude Codeがサブエージェント実行中やBashコマンド実行中の場合、interrupt処理の完了に10秒以上かかることがある。

### OSS調査結果

Claude Code SDKを利用する7つのOSSプロジェクトを調査した結果、**interruptのタイムアウトでプロセスをkillする実装は1つも存在しない**。

| プロジェクト | Stars | 中断方式 | タイムアウトkill |
|---|---|---|---|
| claude-agent-kit | - | AbortController.abort() | なし |
| claude-code-webui | 986 | AbortController + requestId Map | なし |
| Open-Claude-Cowork | 3091 | AbortController + RunnerHandle | なし |
| claude-agent-server | 553 | stream.interrupt() | なし |
| ai-sdk-provider-claude-code | 322 | AbortSignal → AbortController | なし |
| Claude-to-IM | 371 | AbortController + activeTasks Map | なし |
| claude-agent-desktop | 194 | interrupt() + abort flag 二重方式 | なし |

主流パターンは **AbortController.abort()** （4/7プロジェクト）で、SDKの `interrupt()` メソッドを使うのは少数派（2/7）。Releashのブリッジ（claude-sdk-bridge.mjs）はNode.jsプロセス内でSDKを直接使用しており、AbortControllerを利用可能な構成である。

**結論**: interrupt方式をAbortControllerベースに変更し、タイムアウトkillフォールバックを削除すべき。

## 振る舞い定義

```gherkin
Feature: エージェントターンの中断
  ユーザーがエージェントのストリーミング応答を中断し、
  セッションを維持したまま次の操作を続行できる。

  Rule: 中断はターンのみを停止し、セッションは維持される
    Scenario: ストリーミング中にStopボタンでターンを中断する
      Given エージェントがストリーミング応答中である
      When ユーザーがStopボタンを押す
      Then 現在のターンが中断される
      And セッションはReady状態に戻る

    Scenario: サブエージェント実行中にStopボタンでターンを中断する
      Given エージェントがTask tool（サブエージェント）を実行中である
      When ユーザーがStopボタンを押す
      Then 現在のターンが中断される
      And セッションはReady状態に戻る

    Scenario: Bashツール実行中にStopボタンでターンを中断する
      Given エージェントがBash toolを実行中である
      When ユーザーがStopボタンを押す
      Then 現在のターンが中断される
      And セッションはReady状態に戻る

  Rule: 中断後のセッションは再利用できる
    Scenario: 中断後に新しいメッセージを送信する
      Given ターンが中断されてセッションがReady状態である
      When ユーザーが新しいメッセージを送信する
      Then エージェントが新しいターンのストリーミングを開始する

  Rule: 中断処理にタイムアウトによるプロセスkillは行わない
    Scenario: interrupt処理が長時間かかる場合でもプロセスはkillされない
      Given エージェントがストリーミング応答中である
      When ユーザーがStopボタンを押す
      Then 中断シグナルが送信される
      And プロセスのkillは行われない
```

## 実装仕様

**対応方針**: エージェントターン中断のセッション復帰不能バグを修正するために、`claude-sdk-bridge.mjs` に `AbortController` を導入し、`agent_sdk.rs` の10秒タイムアウトkillフォールバックを削除する。

**対象コンポーネント**:
- `src-tauri/resources/claude-sdk-bridge.mjs`: `AbortController` を生成し、`query()` の `options.abortController` に渡す。`interrupt` コマンド受信時に `abortController.abort()` を呼ぶ。既存の `currentQuery.interrupt()` は `abort()` に置き換える。
- `src-tauri/src/agent_sdk.rs`: `interrupt_agent_query` 関数（L924-940）から10秒タイムアウト `tokio::spawn` ブロックを削除。`INTERRUPT_TIMEOUT_SECS` 定数も削除。interruptコマンドのstdin送信（L906-921）はそのまま維持。

**技術選定**:
- `AbortController`（Web標準API / Node.js組み込み）: Claude Agent SDKが `Options.abortController` として公式サポート（デフォルト `new AbortController()`）。OSS 7/7プロジェクトで採用されている主流パターン。追加依存なし。

**検討した代替案**:
- タイムアウトkill削除のみ（`interrupt()` メソッド維持）: 変更量は最小だが、`interrupt()` メソッドが無限に完了しない場合のフォールバックがなく、中断の確実性が低い。`AbortController` はSDKレベルでのキャンセレーションを保証するため、より堅牢。

**リスク**:
- `AbortController.abort()` と `currentQuery.interrupt()` の動作差異: SDKドキュメントでは `interrupt()` は「streaming input modeでのみ利用可能」、`abortController` は「操作のキャンセル用」と明記。`abort()` が `turn_complete` イベントを正しく発火するか実機検証が必要。発火しない場合は、ブリッジ側で `abort` イベントを検知して明示的に `turn_complete` を emit する補完処理を追加する。
- `close_agent_session` のタイムアウトkill（L965-985）は今回のスコープ外として維持する（close は明示的な終了操作であり、interruptとは性質が異なる）。

**影響するテスト**:
- Rust単体テスト: `interrupt_agent_query` にタイムアウトkill関連のテストがあれば削除/更新
- フロントエンド: `useAgentChat` の `interrupt()` テスト — 動作変更なし（invoke呼び出しは変わらない）
- 統合テスト (Playwright): interrupt → セッション復帰の E2E テストを追加推奨
