# Design

## The actual design

### Architecture

#### キー入力変換の責務

`src/hooks/useTerminal.ts` の xterm 入力アダプタが、DOM `KeyboardEvent` の Shift+Enter と Cmd（meta）+Enter を `\x1b\r` へ変換する責務を持つ。修飾キーと IME composition は browser の keyboard event としてのみ確定できるため、この責務は `AGENTS.md` が frontend に認める入力受付に該当する。Issue #1637 も変更対象として `src/hooks/useTerminal.ts` の `attachCustomKeyEventHandler` を名指ししている。新しい domain decision や terminal state は frontend に持たせない。

変換後の入力は xterm 6.0.0 の公開 `Terminal.input` から既存の `Terminal.onData` へ戻し、`dispatchInput`、startup input buffer、attachment ごとの sequence、WebSocket または Tauri command、Rust の Terminal Surface input ingress という現在の経路をそのまま通す。入力の順序、実行中判定、attachment、retry/recovery、および PTY 書き込みの owner は引き続き backend の `terminal_surface` とする。根拠は `src/hooks/useTerminal.ts` の既存入力経路、`src-tauri/src/domain/terminal_surface/entities/terminal_surface_input_ingress.rs`、および `docs/architecture/README.md` の Terminal Surface ownership である。

主要な変更対象は次のとおり。

| Path | 変更の要旨 |
| --- | --- |
| `src/hooks/useTerminal.ts` | 既存の `attachCustomKeyEventHandler` に、対象 Enter chord の IME 除外、`\x1b\r` 投入、xterm 既定エンコード抑止を追加し、`Terminal.onData` の受け口で IME 変換中の Enter 由来バイトを抑止する |
| `src/hooks/useTerminal.test.ts` | B-001 から B-006 の PTY 入力契約を hook unit test で自動検証する |

### Interface

利用者向け契約は、ターミナル surface にフォーカスがあるとき、Shift+Enter または Cmd（meta）+Enter が PTY 入力 `\x1b\r` を一度だけ生成することである。新しい設定、UI、Tauri command、WebSocket message、protocol version は追加しない。

内部境界には xterm 6.0.0 の公開 `attachCustomKeyEventHandler` と `Terminal.input(data, true)` を使用する。後者が発火する既存 `onData` を唯一の送出入口とし、custom key handler から `write_terminal_surface` や WebSocket を直接呼ばない。既存の Tauri `write_terminal_surface` と WebSocket `write` frame の `data: string` 契約は変更せず、変換済みの 2 文字を従来と同じ一つの入力単位として渡す。

互換境界は非破壊とする。修飾キーなしの Enter、Ctrl+Enter、Alt（Option）+Enter、追加修飾キーを伴う Enter、その他のキー、および既存ペイン操作 chord は従来どおり xterm または既存抑止処理へ委譲する。

### Data Model

新しい record、frontend state、backend state、永続データは追加しない。`\x1b\r` は既存 terminal input の `data` としてその場で送出し、保持や versioning の対象にしない。

### Database

該当なし。

### UI/UX

ターミナル surface 上の Shift+Enter と Cmd+Enter のキー操作だけを変更する。表示、設定項目、操作フローは追加しない。

### Algorithm

custom key handler の Enter 変換分岐は `key === "Enter"` に限定する。この分岐内で最初に `isComposing === true` または `keyCode === 229` を判定し、該当時は変換せず xterm の composition 処理へ委譲する。Enter 以外はこの分岐へ入れず、既存のペイン操作 chord 判定をそのまま適用する。

IME 変換中の Enter は、委譲だけでは R-004 を満たさない。xterm 6.0.0 の `CompositionHelper.keydown` は `keyCode === 229` の Enter に対しては `false` を返してキーエンコードへ到達させないが、`isComposing` かつ `keyCode === 13` の Enter（WebKit 系が発火させる形。macOS の Tauri WebView はこちら）に対しては `_finalizeComposition(false)` で確定文字列を送出したのち `true` を返し、Enter を通常どおりエンコードする。そのため後者では確定文字列に続けて Enter 由来の `\r`（Alt 併用時は `\x1b\r`）が `onData` へ届く。

この差を吸収するため、custom key handler は `isComposing` かつ `keyCode === 13` の Enter keydown を検出したときに、直後に届く Enter 由来バイトの抑止フラグを立てる。`onData` の受け口はフラグが立っている間に届いた `\r` または `\x1b\r` を一度だけ捨て、フラグを下ろす。フラグは `queueMicrotask` でも解除する。xterm がこの keydown に対して発火させるデータイベントは同一 task 内で同期的に完了するため、次の microtask まで残ったフラグは対象イベントが発生しなかったことを意味し、後続の入力へ波及させない。`keyCode === 229` の Enter ではフラグを立てない。この経路では Enter 由来バイトが発生せず、確定文字列を誤って捨てうるためである。フラグの判定は `Terminal.onData` の受け口に閉じ、`inputDispatchRef` 経由の非キー入力経路には影響させない。

この抑止は修飾キーの有無を問わない。IME 変換中の Shift+Enter、Cmd+Enter、Ctrl+Enter、Alt+Enter のいずれでも、PTY 入力へ届くのは IME が確定した文字列だけになる。

composition 中ではない Enter のうち、変換対象は次のいずれかを満たす入力に限定する。

- Shift のみが押され、meta、Ctrl、Alt は押されていない。
- meta のみが押され、Shift、Ctrl、Alt は押されていない。

対象 chord の `keydown` では browser の既定処理を `preventDefault` し、`Terminal.input("\x1b\r", true)` を一度呼ぶ。対象 chord の keydown、keypress、keyup はすべて handler から `false` を返して xterm のキーエンコーダへ渡さず、投入は keydown に限定する。これにより custom handler が複数の keyboard event 種別で呼ばれても `\x1b\r` は一度だけ `onData` へ到達し、xterm 由来の追加 `\r` は発生しない。IME 変換中と対象外のキーは `true` を返し、既存エンコードを維持する。この event 順序と入力 API は xterm 6.0.0 の [`CoreBrowserTerminal.ts`](https://github.com/xtermjs/xterm.js/blob/6.0.0/src/browser/CoreBrowserTerminal.ts) と[公開 typings](https://github.com/xtermjs/xterm.js/blob/6.0.0/typings/xterm.d.ts)を根拠とする。

B-001 から B-006 のうち検証手段が自明でないのは、キー入力に対して PTY 入力へ渡る内容である。これは `src/hooks/useTerminal.test.ts` の hook unit test で、送出された `data` と送出回数を観測して検証する。workspace owner と Agent session owner の双方も同じ種別の検証で固定する。実 provider process は起動しない。

### Infra

該当なし。

## Alternatives Considered

- custom key handler から既存 `inputDispatchRef` または backend transport を直接呼ぶ案: PTY へ同じ bytes は送れるが、xterm の公開 user-input 経路と既存 `onData` 入口を迂回し、入力時の xterm 処理と Releash の dispatch 経路を分岐させるため採用しない。
- browser key event を新しい backend command へ渡して Rust で変換する案: custom handler は xterm の同期処理前に受理可否を返す必要があり、DOM event のためだけに新しい同期境界と protocol を追加することになる。Requirements が確定した入力フォーマット以上の domain decision はなく、既存の Terminal Surface 入力契約も変更するため採用しない。

## Cross-cutting concerns

- 可観測性: 既存 dispatch を通すことで terminal input performance probe の `on_data` 計測点も通常入力と同じ位置に維持する。新しい telemetry event は追加しない。

## Risks

該当なし。
