# Design 04

## 開始状態

base は `main`、派生点は `a31937204`（#1732 取り込み後）。branch `feat/issues/1733` は派生点と同一 commit のままで、design-01〜03 に基づく Impl の成果は未コミットの変更（138 ファイル）として working tree にある。この未コミットの実装をこの周の開始状態とする。直前の Design は `docs/specs/issues-1733/design-03.md`。

design-03 の「変える部分」9 件（34037639、dcc44c17、3fa42346、315645f8、dbb1f3c0、14bdc805、5f5e241d、2451fe7f、ed713330）はすべて開始状態で実装済みであり resolve された。この周へ再掲しない。

design-03 の「未確定・リスク」3 点は次のとおり決着した。空 items の Fanout の復元は、集約の replay が worktree を持つ空 Fanout だけを pending にし、事実の適用前に完了を導出する形を取り、非 isolated の空 Fanout の復元経路には及んでいない。合成子の決着規則と読み取り方式の両立は、保存事実からの Artifact 導出が対象 scope を復元して集約と同じ適用規則を通す形で成立した。Thread キーと `workspace_identity` の同一性は、AgentSession の DTO に root の worktree path（`workspace_worktree_path`）を持たせる側で決着した。

この周までに解消・見送りとなった Thread は次のとおり。

- 85e77970-8c96-4aa4-af87-efafb9994559（Artifact 産出後の UI で Artifact の `worktree` キーを確認できない）: 派生点の UI に Artifact の値を表示する経路が無いことを確認し、R-010 と B-018 / B-023 を「UI での Artifact の `worktree` の参照は同じ値を示す Node の branch / path の表示が担い、Artifact 値の表示経路を新設しない」に改めて resolve。実装の変更を伴わない。
- 49f6d16c-3f5f-46b5-af56-e4d1e391de35（`WorkflowEvent` 型名の禁止語）、15c37982-ebb5-463f-a1b9-1146a466bfc0（frontend の PR 状態結合）、8c2ad343-7174-4092-85ad-7cd0841a78c8（review helper の重複）: いずれも派生点に既に存在し本差分が導入したものでないため [OUT_OF_SCOPE] で resolve。
- 83bd9e99、9b74a03f、fbe3365e、338704dc、f853493c、e318e97f、2e726380: design-03 までに決着済み。

open Thread は 10 件で、全件に [FIX_POLICY] が投稿済みであり、この周で変える部分の全部である。不成立と見送りは 0 件。

## 変える部分

