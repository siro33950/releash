# Releash

**AIに任せきりでは、意図通りの実装は実現しない。AIエージェントのリードを取り戻すためのデスクトップアプリ。**

## なぜReleashか

Claude Code に「このバグ直して」と頼めば、それなりに動くコードは返ってくる。だが、それが意図通りか・考慮漏れがないかは別問題だ。最終的にあなたがdiffを読み、考慮漏れを指摘し、テストを書かせ、修正を促すことになる。AIエージェントの登場で「コードを書く時間」は減ったが、「**意図と実装をすり合わせる時間**」はむしろ増えている。

その時間の中で発生するもの：

- diffから得た気づきを、毎回チャットに長文で書き起こす
- 同じレビュー観点を毎回口頭で繰り返す（テストは？エラー処理は？仕様充足は？）
- 並行する複数タスクの進行状況・停止地点を見失う
- 承認した内容の記録が残らない
- Agentの「実装した」「テスト通った」は、実態と一致するとは限らない

Releashは、**AIに任せきりだとこぼれ落ちるこれらの摩擦をデスクトップアプリ側で吸収する**ことで、意図通りの実装に到達するまでの時間を短くする。

## Releashが提供するもの

### 1. Diffにコメント → AIに直接渡る

シンタックスハイライト付きdiffビューア（gutter / inline / split）の任意の行にコメントを記す。送信操作により `@path:Lstart-Lend` 形式に構造化され、AIセッションへ届く。AIは指摘対象を正確に把握する。送信済みコメントは記録され、画像 / Markdown の差分も専用ビューアで扱える。

### 2. AIエージェントの作業をワークフローとして定義する

ワークフローをYAMLで宣言することで、AIの振る舞いを構成する。順序・並列・反復・承認ゲートを組み合わせ、AIに実行させる手順を定義する。構造はあなたが定め、AIは各ステップの中身を埋める。

Releash自身の開発に使用しているワークフローをビルトインで同梱している。

### 3. Worktree単位でセッション・Diff・ターミナル・承認状態が連動する

複数タスクの並行進行は、各タスクをworktreeに分けて行う。Releashでのworktree切替に連動し、AIセッション・Diff・ターミナル・コメント・承認状態がすべて切り替わる。

### 4. 進行と状態を画面で常に把握できる

各AIセッションの状態（待機中 / 動作中 / 許可待ち）はリアルタイムに表示される。ワークフロー実行中は Timeline / Step detail / Step conversation を Workflow パネルで観測できる。許可待ちは Slack / Discord 互換 Webhook で通知可能（`Always` / `WhenInactive` の抑制制御）。

## 機能一覧

| 領域 | 内容 |
|---|---|
| Diffレビュー | gutter / inline / split ビュー、Shikiハイライト、インラインコメント、画像 / Markdown 専用ビューア |
| コメント → AI送信 | 構造化送信（`@path:L1-L10` + 抜粋）、送信履歴の記録 |
| Workflow Engine | `agent` / `approval` / `parallel` ノード、cycle guard、output contract（自動修正prompt）、aggregate（all_match / any_match）、transition rules、input/output contract |
| Workflow Panel | Run history、Active run、Timeline、Step detail、Step conversation transcript |
| Workflow CLI | `list` / `runs` / `status` / `logs` / `approve` / `reject` / `abort` |
| Worktree管理 | worktree作成 / 削除 / 切替、worktree毎にセッション・Diff・ターミナル・承認状態を保持 |
| Agent対応 | Claude（Anthropic Claude Agent SDK）/ Codex、状態を待機中 / 動作中 / 許可待ちでリアルタイム表示 |
| ターミナル | portable-pty による真のPTY、bash / zsh / fish のシェル統合でコマンド完了検出 |
| 通知 | Slack / Discord 互換 Webhook、`Always` / `WhenInactive` で抑制制御 |
| Git | branch / commit / status / diff / stage / worktree / log、`gh` 経由でPR / Issue取得 |

## Roadmap

- **複数AIによる相互レビュー** — 複数のAIが独立にdiffをレビューし、互いの指摘を議論・合意形成してから人間に渡す。指摘ノイズを減らし、人間の判断負荷を下げる
- **コード構造情報のAIへの提供** — エディタ機能を持たず、定義・参照などの構造情報をAIに渡してレビュー精度を上げる
- **モバイル向けネイティブアプリ** — スマホからworktreeの状況確認・承認操作・許可待ち応答を行う専用アプリ

## Getting Started

**対応プラットフォーム**: macOS（Linux / Windows は未対応）

最新のインストーラは [Releases ページ](https://github.com/siro33950/releash/releases/latest) から取得する。

任意: [GitHub CLI](https://cli.github.com/)（`gh`）— PR / Issue 取得に使う場合

### ソースからビルド

前提: [Node.js](https://nodejs.org/)（v18+）, [pnpm](https://pnpm.io/), [Rust](https://www.rust-lang.org/tools/install), [Tauri v2 prerequisites](https://v2.tauri.app/start/prerequisites/)

```sh
pnpm install
pnpm tauri dev
```

## 技術スタック

| Layer | Technology |
|---|---|
| Desktop shell | Tauri 2 (Rust) |
| Backend | Rust（git2 / tokio / clap） |
| Frontend | React 19 + TypeScript + TailwindCSS 4 + Shiki |
| Terminal | portable-pty + bash / zsh / fish シェル統合 |

## ライセンス

MIT OR Apache-2.0
