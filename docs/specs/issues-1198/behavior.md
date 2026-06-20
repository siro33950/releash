# Behavior

要求 (`requirements.md`) を、実装詳細を含まない観測可能な振る舞いとして Gherkin で定義する。

本 Issue は土台リファクタ（決定済みの削除掃除）であり、機能追加ではない。したがって振る舞い定義の中心は、
「ドリフトした remote クライアント・remote ビルド経路・ws の req/resp ハンドラ群一式が除去され、ws が
『認証＋push broadcast の shell』へ縮退する」という観測可能な構造変化と、
「デスクトップ本体の機能的振る舞いがこの掃除によって一切変化しない」という不変条件である。

## 仮定（本文中で明示）

- **削除対象（req/resp 表層）**: `routing.rs` の `WsMessage::*Request` インバウンド分岐と、それが呼ぶ `handlers.rs` の
  req/resp 実装、対応する `*Request` / `*Response` variant（`ReviewThreadResponse` 等の応答 variant を含む）、
  および `commands.rs` / `validation.rs` 等の req/resp 専用部分とそのテストを指す。
- **削除対象（リモート起動トリガー一式／ユーザー合意「使わないものは全て削除」）**: `src/remote/` クライアント削除により
  接続先が消えるため、ユーザー合意に基づき**リモートアクセスの起動トリガーを残らず削除する**。具体的には以下を含む。
    - フロント: `src/components/panels/RemotePanel.tsx`（+ テスト）、`src/hooks/useRemoteServer.ts`（+ テスト）、
      `src/hooks/useRemoteAutoStart.ts`（+ テスト）、`App.tsx` 等からの RemotePanel 参照。
    - Tauri command: `start_server` / `stop_server`（`adaptor/controller/command/mod.rs` の登録と invoke ラッパー）。
    - tray（独立経路）: `Start Server` / `Stop Server` メニュー、`handle_start_server` / `handle_stop_server`、
      `update_tray_menu(server_running)`。
    - 自動起動（独立経路）: `lib.rs` の `config.remote.auto_start` を見て `start_server_core` を spawn する配線。
    - config: `remote.auto_start` / `app.last_bind_ip` / サーバーポート等の remote 起動専用設定。
    - CI（`ci.yml`）: `build:remote` ステップ、`RemotePanel*` スクリーンショットフィルタ。
  これらは requirements の当初 Scope を超えるため、requirements への追補を反映済み。
- **残置（再利用＝案X、呼び出し元ゼロでも温存）**: ws の `start_server_core` / `stop_server_core`（`ws_server/commands.rs`）と
  shell（`http.rs`/`auth.rs`/`rate_limit.rs`/`WsBroadcaster`）は A1 で再利用するため残す。起動トリガーを全削除すると
  これらの呼び出し元・push 送信元がゼロになるが、**`#[allow(dead_code)]` 等で温存し A1 で起動を再配線する**（コードを残し、
  配線は A1）。これにより「使わないトリガーは全削除・再利用コードは温存」を両立する。`#[allow(dead_code)]` か `cfg` か等の
  温存手段の具体と、config マイグレーション（remote セクション除去）の互換性確保は `design.md` で確定する。
- **残置（push／認証 shell）**: push（broadcast）系 variant ——
  `WorktreePrStatusSync` / `BranchListSync` / `AgentStateSync` / `WorkflowStateSync` / `AgentStreamSync`、
  `PtyOutput` / `PtyExit` / `PtyReady`、認証系 `AuthChallenge` / `AuthResponse` / `AuthResult`、`Error` ——
  と、`http.rs` / `auth.rs` / `rate_limit.rs` / `WsBroadcaster`（`ws_bridge.rs`）を残す（案X）。
- **インバウンドのアプリ制御メッセージ**（`PtyInput` / `PtyResize` / `PtyOutputRequest` 等）は、remote クライアント削除後に
  送信元が消えるため req/resp ハンドラ群と同様に削除対象とみなす。ws の inbound 受理は認証ハンドシェイクに縮退する。
