# Context

要求の正本は [Issue #2065](https://github.com/siro33950/releash/issues/2065) である。
補助資料は次のとおり。

- [Issue #1204](https://github.com/siro33950/releash/issues/1204)、`docs/specs/issues-1204/requirements.md` の R-016、`docs/specs/issues-1204/behavior.md` の B-017。
- [Issue #1734](https://github.com/siro33950/releash/issues/1734)、`docs/specs/issues-1734/requirements.md` の R-012、`docs/specs/issues-1734/behavior.md` の B-017。
- `docs/glossary/DOMAIN.md` の「実行木」「状態所有」。
- `server/src/domain/workspace_tree/card.rs`、`server/src/domain/workspace_tree/visible.rs`、`server/src/domain/workspace_tree/entities/mod.rs`。
- `server/src/domain/workflow/entities/workflow_execution/mod.rs`。
- `server/src/adaptor/presenter/client/workspace.rs`、`clients/macos/Sources/SidebarView.swift`。
- `server/src/domain/workspace_tree/card_test.rs`、`server/src/domain/workspace_tree/projection_test.rs`、`server/src/adaptor/presenter/client/workspace_test.rs`。
- `AGENTS.md`、`docs/architecture/DOMAIN.md`、`docs/architecture/PRESENTER.md`。

#1204 の R-016 は、workflow 行の N を起動済みの Session と Command の数、四角を Session ごとの状態表示とし、Sequence、Fanout、過去の試行を集計から除く。
#1734 の R-012 は、delegate の child を発火ごとの行として実行木に残す。
`docs/specs/issues-1734/requirements.md:68` の最大 attempt による分類は、同文書の Current Behavior に記録された変更前の状態である。

本変更では、一覧と実行木の対象判定を一つの規則に統一する。
集計から除く過去の試行は retry によって置き換えられた履歴とし、delegate の発火や通常のループ再訪を含める。
判定と集計はサーバが所有し、client は受け取った値を表示する。

# Outcome

対象は、Workspaces 一覧から workflow の実行状況を把握する利用者である。
現在は、delegate を繰り返すと過去の発火分が一覧の Node 数と四角から欠落し、実行木に残る処理の量と状態を一覧から把握できない。
変更後は、一覧と実行木が同じ対象を示し、delegate の発火や通常のループ再訪を含めて Node 数と Session の状態を把握できる。

# Current Behavior

調査開始時点の HEAD は `6301d433c2eb4b55ae298069f88ff80f940ca15f` である。
以下の実装とテストを読んで確認した。
アプリの操作、test、lint、build は実行していない。

- 一覧の集計は、実行、Node 名、親参照ごとに最大 attempt を求め、retry 履歴でないことと最大 attempt であることの両方を要求する。最大 attempt でない Sequence や Fanout の子孫はたどらない（`server/src/domain/workspace_tree/card.rs:62-118`）。
- delegate の child は、同じ親の下の最大 attempt に 1 を足して起動される。このため、繰り返し発火した child は一覧の最大 attempt 判定で過去分が除かれる（`server/src/domain/workflow/entities/workflow_execution/mod.rs:893-931`）。
- 実行木は retry の前任関係から履歴を判定し、通常の親子表示からその履歴を除く。最大 attempt による絞り込みは行わない（`server/src/domain/workspace_tree/entities/mod.rs:559-599`、`server/src/domain/workspace_tree/visible.rs:33-42`）。
- retry の履歴に属する child は、その履歴の配下に表示される。一覧の走査は、履歴となった Session 自身を除いてもその子孫をたどるため、この場合も実行木の通常の親子表示と集計対象が異なる（`server/src/adaptor/presenter/client/workspace_test.rs:562-625`、`server/src/domain/workspace_tree/card.rs:113-117`）。
- 一覧の既存テストは、通常のループ再訪の過去分を除くことを期待する。一方、実行木の投影テストは、通常のループ再訪を retry 履歴にしないことを期待する（`server/src/domain/workspace_tree/card_test.rs:373-461`、`server/src/domain/workspace_tree/projection_test.rs:599-650`）。
- サーバは一覧用の Node 数と Session の状態列を配信し、Swift の一覧はその値を表示する。Swift 側で集計対象を決める処理はない（`server/src/adaptor/presenter/client/workspace.rs:120-150`、`clients/macos/Sources/SidebarView.swift:128-154`）。

問題を起こす最小の操作は次のとおり。

1. Session が同じ delegate child を複数回起動する workflow を実行する。
2. retry を行わずに child の次の発火まで進める。
3. Workspaces 一覧の Node 数と四角を、同じ workflow の実行木と比較する。

現行コードでは、実行木に残る過去の発火分が一覧の集計から除かれる。
child が Sequence や Fanout なら、その過去の発火に属する Session と Command も一覧から除かれる。
Issue #2065 は、development-v2 の実行木 `a4d112fa-d61a-4281-b2cd-c7efe87f6bad` で、起動済みの Session と Command が 79 個あるのに、一覧が `7 nodes` と四角 5 個を表示したと報告している。
この実測値は Issue の報告であり、本調査では当該実行データを読み出して再確認していない。

# Scope / Non-goals

Scope:

- Workspaces 一覧の workflow 行で配信・表示する Node 数と Session の状態列。
- 一覧と実行木の対象判定の統一。delegate の各発火、通常のループ再訪、それらの部分木、および retry 履歴に属する部分木を含む。

Non-goals:

- delegate の発火、通常のループ、retry、resume の実行規則や attempt の採番の変更。
- workflow 行のレイアウト、状態の分類や配色の変更。
- 過去の実行データの書き換え。

# Requirements

- R-001: Workspaces 一覧の workflow 行で配信・表示する N は、同じ workflow の実行木から retry の履歴とその配下の部分木を除いた範囲にある、起動済みの Session と Command の数に一致する。delegate の各発火と通常のループ再訪は、同じ名前でも別々の実行として、その配下を含めて数える。Sequence と Fanout 自身は数えない。
- R-002: workflow 行で配信する Session の状態列には、R-001 の集計対象に含まれる Session 一つにつき一つの状態が含まれる。画面は各状態を、従来の 3 値の色の四角一つで表す。Command、Sequence、Fanout、および R-001 の集計対象から除かれた Session の四角は表示しない。

# Assumptions

なし。
