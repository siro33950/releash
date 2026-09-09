# Design 03

## 開始状態

base は `main`、派生点は `a31937204`（#1732 取り込み後）。branch `feat/issues/1733` は派生点と同一 commit のままで、design-01 と design-02 に基づく Impl の成果は未コミットの変更（126 ファイル）として working tree にある。この未コミットの実装をこの周の開始状態とする。直前の Design は `docs/specs/issues-1733/design-02.md`。

design-02 の「変える部分」10 件のうち 8 件（4f3a623a、148af392、85518a37、4a223782、89a86a47、57450817、319cc447、47ef143f）は開始状態で実装済みであり resolve された。この周へ再掲しない。残り 2 件（ed713330、3fa42346）は STILL_OPEN であり、この周の変える部分に含める。

design-02 の「未確定・リスク」2 点のうち、usecase の読み取り表現が NodeExecution の識別を運ばない件は、`artifact_produced` の payload に node_execution_id と attempt を含める形で決着し、この判断は維持する。隔離判定の単一所有と `output get` の読み取り方式の連動は 3fa42346 の未完了として残る。

この周までに解消・見送りとなった Thread は次のとおり。

- 2e726380-bb8e-49b3-8e69-b11ebbd88711（`worktree` を宣言しない Command の起動失敗の復元状態が Paused から Failed に変わることが既存失敗経路維持の対象か）: 現行実装を正として R-002 と B-025 に再起動後の failure 復元を追記し resolve。開始状態の実装（全 NodeFailed を RuntimeFailureObserved に写し Failed を導く）が更新後の要求を満たしており、実装の変更を伴わない。
- 83bd9e99-84ef-458a-b3bd-9517ee2b4d26（新規の WorktreeMode と with_artifact の domain 内 serde 依存）: [DEFERRED] で resolve。定義型と転送形式の境界整理は本 Issue の範囲外の横断課題として別途報告する。
- 9b74a03f-ba5d-4bab-93da-2b17b4b2bab7（今回追加のテスト 5 件の inline module 配置）: [DEFERRED] で resolve。対象 4 ファイルの inline module の `_test.rs` への移設は別途扱う。
- 338704dc、f853493c、e318e97f: [OUT_OF_SCOPE] で resolve 済み。

open Thread は 9 件で、全件に [FIX_POLICY] が投稿済みであり、この周で変える部分の全部である。

## 変える部分

