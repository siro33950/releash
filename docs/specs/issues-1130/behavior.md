# Behavior

本書は #1130「protocol 境界の `adaptor/protocol/` への一本化」の振る舞い定義である。

本 ISSUE はリファクタリングであり、対外的な振る舞い（WebSocket message 名・payload JSON shape）は不変であることが最重要のビジネスルールである。それ以外の受け入れ基準は、移設後に成立すべき構造的不変条件として定義する。Gherkin では「移設前後で外部から観測した結果が変わらないこと」と「完了時点で成立すべき構造・品質ゲート」を扱い、個々の import 書き換え手順やファイル単位の置換経路といった実装詳細は扱わない。

## 仮定

- 移設は型・helper の物理移動と import 経路の更新に限り、シリアライズ結果は完全に維持する。message 名・payload JSON shape を一切変えないため、frontend およびネットワーク互換性に影響を与えない。
- `src-tauri/src/protocol/` 配下（`mod.rs` / `agent.rs` / `auth.rs` / `branch.rs` / `error.rs` / `worktree.rs`）の型・helper が移設対象である。`src-tauri/src/adaptor/protocol/` 配下（`mod.rs` / `code.rs` / `mention.rs` / `pty.rs` / `workflow.rs`）は移設先であり、両者を統合した後の単一の protocol 境界が成果物となる。
- 移設対象ファイルの具体的な配置・分割粒度は design.md で確定する。本書は配置の結果として満たすべき観測可能な条件のみを定義する。
- domain / usecase 層は現状 protocol を参照していない。本 ISSUE 完了後もこの依存方向（adaptor → usecase → domain）を維持する。
- 「production reference を返さない」とは、ビルド対象となる production path の `crate::protocol`（ルート直下）参照を指す。doc comment や移設後の adaptor 内 module path は対象外とする。
- 品質ゲートのコマンドは `src-tauri/` ディレクトリで実行する。

## Feature: protocol 境界を adaptor/protocol へ一本化する

  protocol 境界の定義が `src-tauri/src/protocol/`（ルート直下）と
  `src-tauri/src/adaptor/protocol/` の 2 箇所に併存している二重化を解消し、
  protocol 境界を `adaptor/protocol/` に一本化する。
  対外契約と domain / usecase の振る舞いは変更しない。

  Background:
    Given clean architecture の層構造では adaptor が wire shape を所有する
    And protocol 境界の型・helper が現在 2 箇所に併存している
    And WebSocket の対外契約（message 名・payload JSON shape）は変更してはならない

  Rule: protocol 境界は adaptor/protocol/ にのみ存在する

    Scenario: ルート直下の protocol ディレクトリが削除されている
      When 移設が完了した状態のソースツリーを検査する
      Then `src-tauri/src/protocol/` ディレクトリは存在しない
      And `lib.rs` にルート直下の `mod protocol` 宣言は残っていない

    Scenario: 移設対象の型・helper が adaptor/protocol/ 配下に存在する
      When 移設が完了した状態のソースツリーを検査する
      Then `WsMessage` は `adaptor/protocol/` 配下にある
      And `serialize_message` は `adaptor/protocol/` 配下にある
      And `deserialize_message` は `adaptor/protocol/` 配下にある
      And agent / auth / branch / error / worktree の各 payload type は `adaptor/protocol/` 配下にある

  Rule: production code の import 経路が adaptor/protocol/ に統一されている

    Scenario: ルート直下 protocol への参照が production code に残っていない
      Given 移設前は ws_server / ws_bridge / agent_status_events / adaptor 配下 gateway・controller / infrastructure 配下 agent_session runtime 等が `crate::protocol` を参照していた
      When 移設が完了した状態で `rg 'crate::protocol' src-tauri/src --glob '*.rs'` を実行する
      Then production path に `crate::protocol`（ルート直下）の参照が返らない

    Scenario: 移設対象を参照する全モジュールが新経路を参照する
      When 移設対象の型・helper を参照する production モジュールを検査する
      Then すべて `crate::adaptor::protocol::*`（または adaptor 内の相対経路）を参照している

  Rule: 層責務（依存方向）を維持する

    Scenario: domain / usecase が adaptor::protocol を import しない
      When domain 層および usecase 層のモジュールを検査する
      Then いずれも `crate::adaptor::protocol` を import していない
      And 依存方向 adaptor → usecase → domain は逆転していない

  Rule: WebSocket の対外契約は移設前後で不変である

    Scenario Outline: WsMessage 各バリアントの serialize 結果が移設前後で一致する
      Given <variant> を表す WsMessage 値がある
      When 移設後の `serialize_message` で JSON へシリアライズする
      Then message 名（タグ）が移設前と一致する
      And payload の JSON shape が移設前と一致する

      Examples:
        | variant                |
        | AuthChallenge          |
        | AuthResponse           |
        | AuthResult             |
        | PtyOutput              |
        | PtyExit                |
        | PtyEvicted             |
        | WorktreePrStatusSync   |
        | BranchListSync         |
        | AgentStateSync         |
        | WorkflowStateSync      |
        | AgentStreamSync        |
        | AgentStreamDelta       |
        | ResyncStream           |
        | Error                  |

    Scenario: 任意の正当な wire JSON が移設後も同じ WsMessage へ deserialize される
      Given 移設前に正当だった WebSocket message の JSON 文字列がある
      When 移設後の `deserialize_message` で復元する
      Then 移設前と同じ WsMessage バリアント・値が得られる

    Scenario: 未知の message type は移設後も拒否される
      Given 未知の type タグを持つ JSON 文字列がある
      When 移設後の `deserialize_message` で復元を試みる
      Then エラーが返る

    Scenario: 省略可能フィールドの serialize 挙動が維持される
      Given 値が None の省略可能フィールド（例: AuthResult の message、AgentStateSync の pty_id）を持つ WsMessage 値がある
      When 移設後の `serialize_message` でシリアライズする
      Then 当該フィールドは出力 JSON から省略される

  Rule: 既存の protocol テストが新 module 側で維持される

    Scenario: roundtrip / serialize-deserialize test が新 module 側に存在し通過する
      Given 移設前に protocol 配下に存在した roundtrip / serialize-deserialize test がある
      When 移設後のソースツリーを検査し `cargo test` を実行する
      Then 当該 test は新しい `adaptor/protocol/` 側に存在する
      And 当該 test が通過する

  Rule: 品質ゲートを満たす

    Scenario: フォーマット・lint・テストが通る
      When `src-tauri/` で品質ゲートを実行する
      Then `cargo fmt --check` が通る
      And `cargo clippy -- -D warnings` が通る
      And `cargo test` が通る

## 非対象（このスコープで変更しないこと）

  以下は本 ISSUE の振る舞いに含めない。これらが変化しないこと自体は上記 Rule（対外契約の不変）で担保される。

  - WebSocket message 名・payload JSON contract の変更
  - `ws_server/` の session / auth / routing / rate-limit の構造変更（#1131）
  - `ws_bridge.rs` の buffering / broadcast lifecycle の変更（#1131）
  - domain behavior / command 実装ロジックの移動・変更
  - frontend（TypeScript）側の型定義・通信コードの変更
  - `adaptor/presenter/` / `adaptor/controller/` への mapping ロジックの新規切り出し・再設計（二重化解消に必要な範囲を超える部分）

## Open Questions

なし。
