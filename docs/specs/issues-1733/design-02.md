# Design 02

## 開始状態

base は `main`、派生点は `a31937204`（#1732 取り込み後）。branch `feat/issues/1733` は派生点と同一 commit のままで、design-01 に基づく Impl の成果は未コミットの変更（121 ファイル）として working tree にある。この未コミットの実装をこの周の開始状態とする。直前の Design は `docs/specs/issues-1733/design-01.md`。

design-01 の「変える部分」は開始状態で実装済みであり、この周へ再掲しない。design-01 の「未確定・リスク」3 点は実装で決着した。AgentSession の `worktree_path` は実行木から導出する隔離 cwd とし Workspace の identity は別項目で持つ、隔離 branch / path は attempt 開始時に NodeExecution の永続 field として残す、Session の起動失敗と resume 時の起動失敗は Node failure に落ちる。

この周までに解消・見送りとなった Thread は次のとおり。

- 85e77970-8c96-4aa4-af87-efafb9994559（Artifact 産出後の UI で Artifact の worktree キーを確認できない）: R-010 / B-018 / B-023 を改めて resolve。UI は Artifact の値を表示する経路を持たず Node 詳細の branch / path が担うと確定した。改めた R-010 / B-018 / B-023 は開始状態の実装が満たしており、実装の変更を伴わない。
- fbe3365e-cf43-4cdf-a29d-0593942f7005（隔離起動手順の gateway 所有）: [DEFERRED] で resolve。起動手順は runtime host が所有する既存構造であり、この分岐だけを usecase へ移すと起動手順が層をまたいで分断されるため、本 Issue の範囲外の横断課題として別途報告する。
- e318e97f-06b4-485a-a411-67b2138bcaf3: [OUT_OF_SCOPE] で resolve 済み。

open Thread は 10 件で、全件に [FIX_POLICY] が投稿済みであり、この周で変える部分の全部である。

## 変える部分

- `output get` の提出情報と Artifact の整合: isolated Node の `releash workflow output get` が返す提出情報（contract / submitted_at / request_id / timestamp）と structured_output（`worktree` キーを含む）を同じ NodeExecution から取る。開始状態は `usecase/workflow/output.rs` の isolated 分岐が、提出イベントと aggregate の Artifact をそれぞれ node 名だけで逆順に選んで結合しており、先に開始した slot だけが提出済みのとき NotSubmitted になる。対象 NodeExecution の選択規則（execution_id と node 名で選び、同名 slot のうち最後に提出したものを返す）は変えない。根拠: Thread 4f3a623a-8623-4d27-a0bb-4d827ee462ff（blocking）、R-010「その Artifact を取得できる各経路 … で Artifact の `worktree.branch` / `worktree.path` を読める」、B-018。ルート: 委任。
- `output get` の読み取り方式: 上記と同じ経路で、isolated Node の `output get` が実行木を二重に全件読みし aggregate 全体を再構築する分岐を残さない。非 isolated Node と同じく実行木の読み取りを一度で済ませる。開始状態は `read_events` で木全体を読んだ後、isolated 分岐で `get_node_artifact` から `fold_tree_from` を呼び再読込と再構築を行う。根拠: Thread 148af392-533c-4fb5-b24f-3e607a22980a、AGENTS.md「full-recompute 経路を増やさない」。ルート: 委任。
- isolated 合成子の子開始 append 失敗の決着: `prepare_isolated_starts` で合成子の子開始イベントの append が失敗した場合に、失敗した起動対象（合成子の node_execution_id）との対応を保持したまま R-002 の failure 規則で決着させる。対象の合成子が Running のまま残らず、対象外の leaf を失敗扱いにしない。root が isolated な合成子でも同じ。開始状態は append 失敗を `?` でそのまま返し、呼び出し元が最新の active な非合成 leaf を決着先に選ぶ。根拠: Thread 85518a37-bc35-49ce-8af2-40b18896d8c3、R-002「Sequence / Fanout に宣言した `isolated` の生成失敗も同じくその合成子の failure になる」、B-025。ルート: 委任。
- recovery fence 経路の削除: production で None しか供給されない recovery fence の導出・適用・分類経路（`RecoveryFenceProjected` の生成と適用、`recovery_owner_reason`、owner_reasons の集計、fence 判定）と、それにだけ Some を与えるテストを削除する。別用途の runtime の `recovery_reason` と破損データの `error_reason` は対象外。根拠: Thread ed713330-4309-456e-a05e-dc1949bab60f、R-005「隔離環境喪失の recovery reason も存在しない」、B-011、design-01「台帳の削除」の削除範囲の完了。ルート: 委任。
- failure settlement の未使用引数の除去: `settle_node_failure_for_node` / `settle_runtime_failure_for_node` とその呼び出し元に残る未使用の `worktree_path` 引数の連鎖と、その受け渡しのためだけの clone を削除する。決着先の worktree は snapshot から取る。根拠: Thread 4a223782-6b72-47ad-a4df-c3f99cfddbcb。要求・振る舞いの変更なし。ルート: 委任。
- 隔離判定と実効 worktree 継承の規則の単一所有: 隔離判定と親からの実効 worktree 継承の規則を domain の一箇所が所有し、runtime の cwd 決定（`WorkflowExecution::execution_worktree_path`）、AgentSession context の読み取り（`worktree_context::execution_worktree_path`）、`output get` の隔離判定（`event_draft::node_is_isolated_in_drafts`）がそれを使う。gateway / usecase は規則の入力（先祖の行、保存定義の復号）の取得に徹する。開始状態は 3 箇所が同じ規則を別々に実装している。根拠: Thread 3fa42346-619f-4d26-8f10-8972c44382f5、R-002 の継承規則、B-007 / B-008、R-014、docs/architecture/DOMAIN.md「一つの概念に一つの表現」。ルート: 委任。
- 生成 editor stub の worktree 公開形のテスト: 生成された stub の `Worktree` 型、`shared` / `isolated` の handle、module field、4 builder の `worktree?` を検証するテストを追加し、公開契約が欠けたら失敗するようにする。根拠: Thread 89a86a47-3106-41b2-bded-54afd636ebfc、R-013 / B-021、docs/architecture/TEST.md「adaptor/gateway/: 必須」。ルート: 委任（配置、fixture）。
- Session 起動 port の workspace / cwd 境界のテスト: Session 起動 port の境界で、root Workspace と隔離 cwd が別々の値として起動要求の workspace / worktree_path へ写ることを検証するテストを追加し、同じ値に戻す・取り違える変更で失敗するようにする。根拠: Thread 57450817-b077-4015-ada1-3a58ada204b7、R-002 / B-003、R-014 / B-024、TEST.md「adaptor/gateway/: 必須」。ルート: 委任（配置、fake の構成）。
- 隔離 Session の resume 後の dispatch 失敗分岐のテスト: 隔離 Session の resume で provider の復元に成功した後に初期指示の dispatch が失敗した場合の分岐（対象の除外、他の再開状態の復元、対象 attempt の failure 決着）を検証するテストを追加する。根拠: Thread 319cc447-f0a0-4317-9b21-877022128add、R-002 / R-005 の failure 規則、TEST.md「adaptor/gateway/: 必須」。ルート: 委任（配置、fake の構成）。
- Node 詳細への worktree 導出のテスト: runtime の worktree を workspace tree の Node と Node 詳細 DTO へ写す境界（projection と query service）に Some を与え、実行中と Artifact なし終端で branch / path が保持されることを検証するテストを追加し、写しが欠落したら失敗するようにする。根拠: Thread 47ef143f-de8e-4721-a7d1-c3c45d163cb8、R-010 / B-023、TEST.md「domain/: 必須」「adaptor/gateway/: 必須」。ルート: 委任（配置、fixture）。