- 空 items の isolated Fanout の保存事実からの完了決着: 空 items に解決された isolated Fanout について、保存事実からの読み取り（`output get` の Artifact 導出、execution 取得 / status の fold）が live と同じく完了として決着し、空の children map に当該 attempt の `worktree` キーを加えた Artifact を返すようにする。開始状態は `expand_fanout_scope` が空座標を live で即時完了とする一方、fact_log は合成子の ArtifactProduced / NodeCompleted を保存せず、`ArtifactQuery::composite_artifact` は子が空だと期待子数 0 の判定に到達する前に None を返し、`replay_node_started` の Fanout 分岐は items を復元するだけで完了に進めない。修正位置（読み側の合成子導出、集約の replay / derive のどちらか、または両方）は Impl が選ぶ。根拠: Thread 34037639-269d-45a1-9916-c75d371e6550（blocking）、R-007「Session / Command / Sequence / Fanout の全種別で同じ形である」、R-010、B-014、B-018。ルート: 委任。次項と一体で実装する。
- 合成子の決着規則の単一所有: 合成子の決着規則（子の失敗処遇 `on_failure: ignore`、Sequence の route 完了、Fanout の期待子数、approval 必須の完了、abort、後続 Artifact による再評価）を、live の集約と保存事実からの Artifact 導出が同じ domain 規則として通るようにし、読み側に独自の合成子状態機械を残さない。開始状態は本差分で新設した `artifact_query.rs` の `composite_artifact` が route 判定だけ `routing::route_in_scope` を共用し、それ以外の規則を集約とは別に持つ。切り出す単位と配置は Impl が選ぶ。根拠: Thread dcc44c17-1911-4c9f-bf8b-1992deb06035、docs/architecture/DOMAIN.md「一つの概念に一つの表現」「状態とライフサイクルを持つ概念は集約が所有する」、B-014 / B-018。ルート: 委任。前項と一体で実装する。
- 隔離判定と実効 worktree 継承の規則の単一所有（design-02 からの継続）: 「省略時 `shared`。`isolated` なら attempt の隔離 worktree、そうでなければ親を継承」の合成規則を domain の一箇所が所有し、runtime の cwd 決定、AgentSession context の読み取り、`output get` の隔離判定がそれを呼ぶ。gateway / usecase は規則の入力（先祖の行、保存定義の復号）の取得に徹する。開始状態で共通化されているのは祖先列から先頭の Some を選ぶ `inherited_worktree_path` だけで、合成規則は `workflow_execution/mod.rs`（runtime）、`worktree_context.rs`（gateway）、`event_draft.rs`（usecase）がそれぞれ持つ。根拠: Thread 3fa42346-619f-4d26-8f10-8972c44382f5（STILL_OPEN、既存 [FIX_POLICY] 有効）、R-002 の継承規則、B-007 / B-008、R-014、DOMAIN.md「一つの概念に一つの表現」。ルート: 委任。以下 3 項と一体で実装する。
- AgentSession の実行 context 読み取りでの root 再読取の除去: 取得済みの root 行（header と復号済みの定義）を隔離 cwd の導出に渡し、root 行の再読取と detail の再 parse を無くす。Session 1 件の context 読み取りで root 行の読み取りと保存定義の復号がそれぞれ 1 回になり、導出される隔離 cwd と workspace_identity の値は変わらない。開始状態は `read_session_context` が root 行の読み取り・header 復号・detail の parse を済ませた後に `worktree_context::execution_worktree_path` を呼び、そこで root 行の再読取・header の再復号・detail の再 parse が行われる。根拠: Thread 315645f8-4da0-4d2f-b82e-3d6c78915473、R-002 / R-014、AGENTS.md「full-retention 設計を避ける」。ルート: 委任。前項と一体で実装する。
- isolated `output get` での識別のためだけの payload 再復号の除去: 提出イベントの一度の復号から Artifact の値・提出情報・所有 NodeExecution の識別（node_execution_id と attempt）を同時に取り、識別のためだけに payload 全体を再 clone・再復号する経路を無くす。返る contract / structured_output（`worktree` キーを含む）/ submitted_at / request_id / timestamp は変わらない。開始状態は `latest_artifact_produced_event` が payload を復号した後、`latest_isolated_artifact_from_drafts` が同じ payload を識別だけの構造体へもう一度復号する。根拠: Thread dbb1f3c0-f6d4-4061-a3b7-12a35ab8ccd8、R-010 / B-018、AGENTS.md「id-based operation で足りる場合に Artifact 全体を clone しない」。ルート: 委任。隔離判定の単一所有と一体で実装する。
- `--session-id` 経由の review の Workspace 解決: session 指定の review（list / get / comment / resolve / history）が、AgentSession の情報が既に持つ Workspace 情報から Thread の対象 worktree を決め、隔離 cwd から root を読み戻すための event store の開き直しと root 行の再読取を行わない。返る Thread 集合は変わらない。開始状態は `review_session_context` が DTO の `worktree_path`（隔離 cwd）だけを見て、隔離 path なら読み取り usecase 一式を構築し root の header まで読み戻す。DTO に root の worktree path を持たせるか、Thread キーの正規化規則を確認して `workspace_identity` を使うかは Impl が判断する。Command からの `RELEASH_WORKTREE_PATH` 経由の逆引き経路は対象外。根拠: Thread 14bdc805-a1c1-450e-8614-09016cc0783e、R-014 / B-024。ルート: 委任。隔離判定の単一所有と一体で実装する。
- isolated 合成子の子開始 commit 後の重複 broadcast の除去: `prepare_isolated_starts` で `finalize_after_commit` が既に行う状態通知と重複する `broadcast_state` を無くし、同じ commit の状態通知を 1 回にする。他の `finalize_after_commit` 呼び出し元 3 箇所と同じ形にする。通知の内容と表示は変わらない。根拠: Thread 5f5e241d-d5ef-471b-bd2b-4293b3d9f86b、AGENTS.md「workflow state 全体を resend しない」。ルート: 委任。
- 台帳削除で状態を失った worktree 分類の協力者の除去: field のない unit struct になった `WorktreeClassificationQuery` について、アプリ起動・CLI / local API・テスト用の組み立てでの生成、`RepositoryStateService`（4 つの constructor）と `RepositoryQueryService` への constructor 引数・field としての注入と保持、同値の `empty` コンストラクタを取り除く。命名規則による除外規則（domain の `matches_isolated_identity_rule`）とその適用（B-010）は維持する。根拠: Thread 2451fe7f-e3cc-4f2c-ae95-40d65bcb7e4e、R-005、design-01「台帳の削除」の削除範囲の完了。ルート: 委任。
- recovery fence 経路の削除の完了（design-02 からの継続）: 常時 None の `resume_unavailable_reason`（domain/workspace_tree の entities / value_objects / services、usecase DTO、gateway query_service、`src/types/workspace-tree.ts`）と、その `is_none()` による resume 可否判定を削除範囲に含める。resume / retry / 表示の既存挙動は変わらない。開始状態は `RecoveryFenceProjected`・`recovery_owner_reason`・owner_reasons の集計は削除済みだが、この field と判定が残る。根拠: Thread ed713330-4309-456e-a05e-dc1949bab60f（STILL_OPEN、既存 [FIX_POLICY] 有効）、R-005「隔離環境喪失の recovery reason も存在しない」、B-011。ルート: 委任。

