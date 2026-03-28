## 要求

**種別**: バグ修正
**現在の挙動**: メッセージを送信しても Agent が応答せず、ログも出力されずサイレントにエラー状態になる。初回メッセージですら動作しない。
**期待する挙動**: メッセージを送信すると Agent が正常に応答し、セッション再開（resume）時も安定して動作する
**再現手順**:
1. アプリを起動する（セッション作成時に prewarm として `start_agent_session` が呼ばれる）
2. Bridge が `session_ready` を emit し、`agent_session_id` が SessionStore に即座に永続化される
3. **この時点でユーザーは一度もメッセージを送っていない**ため、SDKサーバー側に実質的な「会話」は存在しない
4. Bridge プロセスが何らかの理由で終了する
5. ユーザーが初回メッセージを送信する
6. `execute_agent_query` が Crashed 分岐に入り、stale な `agent_session_id` で resume を試行する
7. SDK: `"No conversation found with session ID: ..."` エラー
8. UI にはエラー詳細が表示されず、無応答状態になる

**背景**:
- `session_ready` は「SDKが接続完了した」だけの意味で、「会話が確立された」ことを意味しない。一度もターンが完了していない session ID を永続化・resume に使ってはならない
- Bridge にリトライロジック（`while(true)` ループ、`inflightPrompts`、`resetForRetry()`）を持たせていたが、SDK は同一プロセスで `query()` を2回呼ぶことを想定していない（Issue #34）ため全て無意味
- OSS 調査で prewarm パターンは公式 SDK デモのどこにも存在しないことが判明

**制約**:
- 1セッション = 1プロセスを厳守する
- アプリ起動時に全セッションの Bridge プロセスを起動し、Ready 状態にする
- 起動中はプロセスは常に生存し続ける
- セッションが閉じられたらプロセスも閉じる
- セッション復活時は保持している session ID で resume して Ready 状態にする
- 永続化は「ユーザーターンが正常完了した時点（`turn_complete` で `exit_code=0`）」のみとする
- resume 判定は Rust が唯一の判定者とする（Bridge は stateless な中継者）
- Bridge のリトライロジックを除去する

## 振る舞い定義

