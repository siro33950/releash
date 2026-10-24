# Context

要求の正本は [Issue #2071](https://github.com/siro33950/releash/issues/2071) である。
対象は、ネイティブ版で Codex の Session を snapshot から復元した後、Codex が設定した入力方式が失われる問題である。

調査に使用した資料は次のとおり。

- `server/src/infrastructure/terminal/terminal_emulator.rs`
- [avt 0.18.0 の parser.rs](https://github.com/asciinema/avt/blob/v0.18.0/src/parser.rs)
- `clients/macos/Sources/TerminalSession.swift`
- [利用中の SwiftTerm の Terminal.swift](https://github.com/migueldeicaza/SwiftTerm/blob/2dad6a1e516f755bed77cf9bbd09455062f54758/Sources/SwiftTerm/Terminal.swift)
- [利用中の SwiftTerm の MacTerminalView.swift](https://github.com/migueldeicaza/SwiftTerm/blob/2dad6a1e516f755bed77cf9bbd09455062f54758/Sources/SwiftTerm/Mac/MacTerminalView.swift)
- `file:///Users/siro33950/Library/Application%20Support/com.releash.app/terminal-surfaces/*.journal-v2.jsonl`
- [XTerm の入力モード仕様](https://invisible-island.net/xterm/ctlseqs/ctlseqs.html#h2-Mouse-Tracking)
- [kitty keyboard protocol の設定と保存・復帰の仕様](https://sw.kovidgoyal.net/kitty/keyboard-protocol/#progressive-enhancement)

開発開始時点のコードは commit `6301d433c2eb4b55ae298069f88ff80f940ca15f` である。
`Cargo.lock` は avt 0.18.0、`clients/macos/project.yml` と `Package.resolved` は SwiftTerm 1.99.0 を指定している。

サーバが生成する snapshot の replay は、ネイティブ版と Tauri 版が共通して受け取る。
端末状態の所有と処理はサーバに置き、client は受信した状態の表示と入力の受付を担う。
状態所有と保持量の制約は `AGENTS.md`、外部の端末プロトコルを扱う責務は `docs/architecture/INFRASTRUCTURE.md` に従う。

# Outcome

ネイティブ版で Codex の Session を利用する開発者が、snapshot からの復元後も会話をスクロールし、Codex が設定した方式で入力できるようにする。
現在はスクロール操作で会話が動かず、入力欄が送信履歴へ切り替わる。
変更後は、マウス操作、貼り付け、focus 通知、キー入力の扱いを snapshot の復元によって変化させない。

# Current Behavior

Issue に記載された最小の再現手順と結果は次のとおり。

1. ネイティブ版で Codex の Session を開く。
2. マウスホイールまたはトラックパッドで会話をスクロールする。
3. 会話の表示はスクロールせず、Codex の入力欄で送信履歴が切り替わる。

開発開始時の調査では、アプリを起動してこの手順を再実行していない。
以下はコードと保存済み出力の読解で確認した事実である。

- `terminal_emulator.rs:12-40,57-90` は avt だけに端末状態を保持させ、その dump から replay を生成する。
- avt の `parser.rs:1482-1495` は DEC モードを限定して解釈し、マウス関連の `1000 / 1002 / 1003 / 1006 / 1007`、focus 通知の `1004`、bracketed paste の `2004` を保持しない。
  kitty keyboard の設定を扱う分岐もない。
- `TerminalSession.swift:88-98` は snapshot の受信時に画面を初期化してから replay を流す。
  SwiftTerm の `Terminal.swift:1358-1387` では alternate scroll が有効になり、マウス追跡、bracketed paste、通常画面と alternate screen のキーボード設定が初期状態に戻る。
- SwiftTerm の `MacTerminalView.swift:4219-4233` は、マウス報告が無効で alternate screen と alternate scroll が有効な場合、通常のホイール操作を上下キーへ変換する。
- `terminal_emulator.rs:257-317` は保存済み checkpoint と journal を読み込み、端末状態から snapshot を作り直す。
  `terminal_emulator.rs:330-367` は checkpoint を置き換えた後に元の journal を削除する。
  checkpoint に含まれず、journal からも失われた入力設定は保存データから読み取れない。

実際の Codex 出力は、`/Users/siro33950/Library/Application Support/com.releash.app/terminal-surfaces/a69fdd7b039aec923ce964647bcdf9924dccc7b0d7edf8de288c08cebe222028.journal-v2.jsonl` で確認した。
調査時点の 1 行目（sequence 1）と 2 行目（sequence 2）には、次の制御出力が含まれていた。
以下では ESC を可視表記している。

```text
sequence 1: ESC[?2004h ESC[>7u ESC[?1004h
sequence 2: ESC[?1049h ESC[>7u ESC[?1007l ESC[?1000h ESC[?1002h ESC[?1003h ESC[?1006h
```

マウス以外の設定が失われた場合の実機上の症状、出力を最初から逐次受信した場合の操作結果、Tauri 版での症状は未確認である。

# Scope / Non-goals

変更する対象:

- ネイティブ版の Codex の Session で、snapshot から復元する入力方式。
- マウス追跡方式、座標の送信形式、alternate scroll、bracketed paste、focus 通知、kitty keyboard の設定の保持と復元。
- 復元後の設定変更、通常画面と alternate screen の切り替え、保存したキーボード設定への復帰を含む入力方式の継続。
- 上記の入力方式を再現するために必要なサーバの snapshot の生成、保存、復元。
- 本変更後に受信する入力設定と、復元に必要な情報が残る既存の保存状態。

変更しない対象:

- Codex 自体の会話表示、送信履歴、入力プロトコルの仕様。
- 本件の入力方式の復元と関係しない端末機能や画面操作。
- 本変更前の checkpoint に入力設定が記録されず、元の journal からも情報が失われた場合に、その過去の設定を復元すること。

# Requirements

R-001 から R-005 は、本変更後に受信する入力設定と、復元に必要な情報が残る既存の保存状態を対象とする。

- R-001: ネイティブ版で Codex の Session を snapshot から復元した後も、Codex が設定したマウス追跡方式、座標の送信形式、alternate scroll の有効・無効を維持する。
  Codex が会話のスクロールを受け付ける状態では、マウスホイールとトラックパッドで会話をスクロールでき、その操作によって入力欄が送信履歴へ置き換わらない。
- R-002: snapshot の復元後も、Codex が設定した bracketed paste の有効・無効を維持し、貼り付けた内容を復元を挟まない場合と同じ方式で Codex に渡す。
- R-003: snapshot の復元後も、Codex が設定した focus 通知の有効・無効を維持し、利用者がフォーカスを移した際の通知を復元を挟まない場合と同じにする。
- R-004: snapshot の復元後も、Codex が設定した kitty keyboard の入力方式を維持し、同じキー操作を復元を挟まない場合と同じ方式で Codex に渡す。
  通常画面と alternate screen の切り替えや、保存した設定への復帰の後も、それぞれの画面に対して設定された入力方式を維持する。
- R-005: R-001 から R-004 の入力方式は、snapshot の復元後に Codex が設定を変更・解除した場合も、その出力を継続して受け取った場合と同じ状態になる。
  保存された端末状態から snapshot を作り直した場合にも、この性質を維持する。

# Assumptions

なし。