## 固定するルート

固定する実装上の指定なし。design-01 / design-02 の「固定する実装上の指定なし」を維持する。人間はこの周でいずれの Thread でもルート固定の案を選ばず、対応方法の具体は Impl に委ねた。

## 変えないもの

- design-01 の「変えないもの」（命名規則と配置、実行木の所属、既存の失敗経路、予約環境変数を増やさない、builtin 8 本の定義本文）と design-02 の「変えないもの」（`output get` の対象選択規則、runtime の `recovery_reason` と破損データの `error_reason`、隔離起動手順の runtime host への所在）はそのまま維持する。理由: 人間がこの周でこれらを変更する論点を挙げなかったため、既決の方針と理由を引き継ぐ。
- 「既存の失敗経路」は live の failure 決着経路（`settle_node_failure_for_node` → NodeFailed → Failed、`on_failure` / 手動 Retry）を指す。派生点の事実ログ復元が Command の NodeFailed を Paused に写していた挙動は維持対象に含めず、fact_log の NodeFailed 写像（全種別を RuntimeFailureObserved に写す）は現行のまま変えない。理由: Thread 2e726380 で人間が現行実装を正とし、再起動後の復元を live と用語集「process 起動不能は Node failure」に一致させると決めたため。
- design-02「`output get` の読み取り方式」（実行木の読み取りを一度で済ませ、fold による aggregate 全体の再構築を行わない）は、合成子の決着規則の単一所有、空 items の isolated Fanout の完了決着、識別のためだけの payload 再復号の除去のいずれでも維持する。理由: 人間が dcc44c17 で同じ domain 規則を通す案を選ぶ際、この制約を保つことを条件に含めたため。
- 隔離起動手順の runtime host への所在。重複 broadcast の除去はこの所在に触れない。理由: Thread fbe3365e の見送りを引き継ぐ。
- Command からの `RELEASH_WORKTREE_PATH` 経由の review が隔離 path から root を読み戻す経路。理由: Thread 14bdc805 の対象は `--session-id` 経由だけであり、人間がこの経路を論点に挙げなかったため。
- `artifact_produced` の payload に node_execution_id と attempt を含める表現。理由: design-02 の未確定事項の決着であり、Thread dbb1f3c0 のトリアージで維持と決めたため。

## 未確定・リスク

- 空 items の Fanout の復元と派生点の挙動。非 isolated の空 Fanout が派生点で fold 後に完了として復元されていたか、および `resolve_recovery_dependencies` の本文は未確認である。集約の replay / derive を修正位置に選ぶ場合、その変更は非 isolated の空 Fanout の復元にも及ぶ。派生点でも復元されていなかった場合、Thread 34037639 の受入条件（isolated）を満たす修正が既存の非 isolated 経路の挙動も変えることになり、その扱いは Impl の結果で判明する。
- 合成子の決着規則の切り出しと読み取り方式の両立。集約の決着規則を読み側から呼ぶには、規則の入力（子の状態、`on_failure`、期待子数、approval、abort、後続 Artifact）を一度の読み取りから供給できる必要がある。供給できない入力があると、dcc44c17 の受入条件と design-02 の読み取り方式の制約のどちらかを満たせない。
- Thread キーと `workspace_identity` の同一性。`workspace_identity` は root worktree path を `normalize_repo_path` した値で、Thread のキーに使う生の worktree path との同一性は未確認である。同一でなければ DTO に root の worktree path を持たせる側になり、AgentSession の DTO と frontend 型の変更を伴う。