```gherkin
Feature: Agent セッションのライフサイクル管理
  Bridge プロセスをチャットセッション単位で常駐させ、
  安定したメッセージ送受信とセッション復帰を実現する

  Rule: agent_session_id はユーザーターンが正常完了した時点でのみ永続化される
    Scenario: ターンが正常完了すると agent_session_id が永続化される
      Given Agent プロセスが Ready 状態である
      And ユーザーがメッセージを送信してターンが開始された
      When ターンが正常完了する
      Then agent_session_id が SessionStore に永続化される

    Scenario: SDK 接続完了だけでは agent_session_id は永続化されない
      Given セッションが作成され Bridge プロセスが起動された
      When SDK が接続を完了する
      Then agent_session_id は SessionStore に永続化されない
      And agent_session_id はプロセスのメモリにのみ保持される

    Scenario: 初期化中にエラーが発生すると stale な agent_session_id がクリアされる
      Given SessionStore に過去の agent_session_id が保存されている
      And resume で Bridge プロセスが起動された
      When 初期化中にエラーが発生する
      Then SessionStore の agent_session_id がクリアされる

  Rule: アプリ起動時に全セッションの Bridge プロセスが起動される
    Scenario: アプリ起動時に既存セッションのプロセスが起動される
      Given 過去に作成されたセッションが存在する
      When アプリが起動する
      Then 全セッションの Bridge プロセスが起動される
      And 全プロセスが Ready 状態になる

    Scenario: アプリ起動時にセッションが存在しない場合は新規作成される
      Given 過去に作成されたセッションが存在しない
      When アプリが起動する
      Then 新しいセッションが作成される
      And Bridge プロセスが起動され Ready 状態になる

  Rule: セッション作成時に Bridge プロセスが起動される
    Scenario: 新規セッション作成でプロセスが起動される
      Given ユーザーが新しいセッションを作成する
      When セッションの作成が完了する
      Then Bridge プロセスが起動され Ready 状態になる

  Rule: メッセージ送信後もプロセスは生存し続ける
    Scenario: メッセージを送信すると Agent が応答する
      Given Agent プロセスが Ready 状態である
      When ユーザーがメッセージを送信する
      Then Agent が応答を返す

    Scenario: ターン完了後にプロセスが次のメッセージを受け付ける
      Given Agent がターン処理を完了した
      When ユーザーが次のメッセージを送信する
      Then Agent が即座に応答を開始する

  Rule: セッション終了時にプロセスが終了する
    Scenario: セッションを閉じるとプロセスが終了する
      Given Agent プロセスが Ready 状態である
      When ユーザーがセッションを閉じる
      Then Bridge プロセスが終了する

  Rule: セッション復活時に resume で Bridge プロセスが起動される
    Scenario: 閉じたセッションを復活させるとプロセスが resume で起動される
      Given 閉じたセッションに agent_session_id が保存されている
      When ユーザーがセッションを復活させる
      Then 保存された agent_session_id で resume して Bridge プロセスが起動される
      And プロセスが Ready 状態になる

    Scenario: agent_session_id がないセッションを復活させると新規で起動される
      Given 閉じたセッションに agent_session_id が保存されていない
      When ユーザーがセッションを復活させる
      Then 新規で Bridge プロセスが起動される
      And プロセスが Ready 状態になる

  Rule: プロセスクラッシュ時はメッセージ送信で自動復帰する
    Scenario: クラッシュ後にメッセージを送信するとプロセスが自動再起動される
      Given Agent プロセスがクラッシュした
      When ユーザーがメッセージを送信する
      Then 新しい Bridge プロセスが自動的に起動される
      And Agent が応答を返す

  Rule: resume 判定は Rust が唯一の判定者である
    Scenario: SessionStore に agent_session_id がある場合は resume で起動される
      Given SessionStore にセッションの agent_session_id が保存されている
      When Bridge プロセスが起動される
      Then 保存された agent_session_id で resume が試行される

    Scenario: SessionStore に agent_session_id がない場合は新規で起動される
      Given SessionStore にセッションの agent_session_id が保存されていない
      When Bridge プロセスが起動される
      Then 新規セッションとして起動される
```

## 実装仕様

**対応方針**: 永続化タイミングを `session_ready` から `turn_complete(exit_code=0)` に変更し、Bridge のリトライロジックを除去することで、stale な agent_session_id による resume 失敗を根本的に解消する。

**対象コンポーネント**:

- `src-tauri/resources/claude-sdk-bridge.mjs`:
  - 除去: `inflightPrompts`, `resetForRetry()`, `while(true)` ループ + `retried` + `shouldRetry` による resume リトライ
  - `promptGenerator`: `inflightPrompts.push()` を除去（yield のみ）
  - `handleInit`: シングルパス（try/catch のみ、ループなし）

- `src-tauri/src/agent_sdk.rs`:
  - `session_ready` ハンドラ: SessionStore への書き込みを除去。`AgentProcess.sdk_session_id` のメモリ更新のみ残す
  - `turn_complete` ハンドラ:
    - `was_streaming` かつ `exit_code=0`: `AgentProcess.sdk_session_id` を SessionStore に永続化
    - `!was_streaming` かつ `exit_code!=0`: SessionStore の `agent_session_id` をクリア（stale ID 除去）
    - `agent-query-completed` emit は `was_streaming` 時のみ（init 中の turn_complete では emit しない）
  - `start_agent_session`: SessionStore から `agent_session_id` を読んで resume パラメータとして `spawn_bridge_process` に渡す

- `src/hooks/useAgentChat.ts`:
  - `initSessions`: 全セッション分の `start_agent_session` を呼んで Bridge プロセスを起動
  - `restoreSessionFn`: `restoreSessionApi` 後に `start_agent_session` を呼んで Bridge プロセスを起動

**影響するテスト**:
- Rust 単体テスト: 既存テストで状態遷移・コマンドフォーマットをカバー済み。変更なし
- フロントエンド単体テスト: `initSessions` で `start_agent_session` が呼ばれることのテスト追加、`restoreSession` で `start_agent_session` が呼ばれることのテスト追加