- 合成子の branch / path の UI での参照: isolated な Sequence / Fanout の attempt について、UI でその合成子を参照するとその合成子自身の attempt の隔離 branch と path を読めるようにする。items が空で children を持たない Fanout でも、完了・失敗・abort 後でも同じ値を読め、Artifact 産出後に読める値はその Artifact の `worktree.branch` / `worktree.path` と同じにする。開始状態は、workspace tree の合成子 DTO（`WorkspaceSequenceDto` / `WorkspaceFanoutDto`）に worktree の表示値が無く、Node 詳細の `worktree` は Session / Command だけが持ち、UI の合成行は展開のみで選択できない。表示位置（合成行への表示か、合成子の詳細表示の新設か）と、そのための usecase DTO / frontend 型の変更は Impl が決める。Artifact の値を UI に表示する経路は新設しない。根拠: Thread 69cbadab-9549-4a9b-88a9-8e32330b8b6c、R-010「合成子（Sequence / Fanout）の attempt についても、children の有無に関わらず、その合成子自身の branch / path を参照できる」、B-027、B-023。ルート: 委任。
- 空 items の isolated Fanout での保存済み abort の保持: 空 items の isolated Fanout について、approval なしの Started → AbortRequested の事実列を fold / `output get` で読むと abort が反映され、実行木が Completed にならず Artifact が返らないようにする。Started → AbortRequested → RuntimeFailureObserved の事実列でも完了に置換しない。Started だけの事実列は引き続き完了と worktree 成果を復元する。開始状態は `apply_record` が各事実の適用前に `derive_empty_isolated_fanouts` を呼び、抑止が同 Node の RuntimeFailureObserved に限られるため、AbortRequested の直前に完了が導出され、その後の abort が終端への適用として受理されない。隔離 worktree の生成は activation gate を解放した後に失敗を決着するため、この事実列は実際に生成できる。根拠: Thread 7b8d1cef-fbbf-40c7-bd48-f7bf4429efa4（blocking）、R-010「Artifact を産出せずに失敗または abort で終わった後のいずれでも参照できる」、B-023、R-002「これらの failure は Releash を再起動した後の復元でも failure のまま」、design-03「空 items の isolated Fanout の保存事実からの完了決着」の維持。ルート: 委任。
- 実効 cwd 読み取りでの一時障害の分類: Session の実効 cwd の導出で reader の QueryBusy / DeadlineExceeded / StorageUnavailable が起きたとき、AgentSession の読み取りを Unavailable として返し Corrupt にしない。祖先行の欠落、不正な祖先関係、定義の復号失敗は引き続き Corrupt とし、両方の区別をテストで固定する。開始状態は `worktree_context::execution_worktree_path` が `run_indexed` のエラーを String に潰し、`session_facts.rs` がその全てを `SessionContextReadError::Corrupt` に写す。root 行の既存の読み取りは同ファイル内で一時障害を Unavailable に写しており、追加した cwd 読み取りだけがこの契約から外れている。根拠: Thread 6e17da26-5f84-4781-9f4f-fb86719ec809（blocking）、R-002 / R-014 の読み取り経路における既存のエラー契約への退行。ルート: 委任。
- Workspace キー通知の購読・発行側の移行: `worktreePath`（隔離 cwd）と `workspaceWorktreePath`（root）が異なる isolated な Session について、backend の pause / 削除 / rename / provider title 更新の通知を Session panel が受けて再取得し、panel と WorkspaceList からの更新通知が Workspace の tree / Node 詳細の購読に届くようにする。root と cwd が異なる fixture でこの経路をテストで固定する。開始状態は backend の通知（`agent_session_rename.rs` / `provider_session_title_ingestion.rs`）が root の Workspace path をキーにする一方、`AgentSessionPanel.tsx` と `WorkspaceList.tsx` は `session.worktreePath` で照合・通知し、`useWorkspaceNodeDetail.ts` は root path で照合する。`AgentSessionLaunchAttachment` は `worktreePath` だけを持つ。既存テストの fixture は両 path が同じ値である。根拠: Thread 83c1fba8-7371-464e-b3fe-40940ce8ac90（blocking）、R-002「Session は隔離 worktree を起動 worktree として起動され」、R-014「隔離 worktree の path を Workspace として新しい Thread 集合を作らない」、design-01「変えないもの」の実行木の所属。ルート: 委任。
- 実効 cwd 導出での上位祖先の読み取り停止: Session の実効 cwd の導出で、自身または最寄りの isolated 祖先で値が確定した場合、それより上の祖先行の読み取りと定義 field の復号を行わないようにする。導出される cwd と `workspace_identity` の値は変えない。開始状態は gateway が `parent_id` が無くなるまで祖先行を読み Vec に積んでから `WorktreeInheritance::effective_path` に渡すが、この規則は最初の isolated で値を確定し後続を使わない。runtime 側の同操作は遅延 iterator でそこまでしか読まない。根拠: Thread 3f82ad26-fb16-479e-b30d-50dab04ea2f6、design-03「gateway / usecase は規則の入力の取得に徹する」、AGENTS.md「full-retention 設計を避ける」。ルート: 委任。
- path からの Workspace 解決の責務境界: path から Thread の Workspace を解決する操作（通常 path の identity 返却を含む）を共有 backend の一つの契約が所有し、CLI 側にその分岐を残さない。通常 path の review で event store の構築を増やさず、返る Thread 集合は変えない。開始状態は `cli/review.rs` の `review_workspace_worktree` と `worktree_context::workspace_worktree_path` の双方が「`isolated_worktree_owner` が None なら path をそのまま返す」分岐を持ち、CLI 側はその分岐で通常 path のときに読み取り usecase の構築を回避している。根拠: Thread cd336083-7240-4856-bbf2-3fe399a49e66、R-014、docs/architecture/README.md の同じ操作を一箇所へ集約する規約、DOMAIN.md の surface が同じ backend usecase / read model を使う規約。ルート: 委任。
- 合成子の準備要求と葉の起動要求の型による区別: domain の公開起動契約で、隔離合成子の準備要求と葉 runtime の起動要求を型で区別し、合成子が葉の起動要求として host の葉起動経路に流れないようにする。isolated 合成子の隔離 worktree 生成と children の展開・実行の観測結果は変えない。起動手順の host から usecase への移設は行わない。開始状態は `isolated_composite_start` が合成子だけを選んで `LeafStart` を返し、`isolated_worktree.rs` が kind の実行時判定で振り分け、`workflow_host.rs` は合成子を同じ型で受けると InvalidState にする。根拠: Thread 0769ba9f-cdb1-4073-9545-28eebebe14f8、docs/glossary/DOMAIN.md の葉（Session / Command）と合成 Node（Sequence / Fanout）の区別、design-02「変えないもの」の隔離起動手順の所在。ルート: 委任。
- Sequence 成果の組立の配置: 集約の `complete_scope` による Sequence 成果の組立を `artifact_query` module に依存しない配置にし、集約が query に依存し query が集約に依存する循環を実行経路から無くす。Sequence の統合 map の値は変えない。`output get` の読み取り方式と合成子決着規則の単一所有は維持する。開始状態は `artifact_query::sequence_artifact` の production 呼出し元が `workflow_execution/mod.rs` の `complete_scope` だけであり、`artifact_query.rs` は集約の scope 復元と replay に依存する。根拠: Thread 7afe560a-6cf4-4507-823c-4b5ddcd03cda、docs/architecture/DOMAIN.md「単一 entity で完結する処理は entity の impl に置き、service は複数 entity にまたがる規則を扱う」、R-007 / B-014。ルート: 委任。
- 通常 branch 保持のテスト固定: `classify_branch_cards` を通した検証で、`worktree_path` が None の通常 branch と隔離命名に一致しない Some の branch が一覧に保持され、隔離命名に一致する card だけが除外されることをテストで固定する。開始状態は `classify_branch_cards` が retain で一覧自体を変更し、同ファイルと `repository_state/service.rs` の分類テストは `worktree_path` が Some の card だけを与える。根拠: Thread 80b545df-91cd-4883-8cb4-780236ff986d、R-005 / B-010。ルート: 委任。
- 同名 slot の選択規則の production 経路でのテスト固定: 提出順と完了順が逆転する同名 slot の事実列で、production の Artifact 導出経路（`output get` が通る経路）が最後に提出した slot の成果を返すことをテストで固定する。開始状態は `fact_replay_test.rs` の逆転ケースが `cfg(test)` の `fact_replay::derive_node_artifact` だけを呼び、`execution_projection_repository.rs` から呼ばれる `artifact_query::derive_node_artifact` はこの条件で検証されていない。根拠: Thread cbe2592e-13da-4c6c-9925-0cc86b54a8eb、design-02 / design-03「変えないもの」の `output get` の対象選択規則、R-010 / B-018。ルート: 委任。