- **観測単位**: 「振る舞いが変化しない」は、デスクトップ利用者が UI／`invoke`／`listen`（emit イベント）を通じて
  外部から観測できる出力（表示内容・コマンド戻り値・受信イベント）が掃除前後で一致することを指す。
- 「remote 専用依存」の具体的特定（`package.json` のどの依存が remote 専用か）と削除可否の精査は `design.md` で行う。
  本 behavior では「remote 専用依存が残らないこと」を観測点とする。
- **縮退後の shell が認証後に「削除済み req/resp 相当」または未知のメッセージを受信したときの観測挙動**は、
  現行 `routing.rs` の既定動作を踏襲し **`Error`（`INVALID_MESSAGE` 相当）で応答し、接続は維持する**（接続切断・
  無言破棄はしない）。これは確定事項であり、残置する `Error` variant を再利用でき、既存テスト
  `test_route_unknown_message_returns_error` と整合する。

Feature: ドリフトした remote クライアントと ws req/resp ハンドラの削除掃除（A1 の土台整備）

  Background:
    Given Releash デスクトップアプリのソースツリーがある
    And デスクトップ本体は Tauri の `invoke`（コマンド）と `emit`/`listen`（イベント）経由で Rust ロジックを利用している
    And 本 Issue の掃除（remote 削除＋ws 縮退）を適用したビルドが用意できる

  # --- Rule 1: remote クライアント・ビルド経路・デスクトップ起動UIの除去 ---
  Rule: src/remote 一式・remote ビルド経路・デスクトップ側リモートアクセス起動UIがリポジトリから除去されている

    Scenario: src/remote ディレクトリ一式が存在しない
      Given 掃除を適用したソースツリーを参照する
      When `src/remote/`（RemoteApp とその配下の components/hooks/styles/main 等）を探す
      Then `src/remote/` 一式は存在しない

    Scenario: リモートアクセスの起動トリガーが残らず削除されている
      Given 掃除を適用したソースツリーを参照する
      When リモートアクセスの起動トリガーを探す
      Then `src/components/panels/RemotePanel.tsx`（およびそのテスト）は存在しない
      And `src/hooks/useRemoteServer.ts` / `src/hooks/useRemoteAutoStart.ts`（およびテスト）は存在しない
      And `App.tsx` 等から RemotePanel への参照が残っていない
      And `start_server` / `stop_server` の Tauri command 登録・invoke ラッパーが存在しない
      And tray の `Start Server` / `Stop Server` メニューとそのハンドラ・`update_tray_menu` が存在しない
      And `lib.rs` の `config.remote.auto_start` による自動起動配線が存在しない
      And `remote.auto_start` / `last_bind_ip` / サーバーポート等の remote 起動専用 config 項目が残っていない

    Scenario: 起動トリガー削除後も再利用する core と shell は温存される
      Given 掃除を適用したソースツリーを参照する
      When ws の起動機構を確認する
      Then `start_server_core` / `stop_server_core` と shell（http/auth/rate_limit/WsBroadcaster）は残存している
      And これらは呼び出し元・push 送信元がゼロでも `#[allow(dead_code)]` 等で温存され、A1 で起動を再配線できる
      And 削除されたのは起動トリガーであり、再利用する起動機構コードそのものではない

    Scenario: remote 専用ビルド設定が存在しない
      Given 掃除を適用したソースツリーを参照する
      When remote 専用のビルド設定を探す
      Then `vite.config.remote.ts` は存在しない
      And `package.json` に `build:remote` 等の remote 専用スクリプトが存在しない
      And `build` / `dev` スクリプトから remote ビルドターゲットへの参照が除去されている
      And remote 専用依存が `package.json` に残っていない

    Scenario: CI から remote ビルド・RemotePanel への参照が除去されている
      Given 掃除を適用した `.github/workflows/ci.yml` を参照する
      When CI 定義から remote 関連の参照を探す
      Then `pnpm build:remote` を実行するステップが存在しない
      And `RemotePanel*` を対象にしたスクリーンショットフィルタが存在しない
      And remote 削除後も CI がクリーンに通る構成になっている

    Scenario: ビルド設定のどこにも remote ターゲットへの参照が残らない
      Given 掃除を適用したソースツリーを参照する
      When ビルド設定全体（スクリプト・vite 設定・依存）を走査する
      Then remote ビルドターゲットを指す参照は 1 つも見つからない

  # --- Rule 2: ws req/resp ハンドラ群と専用 variant の除去 ---
  Rule: ドリフトした ws の req/resp ハンドラ群と専用 variant が除去されている

    Scenario Outline: req/resp 専用 variant が WsMessage から除去されている
      Given 掃除を適用したソースツリーを参照する
      When `WsMessage` の variant 定義を確認する
      Then req/resp 専用 variant <variant> は存在しない

      Examples:
        | variant                              |
        | PtySpawnRequest / PtySpawnResponse   |
        | PtyKillRequest / PtyKillResponse     |
        | PtyOutputRequest                     |
        | PtyInput / PtyResize                 |
        | BranchInfoRequest / BranchInfoResponse |
        | WorktreeListRequest / WorktreeListResponse |
        | WorktreeSelectRequest / WorktreeSelectResponse |
        | BackendListRequest / BackendListResponse |
        | Agent*Request / Agent*Response 群     |
        | Review*Request / ReviewThreadResponse 群 |

    Scenario: req/resp のルーティング分岐・ハンドラ実装・バリデーションが除去されている
      Given 掃除を適用したソースツリーを参照する
      When `routing.rs` / `handlers.rs` / `commands.rs` / `validation.rs` を確認する
      Then req/resp 要求を処理するインバウンド分岐・ハンドラ実装・バリデーションは存在しない
      And それらに紐づくテストも併せて除去されている

    Scenario: 除去された variant に対応する protocol 型が残らない
      Given 掃除を適用したソースツリーを参照する
      When protocol モジュールを確認する
      Then 削除された req/resp variant に対応する protocol 型が孤立して残っていない

  # --- Rule 3: ws の「認証＋push broadcast shell」への縮退 ---
  Rule: ws が認証ハンドシェイクと push broadcast のみを担う shell に縮退している

    Scenario: 認証ハンドシェイクは従来どおり成立する
      Given クライアントが ws へ接続する
      When 認証ハンドシェイク（AuthChallenge → AuthResponse → AuthResult）を行う
      Then 正当なトークンでは認証が成功する
      And 不正なトークンでは認証が失敗し接続が確立しない

    Scenario: 認証済み接続へ push（broadcast）が配信される
      Given クライアントが認証済みで ws に接続している
      When サーバ側で push 対象の状態変化（`*Sync` / `PtyOutput` / `PtyExit` / `PtyReady` 等）が発生する
      Then その push メッセージが認証済み接続へ broadcast 配信される

    Scenario: 縮退後の shell は req/resp 要求を受理しない
      Given クライアントが認証済みで ws に接続している
      When 削除済みの req/resp 要求に相当するメッセージを送信する
      Then shell はその要求を処理して応答を返すことはない
      And `Error`（INVALID_MESSAGE 相当）で応答し、接続は維持される

    Scenario: 残置土台が引き続き機能する（案X）
      Given 掃除を適用したビルドが動作している
      When `http.rs` / `auth.rs` / `rate_limit.rs` / `WsBroadcaster` の経路を利用する
      Then HTTP・認証・レート制限・broadcast は従来どおり機能する
      And これらは縮退に必要な最小限を超えて改変されていない

  # --- Rule 4: デスクトップ機能の不変（リモートアクセス起動機能を除く・回帰なし） ---
  Rule: リモートアクセス起動機能を除き、掃除によってデスクトップの機能的振る舞いが変化しない

    Scenario: invoke 経由のデスクトップ全機能が従来どおり動作する
      Given 掃除前ビルドでのデスクトップ全機能の動作が観測されている
      When 同一の操作（Git 操作・ターミナル・diff 閲覧・コメント・エージェント・ソース管理等）を掃除後ビルドで実行する
      Then 各機能は掃除前と同じ外部観測可能な結果を返す
      And ws 縮退に起因する（リモートアクセス起動UI 以外の）機能の欠落や挙動差は発生しない

    Scenario: リモートアクセス起動機能は本 Issue で意図的に除去される
      Given 掃除前ビルドにはデスクトップからリモートアクセスサーバを起動・QR 表示する機能があった
      When 掃除後ビルドのデスクトップ UI（パネル・tray メニュー）を操作する
      Then リモートアクセス起動UI（RemotePanel 経路）も tray の Start/Stop Server も提供されない
      And 自動起動（auto_start）も行われない
      And これは本 Issue の意図された削除であり、回帰ではない

    Scenario: emit/listen 経路が従来どおり機能する
      Given デスクトップが `emit`/`listen`（イベント）を利用している
      When 掃除後ビルドでイベントを伴う操作を実行する
      Then デスクトップの `listen` 利用は従来どおりイベントを受信する
      And `emit` 経路は本 Issue で撤去されていない

    Scenario: ws req/resp ハンドラが利用していた下層ロジックは保持される
      Given ws req/resp ハンドラが内部で usecase/adaptor 層を呼んでいた
      When req/resp 表層を削除する
      Then デスクトップが `invoke` 経由で利用する下層 usecase/adaptor/コマンドは削除されない
      And 削除は ws の req/resp 表層（routing/handler/protocol variant）に限定される

  # --- Rule 5: クリーンビルド・テスト・lint・CI ---
  Rule: 掃除後にビルド・テスト・lint・CI がクリーンに通る

    Scenario Outline: フロント・Rust・CI がクリーンに通る
      Given 掃除を適用したリポジトリがある
      When <check> を実行する
      Then エラーや警告なくクリーンに成功する

      Examples:
        | check                       |
        | pnpm build                  |
        | pnpm lint                   |
        | pnpm test                   |
        | cargo fmt --check           |
        | cargo clippy -- -D warnings |
        | cargo test                  |
        | CI（.github/workflows）      |

    Scenario: 削除によって参照切れ・未使用による失敗が残らない
      Given 掃除を適用したリポジトリがある
      When ビルドと lint を実行する
      Then 削除対象への参照切れや未使用シンボルによる失敗・警告が発生しない
      And A1 で再利用するため温存した core/shell は `#[allow(dead_code)]` 等で警告対象から除外され、`clippy -D warnings` を通す

  # --- Rule 6: スコープ限定 ---
  Rule: 削除は決定済みのドリフト遺産に限定される

    Scenario: 要求外のスコープ拡大が行われない
      Given 掃除を適用した差分を参照する
      When 変更範囲を確認する
      Then 削除対象は remote クライアント・remote ビルド経路・ws req/resp 表層・リモート起動トリガー一式（RemotePanel/tray/autostart/command/config、ユーザー合意）に限定されている
      And 無関係なリファクタや残置土台（http/auth/rate_limit/WsBroadcaster/start_server_core）の作り直しは含まれない
      And 温存する core/shell の `#[allow(dead_code)]` 付与は除き、再利用コードの作り変えは行わない
      And A1（walking skeleton）そのものの実装は含まれない

## Open Questions

なし（縮退後 shell の inbound 応答は「`Error` 応答し接続維持＝現行踏襲」で確定。remote 専用依存の具体特定は `design.md` で行う）。
