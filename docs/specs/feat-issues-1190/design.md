# Design

対象 Issue: #1190
対象 requirements: `docs/specs/feat-issues-1190/requirements.md`
対象 behavior: `docs/specs/feat-issues-1190/behavior.md`

本書は requirements / behavior と現行実装の調査結果に基づき、復帰 Session の会話コンテキスト引き継ぎを成立させるための実装設計を定義する。

## 概要

復帰した Session で Agent が会話コンテキストを失う症状（#1190）は、表示用 `messages` の復元経路と、Agent プロセス側のコンテキスト復帰経路が独立しており、後者の成立が保証されていないことに起因する。調査で確認した直接原因は次の 2 点。

1. **復帰識別子の永続化が `turn_complete` 正常系に限定されている。**
   - Claude backend: `session_ready` 受信時点では `proc.sdk_session_id`（メモリ）にのみ保持され、`turn_complete && exit_code == 0` のときだけ `ChatSession.agent_session_id` へ永続化される（`bridge_common.rs` 2588-2605 / 2700-2714）。
   - Codex backend: `thread/started` で得た `thread_id` は一時状態 `AppServerBridgeState.thread_id` に保持されるのみで、`turn_complete` 経路（`bridge_common.rs` 5055-5128）で `ChatSession.agent_session_id` へ永続化されていない。
   - このため、正常なターン完了を経ずに Session が閉じる／再起動するケースで復帰識別子が欠落し、復帰時に native resume へ渡す値が無くなる。

2. **復帰時にコンテキスト成立を保証・検証する経路が無い。**
   - `restore_session_state()`（`lifecycle_controller.rs` 16-26）は `state` を `Closed → Idle` に遷移させるだけで、復帰識別子の有無やコンテキスト引き継ぎ可否を判定しない。
   - Bridge へ `messages` を再注入する経路は存在しない（`claude-sdk-bridge.mjs` は `options.resume = cmd.sessionId` のみ）。識別子が欠落すると Agent は新規セッションとして開始するが、UI 上は `messages` が復元されているため「引き継ぎ済み」に見える。

本設計では、(a) 復帰識別子を欠落させない永続化、(b) native resume を優先しつつ識別子欠落時は `messages` を再注入するフォールバック、(c) コンテキスト引き継ぎ可否の判定とその UI への反映、の 3 点で対処する。これにより全 backend で「UI の見え方」と「Agent の記憶状態」を一致させる（requirements / behavior の guarantee = 案 A）。

## 変更対象

### Rust（src-tauri）

- `src-tauri/src/infrastructure/agent_session/runtime/bridge_common.rs`
  - `session_ready` 受信時に復帰識別子を `ChatSession.agent_session_id` へ永続化する処理を追加（メモリ保持に加える）。
  - Bridge `init` コマンドへ再注入用の会話履歴を載せる経路を追加（`build_init_cmd()` の拡張）。
  - 復帰開始時に「native resume を使うか／再注入を使うか／いずれも不可か」を決める判定（後述 `ContextRestorePlan`）を追加。
- `src-tauri/src/infrastructure/agent_session/runtime/codex.rs` / `codex_app_server.rs`
  - `thread/started` 受信時に `thread_id` を `ChatSession.agent_session_id` へ永続化する処理を追加。
  - `thread_id` 欠落時に再注入用履歴を `thread/start` の初回ターンへ載せる経路を追加。
- `src-tauri/src/usecase/agent_session/session/mod.rs`
  - `ChatSession` にコンテキスト引き継ぎ状態を表すフィールドを追加（後述 `context_carry`）。
- `src-tauri/src/usecase/agent_session/session/lifecycle_controller.rs`
  - 復帰時は Session の lifecycle 状態のみを戻し、runtime 起動前に `context_carry` を確定しない。
- `src-tauri/src/resources/claude-sdk-bridge.mjs`
  - `init` で再注入履歴を受け取ったとき、それを初回プロンプトの文脈として Agent へ渡す処理を追加。
- `src-tauri/src/adaptor/protocol/`・`adaptor/controller/command/agent_session/`
  - `SessionSummary` / レスポンス DTO にコンテキスト引き継ぎ状態を露出。

### フロントエンド（src）

- `src/types/session.ts` — `ChatSession` / `SessionSummary` に引き継ぎ状態フィールドを追加。
- `src/components/panels/AgentChatPanel/AgentChatPanel.tsx`・`src/remote/components/RemoteAgentPanel.tsx`
  - 引き継ぎが成立しなかった Session を「引き継ぎ済み」と誤認させない表示（バッジ／注記）。表示用フォーマットのみ。ロジックは Rust 側。

