# Design

対象 Issue: #1193「Agent: 新規セッション初回ターンで普通の指示が「モデルを設定しました」とだけ返ることがある」

この文書は requirements.md / behavior.md を満たす実装方針を確定する。`requirements.md` と `behavior.md` は変更しない。

## 概要

新規 Agent セッション（claude バックエンド）のモデル確立を、**初回ユーザーメッセージ直前に送る `setModel` control request** から **spawn 時の init コマンド**へ移す。これにより「モデルが最初の user message 直前の control request でのみ確立される」という構造的特異点（requirements「確定している原因」3）を解消し、初回ターンの普通の指示がモデル変更のみの定型応答に化けないようにする（R1, R2）。

ライブモデル変更（`set_active_process_model` 経由の即時 `setModel`）と resume（再 spawn の init がモデルを確立）は従来挙動を維持する（R3）。

## 変更対象

requirements の Scope に挙げられた 2 ファイルのみを変更する。

1. `src-tauri/src/infrastructure/agent_session/runtime/bridge_common.rs`
   - `build_init_cmd`: 引数に `selected_model: Option<&str>` を追加し、`Some` のとき init コマンド JSON に `"model"` フィールドを載せる。
   - `spawn_bridge_process`: 既に保持している `selected_model`（line 2375 で受領）を `build_init_cmd` へ渡す。
   - `AgentProcess::sync_pre_turn_settings`: 毎ターン送っていた `setModel` 送信ブロック（line 482–492）を削除する。`setMode` 送信は維持する。
2. `src-tauri/resources/claude-sdk-bridge.mjs`
   - `handleInit`: `cmd.model` があれば SDK の `options.model` に設定する。

`set_active_process_model`（ライブ変更経路）と `build_set_model_command`（control request 組み立て）は変更しない。

## アーキテクチャと責務分割

モデル確立の責務を「ターン直前の制御送信」から「プロセス起動時の初期化」へ移す。経路ごとの責務は次のとおり。

| 経路 | 変更前 | 変更後 |
|---|---|---|
| 新規 spawn のモデル確立 | init には無し。初回 user message 直前の `setModel` control request に依存 | **init コマンドの `model` → SDK `options.model`** で確立。`setModel` には依存しない |
| 2 ターン目以降のモデル維持 | 毎ターン `sync_pre_turn_settings` が `setModel` 再送 | init 時の `options.model` が SDK query に保持され、再送不要 |
| ライブモデル変更 | `set_active_process_model` → `setModel` control request（即時） | 変更なし（従来どおり即時 `setModel`） |
| resume | 再 spawn の init（モデル指定なし）＋ 初回 message 直前の `setModel` | **再 spawn の init `options.model`** で確立。`setModel` には依存しない |

### codex バックエンドへの非影響

`start_agent_turn` は `backend_id == CODEX_BACKEND_ID` のとき `start_codex_backend_turn` へ分岐し（bridge_common.rs:3625）、claude-sdk-bridge と `sync_pre_turn_settings` を経由しない。codex はモデルを `codex_app_server` の conversation/turn パラメータで確立しており、本変更の影響を受けない。`sync_pre_turn_settings` からの `setModel` 削除は claude セッションにのみ作用する。

## データモデルまたは型

新規の永続化型・プロトコル型は導入しない。

- `ChatSession.selected_model` / `AgentProcess.selected_model`: 互換のため `Option<String>` のまま維持する（仮定C）。モデルは spawn 時に lazy 解決され実質非 null（`resolve_selected_model` / `default_model_for`）。
- init コマンド JSON: 既存フィールドに加え `"model": <string>` を追加（`selected_model` が `Some` のときのみ）。`None` の場合はフィールドを出さず、ブリッジは SDK 既定に委ねる（既存 `setModel` の None 時挙動と整合）。
- `build_init_cmd` のシグネチャ:
  - 変更前: `(cwd, permission_mode, plan_mode, session_id, system_prompt, backend_id)`
  - 変更後: 上記に `selected_model: Option<&str>` を 1 引数追加。位置は呼び出し可読性を優先し `system_prompt` の次・`backend_id` の前を想定（最終位置は実装時に clippy/可読性で確定）。

## 処理フロー

### 新規セッション初回ターン（R1, R2）

1. `start_agent_turn`（claude）→ `spawn_bridge_process` が `selected_model`（lazy 解決済みで非 null）を保持。
2. `build_init_cmd` が init コマンド JSON に `"model"` を載せる。
3. init コマンドを stdin へ書き込み。
4. ブリッジ `handleInit` が `options.model = cmd.model` を設定し、`query({ prompt, options })` を生成。**この時点でモデルは確立済み。**
5. 初回 user message が `sync_pre_turn_settings`（`setMode` のみ）→ message 送信で届く。`setModel` control request は送られないため、「モデル変更だけのターン」が先に成立する競合が起きない。
6. CLI は最初の user message を通常ターンとして実行する。

### ライブモデル変更（R3）

`set_agent_model` → `set_active_process_model` が `build_set_model_command` で `setModel` control request を即時送信（既存どおり）。ブリッジ `handleCommand` の `case "setModel"` が `currentModelId` 更新＋`applyModelSafely` を実行し、次ターン以降へ反映。

### resume（R3）

resume は再 spawn を伴う。再 spawn の `build_init_cmd` が `selected_model`（永続値を lazy 解決）を `options.model` に載せて確立する。resume 後の初回ターンも `setModel` 非依存で実行される。

## エラー処理