## 固定するルート

固定する実装上の指定なし。design-01 の「固定する実装上の指定なし」を維持する。

## 変えないもの

- design-01 の「変えないもの」（命名規則と配置、実行木の所属、既存の失敗経路、予約環境変数を増やさない、builtin 8 本の定義本文）はそのまま維持する。
- `output get` の対象選択規則。execution_id と node 名で対象を選び、同名 slot のうち最後に提出したものを返す既存挙動は変えない。理由: 派生点以前からの挙動であり、本 Issue の要求は選択規則でなく提出情報と Artifact が同じ NodeExecution に属することだけを求める（Thread 4f3a623a のトリアージ決定）。
- runtime の `recovery_reason` と破損データの `error_reason`。理由: 隔離喪失の fence とは別用途であり、R-005 の削除対象でない（Thread ed713330 のトリアージ決定）。
- 隔離起動手順（隔離生成・合成子展開・永続化・失敗決着）の runtime host への所在。理由: Thread fbe3365e を見送りとし、host 全体の再配置を別途報告する横断課題とした。Thread 85518a37 の修正は所在を変えずに行う。

## 未確定・リスク

- usecase の読み取り表現が NodeExecution の識別を運ばない。`WorkflowEventDraft`（`usecase/workflow/ports.rs`）は execution_id・event_kind・timestamp・payload だけを持ち、`artifact_produced` の payload は node 名・contract・value・request_id だけで、event store の行が持つ node_execution_id は落ちている。「同じ NodeExecution から取る」と「実行木の読み取りを一度で済ませる」を両立するには、一度の読み取りで提出情報と NodeExecution の識別を対応付けられる表現が要る。想定が外れると 4f3a623a か 148af392 のどちらかの受入条件を満たせない。解決方法は委任。
- 隔離判定の単一所有と `output get` の読み取り方式の連動。`output get` の隔離判定を domain の規則に委ねる場合、その規則の入力（先祖の行、保存定義の復号）を上記の一度の読み取りから供給できる必要がある。両 Thread を別々に満たす変更が互いを壊さないことは Impl で確認が要る。
