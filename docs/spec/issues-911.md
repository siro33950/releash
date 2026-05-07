## 要求

**種別**: バグ修正
**現在の挙動**: `close_agent_session()`は`proc.child.kill()`で直接の子プロセス（Claude Code SDKプロセス）のみをkillしている。SDKが起動した孫プロセス（rust-analyzer、proc-macro-srv等）はプロセスグループ管理されていないため、親がkillされても残存する。セッションを繰り返すたびに孤児プロセスが蓄積し、rust-analyzer 1プロセスあたり4-5GBのメモリを消費する。
**期待する挙動**: セッション終了時に、直接の子プロセスだけでなく、その子孫プロセス全てがkillされ、メモリが解放される。
**再現手順**:
1. Releash上でClaude Codeエージェントセッションを開始する
2. エージェントがRustプロジェクトを解析する過程でrust-analyzerが起動される
3. セッションを終了する
4. `ps -eo pid,rss,comm | grep rust-analyzer` で確認すると、rust-analyzerプロセスが残存している
**背景**: セッションを繰り返すたびに孤児プロセスが蓄積し、ユーザーが手動でkillしない限り解放されない。観測では合計約9.5GBがセッション終了後も占有されていた。
**影響範囲**: agent_sdk.rs（エージェントセッションのプロセス管理）、lib.rs（終了時クリーンアップ）、tray.rs（Quit処理）

## 振る舞い定義

```gherkin
Feature: セッション終了時の子孫プロセス全kill
  Agentセッション終了時に、直接の子プロセスだけでなく
  孫プロセス以下の全子孫プロセスをkillし、メモリを解放する。
  PTY側はportable_ptyが既にsetsid()でプロセスグループを管理しているため対象外。

  Rule: Agentセッション終了時にプロセスグループ全体がkillされる
    Scenario: Agentセッション終了で孫プロセスもkillされる
      Given Agentセッションが起動しており、SDKプロセスがrust-analyzer等の孫プロセスを起動している
      When セッションを終了する
      Then SDKプロセスと全ての孫プロセスがkillされる

    Scenario: Graceful shutdown後にプロセスグループが強制killされる
      Given Agentセッションが起動しており、SDKプロセスがgraceful closeに応答しない
      When セッション終了のタイムアウト（5秒）が経過する
      Then プロセスグループ全体がSIGKILLで強制killされる

  Rule: プロセスグループは起動時に設定される
    Scenario: Agentセッション起動時にプロセスグループが作成される
      Given 新しいAgentセッションを起動する
      When SDKプロセスが生成される
      Then SDKプロセスは新しいプロセスグループのリーダーとして起動される

  Rule: Releash正常終了時に全Agentセッションのプロセスグループがkillされる
    Scenario: アプリ終了で全Agentセッションの子孫プロセスがkillされる
      Given 複数のAgentセッションが起動している
      When Releashを正常終了する
      Then 全てのAgentセッションのプロセスグループがkillされる

  Rule: Releash起動時に前回の孤児プロセスが検出・killされる
    Scenario: 前回クラッシュで残存した孤児プロセスがkillされる
      Given 前回のReleashがクラッシュし、孤児プロセスが残存している
      When Releashを起動する
      Then 前回セッション由来の孤児プロセスが検出されkillされる
```

## 実装仕様

**対応方針**: Agentセッション終了時に子孫プロセスが残存するバグを、Unixプロセスグループ管理の導入で修正する。Agent SDKプロセス起動時に`setsid()`で新しいプロセスグループを作成し、終了時に`killpg()`でグループ全体をkillする。また、Releash正常終了時の全セッションクリーンアップと、起動時のPIDファイルベースの孤児プロセス検出・killを追加する。PTY側はportable_ptyが既に`setsid()`を使用しており対象外。

**対象コンポーネント**:
- `src-tauri/src/agent_sdk.rs`: プロセスグループ設定（spawn）、グループkill（close）、PGID永続化
- `src-tauri/src/lib.rs`: Releash終了時の全Agentセッションクリーンアップ、起動時孤児プロセスクリーンアップ
- `src-tauri/src/tray.rs`: Quit処理にAgentセッション全killを追加

**変更内容**:

### A. プロセスグループ管理（agent_sdk.rs）

1. **spawn_bridge_process()に`pre_exec`追加**:
   - `Command::new("node")` に `.pre_exec(|| { libc::setsid(); Ok(()) })` を追加
   - 子プロセスが新しいプロセスグループのリーダーとして起動される

2. **AgentProcess構造体にpgid保存**:
   - `pub pgid: Option<u32>` フィールドを追加
   - spawn後に `child.id()` でPIDを取得（= PGID、setsid後はPID == PGIDのため）

3. **close_agent_session()のkill処理変更**:
   - Graceful shutdown（closeコマンド送信）は維持
   - タイムアウト後の強制killを `proc.child.kill()` → `libc::killpg(pgid, libc::SIGKILL)` に変更
   - Graceful shutdown成功後もプロセスグループに残存プロセスがないか `killpg(pgid, SIGTERM)` で掃除

### B. PIDファイル永続化

4. **PGID保存ディレクトリ**: `{app_data_dir}/pids/` に `{chat_session_id}.pid` ファイルとしてPGIDを保存
5. **セッション起動時**: spawn成功後にPIDファイルを書き込み
6. **セッション終了時**: プロセスグループkill成功後にPIDファイルを削除

### C. Releash正常終了時のクリーンアップ

7. **tray.rs Quit処理の拡張**: `stop_server_core()` の前に全Agentセッションの`close_agent_session()`を呼ぶ
   - `close_all_agent_sessions()` 関数を新設

### D. 起動時の孤児プロセス検出・kill

8. **起動時クリーンアップ関数**: `cleanup_orphan_processes()` を新設
   - `{app_data_dir}/pids/` 内の全PIDファイルを走査
   - 各PGIDについて `libc::killpg(pgid, 0)` でプロセスグループの生存確認
   - 生存していれば `killpg(pgid, SIGTERM)` → タイムアウト → `killpg(pgid, SIGKILL)`
   - PIDファイルを削除
9. **呼び出し箇所**: `lib.rs` の `.setup()` 内、`init_agent_sessions()` の前に実行

**検討した代替案**:
- プロセスツリー探索方式（`pgrep -P`等）: プロセス名やコマンドラインのパターンマッチに依存するため誤検出リスクがある。PIDファイル方式の方が確実

**リスク**:
- `setsid()`はUnix専用。macOS/Linuxでは動作するが、Windows対応が必要になった場合は`Job Object`等の別機構が必要
  - 緩和策: `#[cfg(unix)]` で条件コンパイルし、非Unix環境では従来動作を維持

**影響するテスト**:
- ユニットテスト: `cleanup_orphan_processes()` のPIDファイル読み書き・削除ロジック
- ユニットテスト: `close_all_agent_sessions()` のイテレーションロジック
- 統合テスト: プロセスグループ管理は実プロセス起動が必要なため、手動テストで確認