## 固定するルート

固定する実装上の指定なし。design-01〜03 の「固定する実装上の指定なし」を維持する。人間はこの周でいずれの Thread でもルート固定の案を選ばず、open Thread の採否と対応方法の具体（修正位置、切り出す単位、配置）を Impl に委ねた。合成子の branch / path の表示位置と、そのための usecase DTO / frontend 型の変更も Impl に委ねる。

## 変えないもの

- design-01 の「変えないもの」（命名規則と配置、実行木の所属、既存の失敗経路、予約環境変数を増やさない、builtin 8 本の定義本文）、design-02 の「変えないもの」（`output get` の対象選択規則、runtime の `recovery_reason` と破損データの `error_reason`、隔離起動手順の runtime host への所在）、design-03 の「変えないもの」（fact_log の NodeFailed 写像、`output get` の読み取り方式、`RELEASH_WORKTREE_PATH` 経由の読み戻し経路、`artifact_produced` の payload 表現）はそのまま維持する。理由: 人間がこの周でこれらを変更する論点を挙げなかったため、既決の方針と理由を引き継ぐ。
- 隔離起動手順の runtime host への所在。合成子の準備要求と葉の起動要求の型の区別は、この所在を変えずに行う。理由: Thread fbe3365e の見送りを引き継ぐ。
- `output get` の読み取り方式（実行木の読み取りを一度で済ませ、fold による aggregate 全体の再構築を行わない）。Sequence 成果の組立の配置と、空 items の isolated Fanout の abort 保持のいずれでも維持する。理由: 人間が Thread dcc44c17 で同じ domain 規則を通す案を選ぶ際にこの制約を保つことを条件に含めたため。
- Command からの `RELEASH_WORKTREE_PATH` 経由の review が隔離 path から root を読み戻す経路。Workspace 解決の責務境界の修正は、通常 path の review で不要な event store の構築を増やさない条件を保って行う。理由: Thread 14bdc805 の対象は `--session-id` 経路だけであり、人間がこの経路を論点に挙げなかったため。
- Artifact の値を UI に表示する経路は新設しない。合成子の branch / path の UI 表示はこの条件の下で行う。理由: Thread 85e77970 の決着であり、人間が Thread 69cbadab で案 A を選ぶ際にこの条件をそのまま採用したため。
- design-03「空 items の isolated Fanout の保存事実からの完了決着」（Started だけの事実列から完了と worktree 成果を復元する）。abort 保持の修正はこの復元を保ったまま、保存済み abort の置換だけを直す。理由: Thread 34037639 の決着。

