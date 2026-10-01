# Context

- 正本: [#1978 `[13] terminal だけの扱いが、汎用の購読の処理に入り込んでいる`](https://github.com/siro33950/releash/issues/1978)
- 調査基準は branch `feat/issues/1978` の `506f9a98`。
- 補助資料: [#1888](https://github.com/siro33950/releash/issues/1888)（terminal の出力を購読に移した ISSUE。`docs/specs/issues-1888/`）、[#1956](https://github.com/siro33950/releash/issues/1956)（購読の読み取りの失敗を画面へ届けた ISSUE。`docs/specs/issues-1956/`）、[#1979](https://github.com/siro33950/releash/issues/1979)（usecase と controller の繰り返し処理のループとタイマー）。
- 本 ISSUE は #1888 と #1956 に依存する。#1888 が定めた terminal の購読の要求（`docs/specs/issues-1888/requirements.md` の R-001〜R-023）と、#1878 が定めた購読の土台の要求は、有効な既存条件である。特に、変更の届け方を対象ごとに丸ごとか差分かで選べること（#1878 R-005）と、差分で進む対象の購読ごとの送り待ちの上限を量で持つこと（#1888 の Scope）は、terminal に限らない購読の土台の規則である。
- 層の規約の正本は `docs/architecture/`。presenter は変換だけを持ち（`PRESENTER.md`）、controller は外からのきっかけを Usecase の引数に変えて Usecase を呼び（`CONTROLLER.md`）、infrastructure は内側の語彙を持たない（`INFRASTRUCTURE.md`）。部品の一覧に無い部品は置かない（`README.md`「部品の一覧」）。
- #1979 は本 ISSUE に依存し、同じ `src-tauri/src/usecase/state_subscription.rs` を変える。購読の worker の定期の取り直しのループと、terminal の最初の状態の作り直しのループの駆動の作り直しは #1979 が扱う。

# Outcome

対象者は、UI と daemon の間の購読の仕組みを実装・保守する開発者である。

現在、terminal の購読だけの扱いが、全ての購読が通る汎用の処理（購読の Usecase、その配信の口、汎用の presenter、配信の土台）の中に、terminal 専用の状態、手続き、引数、分岐として入り込んでいる。購読の仕組みを変えるたびに terminal の分岐も一緒に変える必要があり、汎用の処理を変えると terminal の出力の届き方まで変わる。

変更後は、汎用の購読の処理は対象が terminal かどうかを区別しない。terminal の購読の扱いは terminal の側に置かれ、汎用の処理と terminal の扱いは互いに独立して変えられる。利用者から見た terminal と他の購読対象の振る舞いは変わらない。

# Current Behavior

調査基準 `506f9a98` のコードで確認した挙動である。パスは `src-tauri/src/` からの相対。

## 汎用の購読の Usecase に terminal 専用の状態と手続きがある

- 汎用の購読の Usecase が、terminal 専用の状態 `terminal_inputs`・`terminal_resets`・`terminal` を持つ（`usecase/state_subscription.rs:92-97`）。
- 購読の開始で、入力の識別子が付いているか対象が terminal なら、terminal 専用の経路に分かれる（`usecase/state_subscription.rs:111-131`）。通常の開始は全体のロック `starts` で直列にするが（`:201-202`）、terminal の開始はこのロックを取らない（`usecase/state_subscription/terminal.rs:15-50`）。
- 停止と切断の後始末が、毎回 terminal 用の後始末を呼ぶ（`usecase/state_subscription.rs:442,467`。本体は `usecase/state_subscription/terminal.rs:114-125`）。
- terminal の最初の状態の作り直しが、汎用の Usecase にある（`usecase/state_subscription.rs:489-527`、`usecase/state_subscription/terminal.rs:71-112`）。作り直しの task は、汎用の購読の worker と同じ表 `workers` に入り、購読が無くなった対象の task は汎用の後始末（`usecase/state_subscription.rs:367-376`）が止める。
- 処理済みの量の報告が、汎用の Usecase にある（`usecase/state_subscription/terminal.rs:127-151`）。
- 入力の識別子の検査（空白だけでないこと、128 バイト以下であること）が、汎用の Usecase の terminal の部分にある（`usecase/state_subscription/terminal.rs:5,22-27`）。

## 汎用の配信の口に terminal 専用の引数と操作がある

- 配信の口 `StateSubscriptionOutput` の `start` が引数 `terminal_input_id` を持ち、terminal 専用の操作 `set_terminal_snapshot` を持つ（`usecase/state_subscription.rs:22-61`）。

## 汎用の presenter に terminal 専用の扱いがある

- 汎用の presenter が terminal の Usecase を保持する（`adaptor/presenter/state_subscription.rs:50-68`）。
- 開始の手順（terminal の世代を読み、出力の順番の lock に入り、世代を照合し、購読を登録し、送り待ちの量を読み、流量制御に購読者を登録し、世代が消えていればやり直す）が presenter にある（`adaptor/presenter/state_subscription.rs:193-243`）。
- 失敗の配信で、terminal の対象だけ差分として送る分岐がある（`adaptor/presenter/state_subscription.rs:344-352`）。
- 送り待ちが溢れたときに、汎用の stream から terminal の作り直しを呼ぶ（`adaptor/presenter/state_subscription.rs:163-170`）。
- stream を開くときと終わるときに、presenter が購読の Usecase を呼ぶ（`adaptor/presenter/state_subscription.rs:145-171,257-281`）。
- terminal の出力を受け取る `TerminalSurfaceStateSink` の実装が、汎用の presenter にある。その中に、出力の番号を今の版と比べて、最初の状態を取り直させる・重ね送りを捨てる・差分として積む、を決める判断がある（`adaptor/presenter/state_subscription.rs:419-545`）。

## 配信の土台に terminal 専用の経路がある

- 配信の土台が、terminal の session から対象への経路 `terminal_routes` と、daemon の起動ごとの terminal の epoch `terminal_boot` を持つ（`infrastructure/state_subscription.rs:16-94`）。使うのは汎用の presenter の terminal の部分だけである。
- 送り待ちの上限 `pending_limit` は汎用の仕組みにあるが、設定するのは terminal だけである（`infrastructure/state_subscription.rs:470-500`、`infrastructure/terminal/output_flow_control.rs:11`）。

## 呼び出し元

- controller は、購読の開始で入力の識別子をそのまま汎用の Usecase に渡し（`adaptor/controller/api/client_service.rs:34-55`）、処理済みの量の報告で汎用の Usecase を呼ぶ（同 `:76-99`）。stream は presenter に汎用の Usecase を渡して開く（同 `:12-31`）。
- daemon の組み立てで、汎用の presenter と汎用の Usecase に terminal の Usecase をつなぐ（`adaptor/controller/daemon.rs:115-116`）。

# Scope / Non-goals

## 変更する対象

- 汎用の購読の Usecase、その配信の口、汎用の presenter、配信の土台にある terminal の扱い（`usecase/state_subscription.rs`・`usecase/state_subscription/terminal.rs`・`adaptor/presenter/state_subscription.rs`・`infrastructure/state_subscription.rs`）。
- terminal の扱いを移した先の部品と、その呼び出し元（controller、daemon の組み立て、テスト用の組み立て）。
- terminal の出力の中継の登録表（`adaptor/presenter/terminal_event_hub.rs:21` の `registrations`）と、terminal の session から対象への経路（`infrastructure/state_subscription.rs:20` の `terminal_routes`）を一つにまとめること。
- この変更で使われなくなるコードの削除。

## 変更しない対象

- terminal の出力の流量制御（処理済みの量の報告）の規則と値。#1888 で決まっている。
- 購読の worker の定期の取り直しのループと、terminal の最初の状態の作り直しのループの駆動。#1979 が扱う。
- 購読の失敗の種類の構成。terminal の失敗（入力の識別子が不正、購読が無い）は、購読の失敗の列挙に残す。
- 入力の識別子の規則を domain の型にすること。
- 購読中の対象の集合（最初の状態を捨てない対象を決める集合）の作り方。汎用の購読の Usecase の集合だけから作る今の形のまま。

# Requirements

- R-001: 汎用の購読の処理（購読の Usecase、その配信の口、汎用の presenter、配信の土台）は、対象が terminal かどうかで分岐せず、terminal 専用の状態、手続き、引数を持たない。
- R-002: terminal の購読の外から見える振る舞い（届く値、失敗、送る量の制御）は変わらない。
- R-003: terminal 以外の購読対象の外から見える振る舞い（届く値、失敗）は変わらない。

# Assumptions

なし。
