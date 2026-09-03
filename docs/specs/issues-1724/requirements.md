# Context

- 正本: GitHub Issue #1724 「fix(terminal): terminal 上の URL がクリックで開かない — WebLinksAddon 未導入と linkHandler 未設定」 <https://github.com/siro33950/releash/issues/1724>（label: bug / state: OPEN / milestone: なし / comment: 0 件）。この Issue 本文が要求の入力であり、追加の自由文指示はない。
- spec 配置先: `docs/specs/issues-1724`（作業ブランチ `feat/issues/1724`）。
- 対象コード:
  - `src/hooks/useTerminal.ts` — Releash 内で `Terminal`（xterm.js）を生成する唯一の箇所（`:166`）。consumer は `src/components/panels/TerminalPanel.tsx:78` のみ。
  - `package.json:39-44` — `@tauri-apps/plugin-opener` `^2.5.4` は導入済み。xterm 系依存は `@xterm/addon-fit` `^0.11.0` / `@xterm/addon-webgl` `^0.19.0` / `@xterm/xterm` `^6.0.0`。
  - `src-tauri/capabilities/normal-workbench.json:8` — main window の permissions に `opener:default` を含む。
  - `src/components/workspace/WorkspaceList.tsx:3,1304` — `@tauri-apps/plugin-opener` の `openUrl()` で PR URL を外部ブラウザへ渡している既存経路。
- 確定済みの制約（`AGENTS.md`）: アプリケーションロジックは Rust が所有し、frontend に許すのは表示とレイアウト制御、ユーザー入力の受付、Tauri command の呼び出し、表示用フォーマットのみ。本件が扱う「terminal 表示上の URL 検出」「遷移先の提示」「クリック時の外部ブラウザ起動」は、既存の `openUrl()` 経路と同じく frontend からの Tauri plugin 呼び出しと表示制御の範囲にある。

# Outcome

- 対象者: Releash の terminal（Agent session の terminal を含む）を使う開発者。
- 現在の問題: Agent が terminal に出力した URL（PR、Issue、ドキュメント、ローカル dev server 等）へ terminal から遷移できない。URL を手動でコピーしてブラウザへ貼る操作が毎回必要になり、Agent の出力を判断材料として扱う workflow で参照先へ到達する導線が欠ける。
- 変更後の状態: terminal に表示された URL をクリックするだけで、OS の既定ブラウザが当該 URL を開く。遷移先の URL はクリック前にポインタを合わせて確認できる。

# Current Behavior

Issue #1724 が報告する現象と、その裏付けとして現行コードで確認した事実を分けて記載する。

## 再現手順と結果（Issue #1724 の報告内容）

プレーンテキスト URL の場合:

1. Releash を起動し terminal を開く。
2. terminal で `echo https://github.com/siro33950/releash` を実行する。
3. 出力された URL をクリックする。

実際の結果: URL はリンクとして認識されず、クリック可能な要素が存在しないため何も起こらない。

OSC 8 ハイパーリンクの場合:

1. Releash を起動し terminal を開く。
2. terminal で OSC 8 エスケープシーケンス付きの URL を出力する。
3. 表示されたリンクをクリックする。

実際の結果: 確認ダイアログ（`Do you want to navigate to ...?`）が表示され、承諾しても遷移せず、console に `Opening link blocked as opener could not be cleared` が出力される。Tauri の WKWebView では xterm デフォルト実装（`@xterm/xterm` の `OscLinkProvider`）が使う `window.open()` が新規ウィンドウを作れず `null` を返すため。

## 現行コードで確認した事実

- `@xterm/addon-web-links` は `package.json` にも `pnpm-lock.yaml` にも存在しない（いずれも該当 0 件）。
- `src/hooks/useTerminal.ts:166-175` の `Terminal` 生成オプションは `cursorBlink` / `fontFamily` / `fontSize` / `theme` のみで、`linkHandler` は未指定。`loadAddon` に渡しているのは `FitAddon`（`:174`）と、実行時に動的 import される `WebglAddon`（`:395`）だけ。
- `src/` 全体で `linkHandler` / `registerLinkProvider` / `WebLinks` の出現は 0 件。URL 検出、リンク化、クリック時遷移、遷移先の提示を扱う自前実装はいずれも存在しない。
- したがってプレーンテキスト URL にはリンク要素が生成されず、OSC 8 リンクは xterm デフォルトの `window.open()` 経路に落ちる。ポインタを合わせても遷移先 URL を提示する表示は存在しない。
- 一方 `openUrl()` による外部ブラウザ起動は `src/components/workspace/WorkspaceList.tsx:1304` で動作しており、capability（`opener:default`）も付与済みで、terminal 側だけがこの経路に接続されていない。

# Scope / Non-goals

## 変更する対象

- terminal 表示上のプレーンテキスト URL の検出とリンク化。
- terminal 上のリンク（プレーンテキスト由来・OSC 8 由来の双方）をクリックしたときの遷移経路。
- terminal 上のリンクへポインタを合わせたときの遷移先の提示。
- 上記に必要な frontend 依存の追加。

## 変更しない対象

- Rust 側の terminal surface（PTY の生成、stream、attachment、resize、永続化）。
- terminal 以外の画面のリンク挙動（`WorkspaceList` の PR リンク等、既に `openUrl()` を経由している経路）。
- 既存の terminal のキー入力処理、IME 変換の扱い、ペイン操作キーの抑止、テキスト選択、レンダラー（WebGL）の挙動。
- OSC 8 以外のエスケープシーケンスへの新規対応。
- `http` / `https` 以外のスキームの文字列およびリンクの取り扱い。
- URL 以外の文字列（ファイルパス、Issue 番号、ブランチ名等）のリンク化。
- terminal のテーマ・配色設計。

# Requirements

- R-001: terminal に表示されたプレーンテキストの `http` / `https` URL がリンクとして認識され、クリックすると OS の既定ブラウザで当該 URL が開く。
- R-002: terminal に表示された OSC 8 ハイパーリンクのうち遷移先が `http` / `https` のものをクリックすると、OS の既定ブラウザで当該 URL が開く。
- R-003: R-001 と R-002 のいずれの経路でも、リンクをクリックしたとき Releash 本体の WebView は当該 URL へ遷移しない。
- R-004: 遷移先のスキームが `http` / `https` でない文字列および OSC 8 リンクは、リンクとして認識されない。
- R-005: R-001 と R-002 のいずれの経路でも、リンクへポインタを合わせると遷移先の URL 全体が提示される。
- R-006: R-001 と R-002 のいずれの経路でも、リンクのクリックは確認の応答を求めず、クリックだけで遷移する。
- R-007: terminal の幅で折り返され複数行に分断された URL のうち、クリックした行から上方向・下方向それぞれ 2,048 文字までの連結範囲に URL 全体が収まるものは、1 つのリンクとして扱われる。

# Assumptions / Open Questions

## Assumptions

なし。

## Open Questions

なし。