## 未確定・リスク

- 空 items の isolated Fanout の完了導出を抑止する後続事実の範囲。Thread 7b8d1cef の受入条件は AbortRequested と RuntimeFailureObserved を伴う事実列だけを定めており、それ以外の後続事実（親 scope の abort や他 Node の事実）の前で完了を導出してよいかは定まっていない。事実列の末尾まで読んでから導出する方式を選ぶ場合、`output get` の一度の読み取りの中で末尾の判定を供給できる必要がある。
- Workspace 解決の一つの契約と通常 path での store 非構築の両立。現在 CLI が通常 path で読み取り usecase の構築を回避しているのは、指摘された分岐そのものによる。共有 backend の契約へ寄せるとき、通常 path の識別を契約の呼び出し前に済ませる手段が無ければ、cd336083 の受入条件と「変えないもの」の store 非構築のどちらかを満たせない。
- 合成子の attempt の表示単位。workspace tree の合成子 DTO は現在 attempt を持たず、`on_failure: retry` で合成子の attempt が進んだ場合に UI で参照する branch / path が最新 attempt に限られるか、attempt ごとに残るかは Requirements に定めがない。B-027 は「その合成子自身の attempt」の値を読めることだけを求める。
- 起動 attachment の Workspace キー。`AgentSessionLaunchAttachment` は隔離 cwd の `worktreePath` だけを持ち、Session の DTO が届く前の panel が root の Workspace キーで通知を照合するには attachment 側にも root の path が要る。attachment を組み立てる起動経路がその値を持つかは未確認である。
