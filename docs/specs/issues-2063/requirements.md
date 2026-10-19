# Context

- 要求の正本: https://github.com/siro33950/releash/issues/2063
- 補助資料: https://github.com/siro33950/releash/issues/1205 、`docs/specs/issues-1205/behavior.md`（B-009、B-015）、https://github.com/siro33950/releash/issues/1958
- 対象はネイティブ版（`clients/macos`）の Session タブである。
- タブが同定する Session は AgentSession である。実行木の Node と AgentSession の区別は `docs/glossary/DOMAIN.md` に従う。
- `AGENTS.md` に従い、状態と操作可否の判断はサーバが所有し、client は購読した値の表示と操作の呼び出しを行う。
- 状態の色は #1958 の黄（人の番）、青（システムの番）、緑（誰の番でもない）である。

# Outcome

ネイティブ版で Session を操作する利用者が、開いた経路によって異なるタブや名前を扱い、Rename と Archive のために一覧の行を探している。
同じ会話は同じ Session タブで扱い、タブ列から名前と状態を把握し、そのタブから Rename と Archive を実行できる状態にする。

# Current Behavior

以下は開発開始時のコードの確認結果であり、実機での再現と実際の画面出力は未確認である。

- New Session は `selection.nodeID`、History も同じフィールドをタブに渡す。一覧は `rootID` を渡す。応答には AgentSession の ID と Node の ID が別々に存在する（`clients/macos/Sources/AppModel.swift:617-643`、`clients/macos/Sources/SessionHistoryView.swift:70-78`、`server/src/adaptor/presenter/client/conversions.rs:2114-2121`）。
- タブの重複判定は渡された Node ID の一致だけで行う。一覧の公開 ID は実行を代表する節で実行の ID に置き換わる（`clients/macos/Sources/PaneWorkspace.swift:105-117`、`server/src/domain/workspace_tree/visible.rs:169-173`）。再現候補は New Session で開いた後、同じ Session を一覧から開く操作であり、コード上は異なる ID に対して別タブを作成する。
- タブ名の同期は Root 一覧の ID とタブの Node ID を突き合わせる。New Session と History の初期名は「Session」で、保存したレイアウトの復元も同じ初期名になる（`clients/macos/Sources/AppModel.swift:610-628`、`clients/macos/Sources/PaneWorkspace.swift:44-48`、`clients/macos/Sources/PaneLayout.swift:10-25`）。再現候補は New Session でタブを開き、その Session を一覧から改名する操作であり、ID が一致しないタブのタイトルは更新されない。
- タブ本体は worktree と Node ID で詳細を購読した後、AgentSession の ID で会話の状態を購読する。タブ名はこの購読から更新していない（`clients/macos/Sources/NodePane.swift:11-39,68-75`）。
- Session タブのアイコンは `sparkle` で、タブの作成と更新には状態の色を渡していない（`clients/macos/Sources/PaneLayout.swift:13-18`、`clients/macos/Sources/PaneWorkspace.swift:105-124`）。
- Rename と Archive の入口は一覧の Root の右クリックメニューにある。Archive は中止可能な実行について確認を出し、了承すると実行木を Archive する。Workflow 内の子の行にはこのメニューを出さない（`clients/macos/Sources/RootListView.swift:119-147,197-218,255-265`、`server/src/usecase/workflow/workspace_tree.rs:206-214`）。
- Session タブは、サーバが判定不能または Archive 済みの状態を配ると、TUI の代わりにその状態を表示する（`clients/macos/Sources/NodePane.swift:56-65`）。

# Scope / Non-goals

Scope:
- Session タブの同定、開く経路、保存と復元で使う識別子。
- Session タブに必要な情報の購読と、タブ名および状態色への反映。
- タブ部分からの Rename と、単独 Session の Archive。
- 上記に必要なサーバの配信と操作、および Bonsplit の拡張と利用側への反映。

Non-goals:
- Workflow 内の Session タブからの Archive の許可。
- Archive の対象単位の変更。
- 状態に応じた SF Symbol の差し替え。
- Command、Terminal、ファイルの各タブの機能変更。

# Requirements

- R-001: 会話が紐付いた Session タブは AgentSession の ID だけで対象を同定する。一覧、New Session、History のどの経路から同じ Session を開いても、開いている同じタブを選択する。保存と復元でも同じ Session を対象とする。
- R-002: 会話が紐付いた Session タブに必要な情報は、その AgentSession の ID だけを指定して購読できる。タブの表示と操作可否を得るために Root 一覧との突き合わせを必要としない。
- R-003: Session タブは購読した Session 名をタブ名に反映し、改名後も追従する。New Session、History、一覧、保存したレイアウトからの復元で、この結果は変わらない。
- R-004: Session タブは、一覧と同じ状態の分類をアイコンの黄・青・緑で表示し、状態の変化に追従する。黄は人の番、青はシステムの番、緑は誰の番でもないことを示す。
- R-005: 利用者は Session タブのタブ部分から Rename を実行でき、変更した Session 名がタブと一覧に反映される。
- R-006: 利用者は単独 Session のタブ部分から Archive を実行できる。中止可能な実行は中止して Archive することを確認し、了承した場合に実行する。取りやめた場合は実行しない。中止可能でない場合は確認なしで Archive する。Workflow 内の Session タブからは Archive できない。
- R-007: Session を開けるか判定できない場合と Archive 済みの場合は、既存どおり TUI の代わりにその状態を表示する。
- R-008: 保存したレイアウトの復元で一部の Session の情報取得に失敗しても、ほかのタブと配置は復元する。取得に失敗したタブの参照を保持し、失敗したことを利用者に表示する。
- R-009: 一覧の Session に会話がまだ紐付いていなくても、利用者が開く操作をしたらタブを開く。準備中はその状態を表示し、起動に失敗した場合は理由を表示する。会話が利用可能になったら、同じタブで TUI を表示する。

- R-010: Session タブに会話を開く処理の失敗を表示している場合、その処理が成功するか、購読で会話が利用可能になったことを確認したら、以前の起動エラーを消す。
- R-011: 同じ workspace を表示している間、利用者が閉じたレイアウト復元時の取得失敗は、同じ失敗が購読で継続して届いても再表示しない。新しい失敗と、一度解消して再発した失敗は表示する。

# Assumptions

なし