## アーキテクチャと責務分割

ロジックは Rust に集約する（`.claude/rules/rust-first-logic.md`）。フロントは引き継ぎ状態を受け取って表示するのみ。

### 復帰時のコンテキスト引き継ぎ判定（infrastructure runtime）

復帰 Session の起動時に、以下の優先順で引き継ぎ手段を決定する。これを `ContextRestorePlan` として表現する（infrastructure runtime 内の値オブジェクト想定）。

1. **native resume**: `ChatSession.agent_session_id` が存在する場合、従来どおり backend native の resume を用いる（Claude = `options.resume`、Codex = `thread/resume`）。
2. **再注入フォールバック**: `agent_session_id` が欠落しているが `messages` に引き継ぐべき会話が存在する場合、`messages` を Agent へ再注入してコンテキストを復元する。
3. **引き継ぎ不要**: 復帰元が無い新規 Session、または引き継ぐべき会話が存在しない場合は、コンテキストなしで通常起動する。
4. **引き継ぎ不成立**: 上記 1・2 のいずれも適用すべきだが成立させられない例外（再注入処理自体が失敗する等）の場合、引き継ぎ不成立として扱う。

native resume と再注入は同一ターンで併用しない（コンテキストの二重化を避ける、相互排他）。

> 方針の根拠: requirements / behavior は「全 backend で引き継ぎを保証する（案 A）」とし、Non-goals で「SDK / CLI の resume 仕様の変更は対象外。Releash 側で **利用・補完** する範囲に留める」とする。native resume を第一手段として**利用**し、欠落時に `messages` 再注入で**補完**する本方針はこれに整合する。再注入一本化（native resume を使わない）は代替案として「リスクと代替案」に記載する。

### 復帰識別子の永続化（infrastructure runtime）

- Claude: `session_ready` 受信時（`bridge_common.rs` 2588-2605 付近）に、メモリ保持に加えて `ChatSession.agent_session_id` を永続化する。`turn_complete` 正常系での保存は維持（識別子が変化する場合の更新として）。
- Codex: `thread/started`（`NOTIFY_THREAD_STARTED`）で `thread_id` を取得した時点で `ChatSession.agent_session_id` を永続化する。
- 既存の「resume 失敗時に stale な `agent_session_id` をクリアする」処理（`bridge_common.rs` 2642 付近等）は維持し、native resume mismatch では当該 process を再利用せず close/crash させる。クリア後の次回起動・次ターンは再注入フォールバックへ落ちるようにする。

### 引き継ぎ状態の表現と露出（usecase → adaptor → front）

`ChatSession` にコンテキスト引き継ぎ状態を保持し、`SessionSummary` および Session 取得系レスポンス DTO に露出する。フロントはこの値で「引き継ぎ済み／不成立」の見え方を切り替える。

## データモデルまたは型

### ChatSession への追加フィールド（仮定）

```rust
pub struct ChatSession {
    // 既存フィールド ...
    pub agent_session_id: Option<String>,

    /// コンテキスト引き継ぎ状態。復帰起動時に確定する。
    /// 既存（旧フォーマット）Session では None として扱い、破壊しない。
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub context_carry: Option<ContextCarryState>,
}

/// 復帰 Session のコンテキスト引き継ぎ状態（外部観測用）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextCarryState {
    /// native resume でコンテキストを引き継いだ。
    Resumed,
    /// messages 再注入でコンテキストを引き継いだ。
    Reinjected,
    /// 引き継ぐべきだったが成立しなかった例外ケース。
    Failed,
}
```

- 引き継ぐべきコンテキストが無い状態（新規 Session 等）は `context_carry: None` / JSON フィールド省略（フロントでは `null` 相当）で表し、`ContextCarryState` の variant には含めない。no-context の公開表現を optional field の欠落に統一する。
- 既存の永続化済み Session（`context_carry` フィールドを持たない JSON）を読み込むと no-context 相当として復元され、`messages` 等を破壊しない（`#[serde(default)]`）。
- フロント側 `src/types/session.ts` の `ChatSession` / `SessionSummary` に対応する `contextCarry?: ContextCarryState | null` を追加（camelCase）。

> 仮定: `context_carry` は optional な 1 フィールドの追加に留め、`messages` / `agent_session_id` 等の保存モデルは変更しない。これは Non-goals「永続化フォーマットの全面刷新は対象外」に反しない。

### 再注入用の Bridge 入力（仮定）

