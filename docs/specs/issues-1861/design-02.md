# Design 02

## 開始状態

- 差分の基準は base ブランチ `main`、派生点は `feat/issues/1861` の `81ec380b`。Design 01 の周の実装は未コミットの作業ツリーにあり、Spec 工程ではコードを変更していないため、この実装をこの周の開始状態とする。
- 直前の Design は `docs/specs/issues-1861/design-01.md`。
- この周までに解消・見送りとなった Thread はない。open Thread は7件で、7件とも `[FIX_POLICY]` が付き、`[REJECTED]`・`[DEFERRED]` はない。
- Requirements・Behavior はこの周で変更しない。R-001〜R-015 と B-001〜B-021 の対応、および Assumptions / Open Questions が「なし」であることを再確認した。

## 変える部分

- 走査結果の commit 先の統一: 受理した走査結果を `RepositoryStateService` の `WorktreeState` へ反映し、Workspaces 一覧と `get_snapshot` 経由の既存消費者（status、diff stats、branch 一覧、review usecase）が同じ backend-owned state から観測できるようにする。開始状態では `rescan_branches` が `WorktreeState` を迂回して branch 一覧だけを返すため、同一 Repository の最新状態が Workspaces 一覧と既存消費者で分岐する。根拠: Thread 251fbd1f-8ded-4e7f-beae-a62401da4a6b（blocking）、R-006、B-007。ルート: 委任
- 局所操作後の再読込の対象範囲の是正: 単一 Worktree に閉じた操作（Session の作成・削除・復元、Workflow 操作、Worktree メニュー・作成メニューの展開）の後の再読込が、対象外の Repository の走査を発生させないようにする。開始状態では `useWorkspaceTreeNodes` の reconciliation でない refresh が Workspaces 全体の更新へ入り、全登録 Repository の走査を起こす。根拠: Thread 02db6e27-be50-4952-95a4-fedc8e66d7ba（blocking）、R-005、`AGENTS.md` レビュー観点「full-retention / full-recompute 経路を増やしていないか」。ルート: 委任
- 更新の状態機械の所有者の統一: generation、取得成否、前回値の保持と解放、削除の反映を一つの状態所有者の操作として表現し、usecase は何をどの順で呼ぶかだけを持つようにする。開始状態では `WorkspaceListRefresh` と、usecase 側の `Retained`・階層別 map・`generation`・`snapshot_generation` に同じ lifecycle が分かれている。根拠: Thread 7a56b6c5-6711-4a6a-97a6-22ae486ba6ee、`docs/architecture/USECASE.md`「usecase は状態機械を持たない」、`docs/architecture/DOMAIN.md`「一つの概念に一つの表現」。ルート: 委任
- 取得結果の新旧判定の frontend からの除去: 取得結果を一覧へ適用するかの新旧判定を Rust 側だけに置き、frontend は受け取った一覧・更新状態・エラーを表示する。開始状態では `useWorkspaceList` が `result.generation` と `generationRef` を比較して結果を破棄しており、同じ判断が backend と frontend に分かれている。根拠: Thread 949c3145-8c6b-4083-be09-f5484d7c9521、R-013、B-017、`AGENTS.md`「全てのアプリケーションロジックは Rust に置く。例外なし」、Design 01 の固定ルート①。ルート: 委任
- branch read model の合成と対象選別の usecase テスト追加: open PR を含む `PrStatus` と `worktree_path` を持たない branch を含む入力で、`has_pr`・`pr_number`・`pr_url`、PR 情報による `is_merged`、`worktree_path` を持たない branch が子一覧取得の対象から外れることを入出力で確認できるようにする。開始状態では usecase テストの branch helper が常に `worktree_path: Some`、`pr_status` が常に `PrStatus::default()` である。根拠: Thread 90908bb4-3590-4503-a6c6-be1eeb7119ee、`docs/architecture/TEST.md`「`usecase/` テスト **必須**」。ルート: 委任
- 複数 Worktree の途中失効の usecase テスト追加: 同一 Repository に複数の Worktree を持つ入力で、先行更新が一部の Worktree を処理した時点で後続更新が開始された場合に、先行結果が残りの Worktree へ適用されず、最終 snapshot が後続更新の一覧だけになることを確認できるようにする。開始状態では usecase テストの fixture が各 Repository に Worktree を1件しか持たず、Worktree ループ2周目以降の早期 return を通らない。根拠: Thread 29db26a4-75e4-46e9-b090-d69dc60b856f、R-013、B-017、`docs/architecture/TEST.md`「`usecase/` テスト **必須**」。ルート: 委任
- 階層別の取得状態表示の component テスト追加: Repository 階層と Worktree 階層のそれぞれについて、loaded=false かつ error なしで進行表示、loaded=false かつ error ありで初回失敗表示となり、いずれの場合も「No worktrees」「No sessions or workflows」を表示しないことを判別できるようにする。開始状態の component テストが構築する loaded=false は Workspaces 全体の status だけで、Repository・Worktree の status は既定の loaded=true のままである。根拠: Thread b8d8e38e-e02f-4df3-a556-a1ef06617da7、R-010、B-012、B-013、B-014、`AGENTS.md` テスト方針「component は user interaction と conditional rendering をテストする」。ルート: 委任

