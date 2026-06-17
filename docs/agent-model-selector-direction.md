# Agent モデル選択 UI 方針

Agent チャット下部の「バックエンド選択 + モデル選択」の 2 段ドロップダウンを、プロバイダーアイコン付きの単一フラットリストに統合する。

## 現状

- `BackendSelector`（backend 切替）と `ModelSelector`（model 切替）が別ドロップダウン。
- ラベルは backend 名 / 生モデル ID（例: `claude-opus-4-8`）のみ。アイコン・整形表示名なし。
- モデル一覧の出所は Rust の domain 定数 `CLAUDE_FIXED_MODELS` / `CODEX_FIXED_MODELS`（`src-tauri/src/domain/agent_session/value_objects/agent_models.rs`）。

## 目指す形

全バックエンド横断のモデルエントリを 1 つのリストで提示する。各エントリに整形表示名・選択中チェックを描画し、正しいプロバイダーアイコンを引ける場合のみアイコンも描画する。

## アイコン

- 取得元: `@lobehub/icons-static-svg`（AI/LLM ブランド専用、依存なしで公式カラー SVG を提供）。会社ロゴ（Anthropic / OpenAI）ではなく **Claude / Codex のプロダクトアイコン**を使う。`simple-icons` は Codex / OpenAI を含まないため採用しない。
- `backend` フィールド（`claude` / `codex` …）から SVG アセットの URL を引くマッピングをフロントに持ち、`<img>` で描画する。
- 対応アイコンが存在しない backend は**フォールバックなしで非表示**にする。誤ったアイコンや代用文字は出さない。
- 色は各プロバイダーの**公式ブランドカラー**（`*-color.svg`）で表示する。

## データ構造

モデル選択肢 1 件を以下のエントリで表現する。Rust 側がソース。

```text
ModelEntry {
  id:           一意 ID（フロントが唯一扱う値）
  display_name: 表示用整形名（例: "Opus 4.8"）
  backend:      "claude" | "codex"
  model_id:     実モデル ID（例: "claude-opus-4-8"）
}
```

- フロントは `display_name` を表示し、`backend` からアイコンを引き、選択時は `id` のみをサーバへ送る。
- Rust は `id → (backend, model_id)` を解決し、既存の起動 / `setModel` ロジックへ流す。
- `backend` / `model_id` の対応・解決ロジックは Rust に閉じる（rust-first 原則）。

## 振る舞い

- セッション開始前: 全バックエンドのエントリを選択可能。
- セッション開始後（`session.messages.length > 0` または `agentSessionId` あり、または streaming 中）:
  現在の backend と異なるエントリは disabled。同一 backend のモデル間切替は許可。
- 判定式は既存の `canChangeBackend`（`BoundSessionChat.tsx:204`）を流用し、各エントリの disabled を
  `!canChangeBackend && entry.backend !== currentBackend` で算出する。

## スコープ

- 対象: 既存の Claude / Codex バックエンドに対する構造変更と UI 適用。
- スコープ外: Gemini / Grok など新規バックエンドの追加。型に `backend` 種別を持たせるので、将来の追加余地は残す。

## 影響範囲

| 層 | 変更 |
|---|---|
| Rust domain | `agent_models.rs` に表示名を追加（ID → 表示名対応） |
| Rust infra / protocol | `ModelInfo` / `ModelInfoMsg` を `{ id, display_name, backend, model_id }` に拡張、`id` 解決ロジック |
| TS 型 | `ModelInfo`（`src/types/session.ts` / `src/types/protocol.ts`）拡張 |
| フロント | `ModelSelector` を統合セレクタ化し `BackendSelector` を吸収。アイコンマッピングと選択中チェック表示。`setModel` / `setBackend` を `id` 起点に統合 |
| Remote | `src/remote/components/RemoteAgentPanel.tsx` の `<select>` も同構造に追従 |