`build_init_cmd()` の `init` メッセージに、再注入時のみ会話履歴を載せる。

```jsonc
{
  "type": "init",
  "cwd": "...",
  "sessionId": null,            // 再注入時は resume を使わないため null
  "restoreContext": {           // 再注入時のみ付与
    "messages": [
      { "role": "human", "content": "..." },
      { "role": "agent", "content": "..." }
    ]
  }
}
```

- 再注入履歴は `ChatMessage` のうち外部観測に必要な最小限（`role` / `content`）を採用する。`activities`（ツール実行詳細）・`thinking` は再注入の主目的（直前の会話文脈の参照）には不要なため初版では含めない（後述 仮定）。
- `MessageRole::System` は再注入対象から除外する（運用上のシステム注記であり会話文脈ではないため）。

## 処理フロー

### A. 通常ターン中の識別子永続化（バグ修正の中核）

1. Bridge 起動 → `session_ready`（Claude）/ `thread/started`（Codex）受信。
2. 受信時点で `proc` メモリへ識別子を保持し、**同時に** `ChatSession.agent_session_id` を永続化する。
3. `turn_complete` 正常系では従来どおり最新識別子で `agent_session_id` を更新（変化時のみ実質書き込み）。

### B. 復帰起動時のコンテキスト引き継ぎ

1. `restore_session`（履歴から復帰）では lifecycle 状態のみを `Idle` に戻し、保存済み `messages` / `agent_session_id` / `context_carry` を破壊しない。
2. `start_agent_session`（再起動後に開く）または最初の turn 開始時に runtime が `ChatSession` を読み込み、`ContextRestorePlan` を決定:
   - `agent_session_id` あり → `Resumed`（native resume）。
   - `agent_session_id` 無し かつ 引き継ぐべき `messages` あり → `Reinjected`（再注入）。
   - 引き継ぐべき会話無し → `None`。
3. Bridge 起動:
   - `Resumed`: `sessionId = agent_session_id` を渡す（Claude）／ `thread/resume`（Codex）。
   - `Reinjected`: `restoreContext.messages` を載せて新規起動（`sessionId = null` / `thread/start`）。初回ターンで Agent が当該履歴を文脈として参照できる状態にする。
4. 起動・初回ターン成立後に `ContextCarryState` を確定し `ChatSession` へ保存。`Resumed` 中に resume 失敗や native resume mismatch が観測された場合は stale 識別子と未成立の carry 表示をクリアし、当該 process を再利用せずに次回起動・次ターンを `Reinjected` で再試行する。再注入も成立しない場合は `Failed`。
5. `ContextCarryState` を `SessionSummary` / レスポンス DTO で front へ返す。

### C. 引き継ぎ不成立時の UI

1. `context_carry == Failed` の Session は、UI 上で「引き継ぎ済み」を示す表現を出さず、引き継ぎ不成立を識別できる表示（バッジ／注記）を行う。
2. 発話自体はブロックしない。`Failed` の Session で発話すると、Agent は引き継ぐべき過去コンテキストなしで応答する（behavior の解消済み Open Question に準拠）。

## エラー処理

- **永続化失敗**: `session_ready` / `thread/started` 時の `save_session` が失敗しても Bridge 起動自体は継続する（従来挙動を壊さない）。ただし識別子が未永続化のまま閉じると次回復帰が `Reinjected` に落ちるため、引き継ぎ自体は保証される。
- **native resume 失敗**: backend が resume 失敗を通知した場合、stale な `agent_session_id` をクリア（既存処理を維持）し、`Reinjected` フォールバックへ遷移。最終的に成立しなければ `Failed`。
- **再注入失敗**: Bridge への `restoreContext` 受け渡しや初回プロンプト合成に失敗した場合、`Failed` とし、発話続行は許可する（コンテキストなし応答）。
- **旧フォーマット Session**: `context_carry` 欠落は `None` 相当。`messages` / `agent_session_id` は読み取りのみで破壊しない。
- **backend 不一致**: 復帰時の backend 解決（`resolve_session_backend`）は既存処理を維持。Claude / Codex で `ContextCarryState` の意味（引き継ぎ手段の別を問わず「引き継いだか否か」）を一致させる。

## テスト方針

`docs/architecture/TEST.md` と CLAUDE.md のテスト方針に従い、Rust 側ロジックを単体テスト中心で検証する。外部プロセス（実 Agent / 実 SDK）は起動しない。

### Rust 単体テスト

