# agent-scrutiny: Tauri デスクトップアプリ移行方針

## これは何か

CLI Agent（Claude Code, Aider等）を全権委任で放し飼いにする開発者のための、レビュー＋軽量編集ツール。

**ワークフロー:** エージェント起動 → 放置 → 戻ってDiff確認 → 手直し → エージェントに戻す → また放置

Cursorとの違い: CursorはVS Codeの上にAIを乗せている。このツールはCLIエージェントの自由度（`--dangerously-skip-permissions`、`/architect`等のハック）をそのまま活かしつつ、視覚的レビューと軽い編集を提供する。エージェントの挙動には一切干渉しない。

## アーキテクチャ

```
┌───────────────────────────────────────┐
│            Tauri App (Mac優先)         │
│                                       │
│  ┌─────────┐ ┌─────────┐ ┌────────┐  │
│  │ファイル  │ │ Monaco  │ │ Monaco │  │
│  │ツリー    │ │  Diff   │ │ Editor │  │
│  └─────────┘ └─────────┘ └────────┘  │
│  ┌─────────────────────────────────┐  │
│  │ xterm.js + PTY (ターミナル)      │  │
│  └─────────────────────────────────┘  │
│                                       │
│  Rust バックエンド:                    │
│  - portable-pty (ターミナル管理)       │
│  - notify (ファイル監視)              │
│  - git2 (status / diff / log)        │
│  - WebSocketサーバー (リモート用)      │
│                                       │
│  レイアウト: dockview                  │
└───────────────────────────────────────┘
         │
         │ WebSocket (Tailscale経由)
         ▼
   PWA (スマホ)
   - ターミナル出力閲覧
   - 最低限の操作
```

## 設計原則

- **ターミナルとエディタは互いを知らない。** 共通のローカルファイルシステムを見ているだけ。notifyがファイル変更を検知してMonaco側を更新する
- **エージェントの制御はしない。** PTYプロキシ層やエージェント出力のパースはやらない。ターミナルは純粋に「CLIエージェントが動く黒い画面」
- **本格的なコーディングはVS Codeに任せる。** このツールは「VS Codeに戻る前のレビューステップ」＋「設定ファイルや軽微な修正」

## 技術スタック

| レイヤー | 技術 | 理由 |
|---|---|---|
| アプリフレームワーク | Tauri v2 | 軽量、Rustバックエンド、Mac対応良好、リモート用WebSocketサーバーが自然に乗る |
| フロントエンド | React + Vite | 既存資産の移植が容易、Monaco/xterm.js/dockview全てReact対応 |
| ターミナル | xterm.js + tauri-plugin-pty | VS Code採用の成熟ターミナル。terminon, Wavetermで実証済み |
| エディタ / Diff | Monaco Editor | 既存のMonacoDiffViewer.tsxを流用 |
| レイアウト | dockview | VS Codeライクなパネルレイアウト。タブ、分割、ドラッグ&ドロップ |
| ファイル監視 | notify クレート | 成熟技術、debounceだけ入れれば十分 |
| Git統合 | git2 クレート | status/diff/logの3操作。それぞれ数十行 |
| 検索 | ripgrep | Rustバックエンドから呼ぶ。grepベースの定義・参照ジャンプにも使う |
| リモート | tokio-tungstenite + PWA | Tailscale経由でNAT越え |

## スコープ

### 作るもの

- [ ] Tauri アプリ基盤（React + Vite）
- [ ] 埋め込みターミナル（xterm.js + PTY）
- [ ] Diffビューア（Monaco diffEditor + git2）
- [ ] 軽量エディタ（Monaco Editor、LSPなし）
- [ ] ファイルツリー
- [ ] ファイル監視 → Monaco自動更新
- [ ] Git統合（status / diff / log）
- [ ] 検索（ripgrep）
- [ ] grepベースの定義・参照ジャンプ（ripgrep）
- [ ] dockviewによるパネルレイアウト
- [ ] WebSocketサーバー（リモート配信）
- [ ] PWAクライアント（スマホ用）
- [ ] Tailscale経由のリモート接続

### 作らないもの

- エージェント出力のパース（「承認待ち」「完了」「エラー」等の状態検出）
- エージェント固有のプロトコルへの依存
- PTYプロキシ（出力の傍受・書き換え）
- LSP統合（後から足せる設計にはしておく）
- アカウント管理・マルチユーザー対応
- Windows / Linux対応（当面Mac専用）

**補足: ターミナルへの文字列送信（stdinへの書き込み、Ctrl+C等）は行う。** レビュー画面から指示を書いてエージェントに送り返すワークフローは初期スコープに含む。これはエージェント制御ではなく、キーボード入力の代行。どのエージェントでも動く。

### 後から足す可能性があるもの

- LSP対応（WebSocketリレーの口だけ設計に入れておく）
- 複数ターミナルセッション管理
- エージェントが触ったファイルのレビューセット表示

## 開発フェーズ

### Phase 1: Mac側MVP
- Tauri + Vite + React プロジェクト作成
- dockviewでレイアウト骨格
- xterm.js + PTY統合
- 既存MonacoDiffViewer移植
- 軽量エディタ配置
- ファイル監視（notify + debounce）

### Phase 2: Git + ナビゲーション
- git2でstatus/diff/log
- ファイルツリー
- ripgrepで検索
- grepベースの定義・参照ジャンプ

### Phase 3: リモート対応
- WebSocketサーバー（tokio-tungstenite）
- PWA UI（ターミナル出力閲覧 + 最低限の操作）
- Tailscale経由の接続テスト
- 接続トークン認証 + 同時接続1制限

## 判断の根拠

**Tauriを選んだ理由:**
- VS Code拡張はリモート対応が構造的に不自然（VS Code常時起動が前提になる）
- Theiaは拡張がVS Code拡張と同じ制約との戦いになるうえ、Electronに戻る
- 個人プロジェクトで「ワクワクするか」は持続性に直結する

**エージェント制御をしない理由:**
- `--dangerously-skip-permissions`で全権委任するなら権限プロンプトが出ない
- エージェント出力フォーマットへの依存はメンテコストの主因になる
- CLIエージェントの「行儀の悪さ」をそのまま活かすのがこのツールの存在意義

**LSPを初期に入れない理由:**
- 設定ファイル（JSON, YAML, TOML）はMonaco標準のSchema検証で十分
- ripgrepベースのジャンプで「8割は満たせる」
- monaco-languageclientのバージョン互換性がメンテコストになる
- WebSocketリレーの口だけ用意しておけば後付け可能