## 固定するルート

この周で新しく固定する実装上の指定はない。Design 01 で固定した次の4つを維持する。

- 全体の更新手順と結果の判定を Rust 側に置き、既存の `RepositoryStateService` の走査処理と Workspace の一覧取得処理を再利用する。Frontend は受け取った一覧・更新状態・エラーを表示するだけにする。（Design 01 固定ルート①）
- 一覧を保持する変更と、Session・Workflow の再取得経路の変更を併せて行う。（Design 01 固定ルート②）
- 更新対象は Workspaces の一覧情報に限り、表示中の Session や実行中の Workflow の継続を保つ。（Design 01 固定ルート③）
- 手動更新と自動更新で内部の経路を分けない。（Design 01 固定ルート④）

## 変えないもの

- R-006 の「保存済みの結果をそのまま反映しない」。走査結果の commit 先を変えても、更新が保存済みの結果をそのまま返す形へ戻さない。理由: 走査結果を `WorktreeState` へ反映する修正が、`get_snapshot` の保存済み snapshot を返す経路への回帰になりうるため。
- Workspaces 行の手動更新と自動更新の対象範囲。局所操作後の再読込の範囲を是正しても、手動更新と自動更新は R-005 の3階層を対象とし続け、B-006・B-021 を満たす。理由: 対象範囲を狭める修正が全体更新側にも及ぶことを避けるため。
- R-008・R-010・R-013・R-014 に対応する外部から観測できる結果（B-009、B-011、B-012、B-013、B-014、B-017、B-018）。状態機械の所有者と新旧判定の置き場所を変えても、これらの観測結果は変えない。理由: 該当する Thread の指摘がいずれも内部の所有と配置に閉じるため。

## 未確定・リスク

- 走査結果の commit と既存の通知経路の関係が未確定である。`WorktreeState::notify_snapshot_changed` は `BranchListSync` push を発行し、`useWorkspaceList` は `branch-list-sync` を受けて更新を再実行する。commit に既存の通知をそのまま伴わせると更新が自身の再実行を呼び、R-012 の「更新が成功・失敗のいずれで終わった後も、再び操作できる」状態へ落ち着かない。Thread 251fbd1f の受入条件は同じ backend-owned state から観測できることまでで、通知の要否を定めていない。
- 局所操作後の再読込を Workspaces 全体の更新から切り離す場合、Worktree 配下の Session・Workflow 一覧の取得経路が二つになる。取得結果の新旧判定は Rust 側だけが持つ形へ変えるため、切り離した経路が同じ判定の管理下に入らないと、R-013・B-017 の「より古い取得結果でより新しい一覧を上書きしない」を保てない。