- **識別子永続化（A）**:
  - `session_ready` 受信相当の入力で `ChatSession.agent_session_id` が永続化されること（Claude）。
  - `thread/started` 受信相当で `agent_session_id` が永続化されること（Codex）。
  - `turn_complete` 正常系での更新が従来どおり機能すること（回帰）。
- **`ContextRestorePlan` 決定（B-2）**:
  - `agent_session_id` あり → `Resumed`。
  - `agent_session_id` 無し + `messages` あり → `Reinjected`。
  - 会話無し → `None`。
  - resume 失敗後の再試行で `Reinjected` → 不成立で `Failed`。
- **再注入履歴の構築**:
  - `System` ロールが除外され、`Human`/`Agent` のみが順序保持で含まれること。
  - 空 `messages` で再注入が選択されないこと。
- **永続化互換**:
  - `context_carry` 欠落 JSON を読み込んでも `messages` 等が保持され、`None` 相当になること。
- **DTO 露出**:
  - `SessionSummary` / レスポンスに `context_carry` が反映されること。

### フロントエンド単体テスト（Vitest）

- `AgentChatPanel` / `RemoteAgentPanel`: `contextCarry == "failed"` の Session で「引き継ぎ済み」と誤認させない表示が出ること、`resumed`/`reinjected`/`none` では出ないこと。
- Tauri API はモック。

### 手動 / 統合確認（Success Criteria に対応）

- 数ターンで文脈を作った Session を、再起動後／履歴から復帰し、文脈依存の質問で過去会話を踏まえた応答になること（Claude / Codex 双方）。
- 識別子の永続化が正常フローで欠落しないこと。
- 引き継ぎ不成立ケースで UI が誤認させないこと。

## リスクと代替案

### リスク

- **再注入の忠実度**: `messages` 再注入は表示用履歴の `role`/`content` を渡すのみで、ツール実行結果・thinking の内部状態までは復元しない。直前会話の文脈参照（behavior の「文脈依存の発話」）には十分だが、ツール結果に依存する高度な継続では native resume と差が出る。→ native resume を優先する設計で影響を最小化。
- **resume 成功/失敗の検出**: Claude SDK の resume はサイレントに新規開始する場合があり、runtime での成立検出に限界がある。→ 既存の resume 失敗通知ハンドリングを利用しつつ、不確実なケースは `Reinjected` フォールバックで吸収する。検出精度は別途検証する（下記 Open Questions）。
- **トークン量**: 再注入は履歴全量を初回プロンプトへ載せるため、長い会話でトークンを消費する。コンテキスト量最適化（要約・トリミング）は Non-goals のため初版では行わない。自然な切り詰めは許容。
- **既存 Session への影響**: フィールド追加・永続化タイミング変更が旧フォーマット Session や通常ターンへ波及するリスク。→ optional フィールド + `serde default`、`turn_complete` 経路の保持で回帰を抑える。

### 代替案

- **再注入一本化（native resume を使わない）**: 全 backend で `messages` 再注入に統一し、復帰識別子に依存しない。引き継ぎ可否の判定が単純化し「見え方と記憶の不一致」を構造的に排除できる利点があるが、(1) native resume が持つツール状態・効率を捨てる、(2) 毎回フル履歴を再送するトークンコスト、(3) Non-goals の「resume を利用・補完する範囲に留める」という方針との整合性、の点で初版方針からは外す。要確認事項として Open Questions に記載。
- **復帰時に必ず識別子検証ターンを走らせる**: 復帰直後にダミーターンで resume 成立を確認する案。挙動が重く、通常フローを変えるため採用しない。

## 仮定

- スコープは Claude / Codex 両 backend。再現が片側のみでも、もう一方の引き継ぎ成立を確認対象に含める（requirements 仮定に準拠）。
- 引き継ぎ手段は native resume 優先・再注入フォールバックの併用（案 A）。同一ターンで両者を併用しない。
- `context_carry` は optional な 1 フィールド追加に留め、保存モデルの全面刷新はしない。旧フォーマット Session は破壊しない。
- 再注入履歴は `Human`/`Agent` の `role`/`content` を順序保持で渡し、`System`・`activities`・`thinking` は初版では含めない。
- 「会話コンテキストを引き継ぐ」は復帰前のやり取りを Agent が参照できる状態を指し、トークン上限等による自然な切り詰めは対象外。
- UI 変更は表示用フォーマット（バッジ／注記）に限定し、メッセージ履歴表示 UI の仕様は変更しない（Non-goals）。

## Open Questions

なし

（解消済み: 引き継ぎ手段は「native resume 優先 + 識別子欠落時に `messages` 再注入」の併用方針で確定。再注入一本化は採らない。）