- init コマンド書き込み失敗は既存の `Failed to write/flush init command` 経路で従来どおり扱う。
- ブリッジ `options.model` 設定は単純代入で例外を投げない。ライブ変更時の `applyModelSafely` は既存の try/catch（stderr ＋ `error` emit）を維持する。
- `selected_model == None`（理論上のみ）の場合、init に `model` を出さず SDK 既定へ委ねる。これは削除前の `sync_pre_turn_settings`（None 時 setModel 非送信）と同じ安全側挙動。

## 重要な実装判断（init 後の冗長 setModel 回避）

ブリッジ `handleInit` で `options.model` を設定する際、**`currentModelId` には init 由来モデルを代入しない**方針を採る。

- 理由: while ループ内 line 240 の `if (currentModelId) applyModelSafely(currentModelId)` は、`currentModelId` が非 null だとターン開始直後に `setModel` を再実行する。init で確立済みのモデルと同一でも、ターン直前に control request を流すことになり、本件で解消したい特異点を再現するリスクがある。
- 方針: `options.model` のみ設定し、`currentModelId` は `null` のまま（ライブ `setModel` を受けたときだけ非 null になる）。`options` オブジェクトは while ループ間で保持されるため、`options.model` は内部 resume ループでも維持される。`currentModelId` はブリッジ内部の line 240 / 149 でのみ参照され外部へ emit されないため、null 維持による副作用はない。

## テスト方針

requirements 仮定D のとおり、CLI 内部挙動は非公開で最終確証は実機再現に依存する。ユニットテストは「releash 側が約束する入出力」を検証する。

### Rust ユニットテスト（`bridge_common.rs` 内 `#[cfg(test)]`）

- `build_init_cmd` のモデル付与:
  - `selected_model = Some("sonnet")` で init JSON に `"model": "sonnet"` が含まれる。
  - `selected_model = None` で `"model"` キーが含まれない。
  - 既存の `build_init_cmd_*` テスト群（permission mode / system_prompt / codex 等）は新引数 `None` で更新し、回帰がないことを確認（AC4）。
- `sync_pre_turn_settings` 由来の `setModel` 非送信:
  - 既存の setModel 関連アサーション（stdin に setModel が書かれることを期待するテストがあれば）を、削除後の仕様（setMode のみ送信）へ合わせて更新する。テスト期待値は「実装に合わせて」ではなく「変更後の仕様（毎ターン setModel を送らない）」に基づいて修正する。
- `build_set_model_command` のフォーマットテスト（line 9386）は不変なので維持。

### ブリッジ（`claude-sdk-bridge.mjs`）

ブリッジに自動テスト基盤がない場合は、`handleInit` の `options.model` 設定をコードレビューと実機確認で担保する（`bridge-utils.test.mjs` の範囲外）。pure な分岐ロジックを切り出せる場合のみ単体テスト化を検討する（スコープ拡大を避け、必須化はしない）。

### 実機確認（AC1–AC3）

- AC1: 実機ビルドで新規セッションを開始し、初回ターンに普通の指示を複数回送り、いずれもモデル変更のみの定型応答が単独で返らず指示が実行される。
- AC2: 実行中にモデルを切り替え、以降のターンで切り替え後モデルが使われる。
- AC3: resume したセッションで意図したモデルが使われ、初回・以降のターンとも指示が実行される。

## リスクと代替案

- リスク1（残存不確実性）: 「モデルを設定しました」を出力するのは非公開 CLI。releash 側で原因経路（初回 message 直前の setModel）を消しても、CLI 側の最終挙動は実機再現でのみ 100% 確証できる（requirements 残る不確実性 / 仮定D）。緩和策: AC1 の複数回試行で再現消失を確認する。
- リスク2（モデル維持の退行）: 毎ターン `setModel` 再送を廃止するため、2 ターン目以降のモデルが意図せず変わらないことを保証する必要がある。SDK の `options.model` が query 生存中保持される前提に依存する。緩和策: behavior の「2 ターン目以降に回帰がない」シナリオ（Scenario Outline turn 2/3）を実機で確認。
- 代替案A（init に加え setModel も維持）: 二重確立で安全側に見えるが、初回 message 直前の setModel が残るため本件の特異点を解消できない。採用しない。
- 代替案B（init は据え置き、送信タイミングだけ前倒し）: control request と message の競合構造自体は残り、根本解消にならない。採用しない。
- 代替案C（`currentModelId` を init で設定）: 上記「重要な実装判断」のとおり line 240 で冗長 setModel を誘発しうるため採用しない。

## 仮定

- 仮定A（requirements 仮定B 準拠）: 修正方針は (1) init コマンドに `model` を載せる、(2) ブリッジ `handleInit` で `options.model` に設定、(3) `sync_pre_turn_settings` の毎ターン `setModel` を廃止、の 3 点。
- 仮定B（requirements 仮定C 準拠）: モデルは spawn 時 lazy 解決で常に非 null。`selected_model` フィールドは互換のため `Option` を維持。
- 仮定C: codex は別経路（`start_codex_backend_turn` / `codex_app_server`）でモデルを確立しており、claude-sdk-bridge / `sync_pre_turn_settings` を通らないため本変更の影響を受けない（コードで確認済み: bridge_common.rs:3625）。
- 仮定D: ブリッジ `handleInit` で `options.model` のみ設定し `currentModelId` は init 由来値で更新しない（「重要な実装判断」参照）。
- 仮定E（requirements 仮定D 準拠）: 不具合解消の最終確証は実機ビルドでの再現確認に依存する。

## Open Questions

なし
