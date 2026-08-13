# Context

- 要求の正本: [Issue #1637](https://github.com/siro33950/releash/issues/1637) 「[Agent TUI] Shift+Enter / Cmd+Enter を改行挿入（ESC+CR）としてTUIへ送る」（OPEN、2026-08-12、comment なし）。
- 補助資料（本書作成時に参照して事実を確認したもの）:
  - `src/hooks/useTerminal.ts` — Releash のターミナル surface の入力経路。現行のキー処理と PTY への送出はここを通る。
  - xterm.js upstream `src/common/input/Keyboard.ts` — Enter（keyCode 13）のエンコード規則。
  - xterm.js upstream `src/browser/CoreBrowserTerminal.ts` — `attachCustomKeyEventHandler` と IME composition 処理の関係。
  - [codex-rs/tui/src/keymap.rs](https://github.com/openai/codex/blob/main/codex-rs/tui/src/keymap.rs) — Codex の default keymap。`insert_newline` に `plain(KeyCode::Enter)` / `shift(KeyCode::Enter)` / `alt(KeyCode::Enter)` がバインドされていることを確認済み。
  - 先行事例（同方式の実装）: [superset](https://github.com/superset-sh/superset/blob/main/apps/desktop/src/renderer/lib/terminal/line-edit-translations.ts)（Electron）、[nezha](https://github.com/hanshuaikang/nezha/blob/main/src/shortcuts.ts)（Tauri 2、IME composing 除外あり）、[tessera](https://github.com/horang-labs/tessera/blob/main/src/lib/terminal/terminal-key-input.ts)（Electron）。
- 要求元が確定済みとして提示した背景:
  - 送出するバイト列は ESC + CR（`\x1b\r`）である。Claude Code（Ink の parse-keypress が meta+return として解釈。`/terminal-setup` が VS Code / Alacritty / Zed に書き込む実体も同じ 2 バイト、v2.1.228 バイナリで確認済み）と Codex（crossterm が ESC プレフィクスを Alt 修飾へ変換し、default keymap の `alt(KeyCode::Enter)` が `insert_newline`）の双方が「改行挿入」と解釈する。
  - この方式は kitty keyboard protocol のネゴシエーションに依存しない。したがって kitty 未対応の xterm.js 6.0.0 のままで機能する。
  - IME 変換中の判定条件として `isComposing` / `keyCode === 229` を挙げている。
- 完了条件に関する制約: Issue は「対象キー入力に対して PTY 入力へ送られる内容を自動検証できること」を完了条件としている。実機の手動確認のみでは完了条件を満たさない。
- 完了条件が挙げる「IME 確定の Enter では従来通り `\r` が送られる」は、未確定文字列がない状態（変換が確定した後）の Enter を指すものとして解釈し、修飾キーなしの Enter と同じ扱いとする（R-003 が担う）。
- 未確定文字列がある状態の Enter 押下では、Enter 由来のバイトを PTY へ送らず、IME が確定した文字列だけを送る（R-004）。これは修飾キーの有無を問わない確定判断である。xterm.js 6.0.0 は `keyCode === 229` の Enter では Enter 由来のバイトを送らないが、`isComposing` かつ `keyCode === 13` の Enter（WebKit 系が発火させる形）では composition を確定させたうえで Enter をエンコードする（根拠は Current Behavior）。この差を Releash 側で吸収し、どちらの経路でも同じ結果にする。
- 適用範囲に関する事実: Releash の workspace ターミナルと Agent session の TUI（`AgentSessionPanel` → `TerminalPanel`）は同一のターミナル surface 実装を共有しており、キー入力から PTY までの経路も共通である。

# Outcome

- 対象者: Releash のターミナル surface 上で TUI（Claude Code / Codex）を操作する開発者。
- 現在の問題: TUI のプロンプトに改行を挿入する手段がない。Shift+Enter も Cmd+Enter も素の Enter と同じ「送信」として扱われるため、複数行の指示を書くには Releash の外で本文を組み立てて貼り付けるしかない。
- 変更後に実現する状態: ターミナル上の TUI で Shift+Enter / Cmd+Enter が改行挿入として働き、素の Enter は従来どおり送信のままである。IME で日本語を変換中の Enter がこの変換に巻き込まれない。

# Current Behavior

Issue #1637 が報告する挙動を、2026-08-13 時点の worktree（branch `feat/issues/1637`、`55410de53` 系）の現行実装と xterm.js upstream のソースで裏付けた。

- 再現手順: worktree でターミナル（または Agent session の TUI）を開き、Claude Code または Codex を起動する。プロンプトへ文字を入力した状態で Shift+Enter または Cmd+Enter を押す。
- 実際の出力: 改行は挿入されず、その時点の入力が送信される。素の Enter を押した場合と区別がつかない。
- 経路上の根拠:
  - xterm.js（`@xterm/xterm` ^6.0.0）の Enter エンコードは `result.key = ev.altKey ? C0.ESC + C0.CR : C0.CR` であり、`shiftKey` と `metaKey` は特別扱いされない。Shift+Enter も Cmd+Enter も `\r` 1 バイトになる。ESC+CR になるのは Alt（Option）+Enter のときだけである。
  - `src/hooks/useTerminal.ts:217` のカスタムキーハンドラは、ペイン操作キー（Cmd+D / Cmd+Shift+D / Cmd+Option+矢印）だけを xterm に処理させず、それ以外は素通しする。Enter 系の分岐は存在しない。
  - そのため Shift+Enter / Cmd+Enter は xterm のエンコード結果 `\r` として `terminal.onData` に届き、既存の入力 dispatch 経路を通って PTY へ送られる。
  - カスタムキーハンドラは xterm の composition 処理（`_compositionHelper.keydown`）より前に呼ばれる。IME 変換中の keydown（`isComposing === true` または `keyCode === 229`）もカスタムキーハンドラへ到達する。
  - IME 変換中の Enter 押下の扱いは、`CompositionHelper.keydown`（`src/browser/input/CompositionHelper.ts`）で 2 経路に分かれる。`keyCode === 229` の Enter は `false` が返り、`CoreBrowserTerminal._keyDown` がキーエンコード（`evaluateKeyboardEvent` と `triggerDataEvent`）へ到達しないため Enter 由来のバイトは送られない。`isComposing` かつ `keyCode === 13` の Enter は `_finalizeComposition(false)` で composition を確定させたのち `true` が返るため、確定文字列に続けて Enter のエンコード結果（Alt なしなら `\r`、Alt ありなら `\x1b\r`）が PTY へ送られる。
  - IME が確定した文字列は composition 完了時に `triggerDataEvent` から送られ、通常の入力と同じ `onData` 経路を通って PTY へ届く。
- 既存テスト: `src/hooks/useTerminal.test.ts` にキー入力ごとの PTY 送出内容（Shift+Enter / Cmd+Enter / 素の Enter の区別）を検証するテストはない。

# Scope / Non-goals

## Scope

- ターミナル surface でのキー入力から PTY 入力までの経路における、Shift+Enter と Cmd+Enter の送出内容。
- IME 変換中の Enter 押下をこの変換の対象から除外すること。
- 上記の送出内容を自動テストで検証できるようにすること。

## Non-goals

- IME 変換中でない状態での、素の Enter、Ctrl+Enter、Alt（Option）+Enter の送出内容の変更。
- ペイン操作ショートカット（Cmd+D / Cmd+Shift+D / Cmd+Option+矢印）の扱いの変更。
- kitty keyboard protocol への対応、および xterm.js のバージョン更新。
- provider 側（Claude Code / Codex）の keybinding 設定や TUI 実装の変更。
- 改行挿入に割り当てるキーのユーザー設定化。
- ターミナル surface 以外の入力欄（チャット入力欄、ダイアログ等）のキー挙動。

# Requirements

- R-001: ターミナル surface にフォーカスがある状態で Shift+Enter を押すと、PTY 入力へ `\x1b\r`（ESC + CR の 2 バイト）が送られる。`\r` は重複して送られない。
- R-002: ターミナル surface にフォーカスがある状態で Cmd（meta）+Enter を押すと、PTY 入力へ `\x1b\r`（ESC + CR の 2 バイト）が送られる。`\r` は重複して送られない。
- R-003: 修飾キーなしの Enter を押したときに PTY 入力へ送られる内容は本変更前と変わらず、`\r` のままである。
- R-004: IME による変換中（未確定文字列がある状態）の Enter 押下では、PTY 入力へ `\x1b\r` も Enter 由来の `\r` も送られない。この Enter 押下によって PTY 入力へ送られるのは、IME が確定した文字列だけである。修飾キーの有無と種類（Shift、Cmd、Ctrl、Alt）を問わず同じである。
- R-005: 互換性要件 — Shift+Enter、Cmd+Enter、および IME 変換中の Enter 以外のキー入力について、PTY 入力へ送られる内容は本変更前と変わらない。既存のペイン操作ショートカットが xterm に処理されない扱いも維持される。
- R-006: R-001 から R-005 は、workspace ターミナルと Agent session の TUI のどちらのターミナル surface でも同じように成り立つ。

# Assumptions / Open Questions

なし。
