# Requirements

対象 Issue: #1193「Agent: 新規セッション初回ターンで普通の指示が「モデルを設定しました」とだけ返ることがある」

## Background / 背景と目的

Agent セッションで、新規セッション開始直後の初回ターンに限り、ユーザーが普通の指示テキスト（スラッシュコマンドではない）を送ったにもかかわらず、AI が

> モデルを設定しました。まだ具体的な指示をいただいていません。何を行いましょうか？

とだけ返すことが "ときどき" 発生する。ユーザーは `/model` 等のスラッシュコマンドを打っていない。これはユーザー指示が実行されない不具合であり、初回ターンの信頼性を損なう。

### 確定している原因（コードで裏取り済み）

1. Rust 側はユーザー指示を欠落させず、モデルテキストにも化けさせない。直接送信・pending drain（`start_pending_message_turn`）の両経路とも prompt は `content` から生成され空にならず、mention 解決も content を消費しない。
2. `setModel` / `setPermissionMode` は SDK の control request であり、ターンも assistant 発話も生成しない。よって releash 側がこの定型発話を作ることはない。
3. **モデルは init 段階で一切設定されていない。** `build_init_cmd`（`src-tauri/src/infrastructure/agent_session/runtime/bridge_common.rs`）にも、ブリッジ `handleInit`（`src-tauri/resources/claude-sdk-bridge.mjs`）の query options にもモデル指定がない。モデルは `sync_pre_turn_settings` が**毎ターンの message 送信直前**に送る `setModel`（control request）でのみ確立されている。

→ 新規セッション初回ターンでは、モデルが「最初のユーザーメッセージ直前に送られる set_model control request」でしか確立されない構造的特異点が存在する。この control request の応答完了タイミングと最初の user message が CLI に届くタイミングの競合により、CLI 側が「モデル変更だけのターン」を先に成立させると、`/model` 単体実行時と同じ定型応答が出る。これにより全症状（初回ターン限定・普通の指示・"ときどき"）が一貫して説明できる。

### 残る不確実性

「モデルを設定しました」という文字列を実際に出力するのは、ソースを読めない閉じた `claude` CLI バイナリである。releash 側に原因（prompt 欠落・テキスト注入）が**無い**こと、および初回ターン限定の構造的特異点が**この set_model 経路だけ**であることは確定しているが、CLI 内部の最終挙動は実機再現でのみ 100% 確証できる。

### 目的

新規セッション初回ターンにおける上記の構造的特異点を解消し、ユーザーの初回指示が定型応答に化けず正しく実行されるようにする。

## Requirements / 要求事項

### R1. モデルを init 段階で確立する

- 新規セッションのモデルは、最初のユーザーメッセージより前に init 段階で確立されること。
- 初回ユーザーメッセージ直前に送られる set_model control request に、モデル確立を依存させないこと。

### R2. 初回ターンで定型応答が混入しない

- 新規セッション初回ターンで普通の指示テキストを送った際、「モデルを設定しました。…」のようなモデル変更のみの定型応答が単独で返らないこと。
- ユーザーの初回指示が当該ターンで通常どおり実行されること。

### R3. ライブモデル変更・resume の挙動を維持する

- セッション実行中のモデル変更（`set_agent_model` → `set_active_process_model`）は、従来どおり即時 `setModel` でブリッジへ同期され、変更が反映されること。
- resume 時は再 spawn の init がモデルを確立することで、resume 後のセッションでも正しいモデルが使われること。
- 2 ターン目以降の通常のメッセージ送受信に回帰がないこと。

## Acceptance Criteria / 受け入れ基準の概要

- AC1: 実機ビルドで新規セッションを開始し、初回ターンに普通の指示テキストを複数回送って、いずれの試行でもモデル変更のみの定型応答が単独で返らず、指示が実行される（R2 の再現が消えることを確認）。
- AC2: セッション実行中にモデルを切り替えると、その後のターンで切り替え後のモデルが使われる（R3）。
- AC3: resume したセッションで、意図したモデルが使われ、初回・以降のターンとも指示が正しく実行される（R3）。
- AC4: 既存の関連ユニットテスト（`build_init_cmd_*` 等）が通り、回帰がない。

## Scope / スコープ

- 新規セッションのモデル確立タイミングを init 段階へ移す変更。
- 上記に伴う、初回ユーザーメッセージ前の set_model control request への依存の解消。
- ライブモデル変更・resume が従来どおり機能することの維持。
- 関連ファイル:
  - `src-tauri/src/infrastructure/agent_session/runtime/bridge_common.rs`（`spawn_bridge_process` / `build_init_cmd` / `sync_pre_turn_settings` / `set_active_process_model`）
  - `src-tauri/resources/claude-sdk-bridge.mjs`（`handleInit`）

## Non-goals / 非スコープ

- `claude` CLI バイナリ内部の挙動変更（ソース非公開・変更不可）。
- permission mode 同期（setMode）の挙動変更。本件はモデル確立経路のみを対象とし、setMode は対象外。
- モデル選択 UI・モデルレジストリ・既定モデル解決ロジックの仕様変更。
- Agent 以外のバックエンド固有の挙動拡張。

## Assumptions / 仮定

- 仮定A: Spec ディレクトリ名は `docs/specs/issues-1193` とする（提出データ仕様の例 `docs/specs/issues-NNN` に準拠）。
- 仮定B: Issue 記載の「修正方針（案）」に沿い、(1) spawn 時の init コマンドに `model` を乗せる、(2) ブリッジ `handleInit` で `options.model` に設定する（SDK の `Options.model` を利用）、(3) `sync_pre_turn_settings` の毎ターン `setModel` 送信を廃止する、という方針を採用前提とする。詳細な実装判断は design.md で確定する。
- 仮定C: モデルは spawn 時に lazy 解決されて常に非 null である（`selected_model` フィールドは互換のため `Option` のまま）という現行コードの前提を維持する。
- 仮定D: 最終的な不具合解消の確証は、CLI 内部挙動が非公開であるため、ユニットテストではなく実機ビルドでの再現確認に依存する。

## Open Questions

なし
