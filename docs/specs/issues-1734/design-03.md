# Design 03

## 開始状態

base は `main`、派生点は `c4a9f36c`（#1733 取り込み後）。直前の Design は `docs/specs/issues-1734/design-02.md`。branch `feat/issues/1734` の作業ツリーには design-01 と design-02 の「変える部分」を実装した未コミットの変更（Rust の domain / usecase / gateway / controller / presenter、`workflows/examples/full-cycle-development.yml`、`docs/glossary/WORKFLOW.md` / `DOMAIN.md`、frontend の型と表示）があり、これを今周の開始状態とする。

design-02 の「変える部分」に対応する Thread のうち 17 件（ed4034bc、81952cce、d57f8f44、46db9bb0、1efdc93e、4fac6a06、e2ad6b93、cb5f6b23、9d4769b9、a6cedc5f、36880d01、d3244571、d52246ba、356673c7、e5a44958、70d89426、5012cd26）は解消済みとして resolve された。8de80fee（注入済み fact の永続化形式の所在）は成立しない指摘として `[REJECTED]` で resolve された。c78912d0 と fabbe34c は前周の `[FIX_POLICY]` のまま `[STILL_OPEN]` で継続し、今周の Review で 13 件が新たに起票されて `[FIX_POLICY]` が付いた。open Thread は 15 件であり、いずれも分類 implementation（223280a7 のみ scope）で、今周の Spec 工程で Requirements・Behavior は変更されていない。`[DEFERRED]` の Thread は無い。

## 変える部分

- `completion` 未知キーの YAML / Lua Diagnostic の一致: `delegate` を含まない `completion` の未知キー（例: `{require: approval, extra: true}`）について、YAML と Lua の Diagnostic の code・stage・message を一致させ、`diagnostics_test.rs` の YAML / Lua 比較ケースにその組を復元する。Lua の `completion` table が `delegate` を受理せず Error になること（B-025）は維持する。根拠: R-002、B-002、`docs/glossary/WORKFLOW.md`「Diagnostic」節（YAML と Lua の同じ誤りに同じ code・stage・message）、Thread 3fc50e32-3cef-4097-9b82-2ed7b65ebc4e。ルート: 委任。
- frontend の未供給 `delegate` 宣言の取り消し: `src/types/workflow.ts` の `ExecutionParentRef` に追加された `delegate?: boolean` の宣言を取り除き、field を backend の `ExecutionParentRefView`（parentId / itemIndex / childIndex）と一致させる。backend の view と presenter は変更しない。根拠: R-012、B-017、Non-goals「frontend へのロジック追加」「実行木の UI への新しい表示要素の追加」、Thread 223280a7-7b37-40f6-b753-5ffba4f8388d（分類 scope）。ルート: 取り消し範囲は当該宣言のみ。
- delegate bindings の供給元選択の一元化: delegate child の bindings の供給元選択（delegate 親なら delegate の bindings、それ以外は scope の配線）を一つの判断に集約し、新規起動・再発火・leaf 再構成のいずれもその判断から bindings を得るようにする。B-016 の結果は維持する。根拠: R-011、B-016、Thread cda02e8b-a366-4a1c-9c3d-b36810ebce1b。ルート: 委任。
- delegate 検査 kind と Diagnostic field path の対応の一元化: `InvalidDelegateKind` から Diagnostic の field path への対応を一つの定義に基づかせ、domain の message と gateway の field・span 選択が同じ対応から導かれるようにする。各 kind の message と field の値は現行と一致させる。根拠: R-002 / R-006、B-002 / B-008、design-02「`when` の型エラーの Diagnostic 位置」「delegate 検査結果の構造化」、Thread 2be5109d-cc16-4adb-8d34-e3eb4e70e035。ルート: 委任。
- 無名インライン Session の判定の所在: delegate の `inputs` 配線検査で無名インライン Session を判定する際に、合成名（`owner#child_index`）の生成規則を所有する側の操作を通し、名前文字列の `#` を配線検査が独自に解釈しないようにする。B-015 の受理・拒否の結果は維持する。根拠: R-011、B-015、Thread efc9f717-7269-4c29-b389-b4c16a6bd2db。ルート: 委任。
- 新設テストの配置: `workflow_host/delegate_test.rs` を `delegate.rs` の末尾から取り込み、`isolated_worktree_test.rs` 経由の取り込みを残さない。`workflow_execution/mod.rs` の inline `#[cfg(test)]` に追加された正本サンプルのテストを対応するテストモジュール（`mod_test.rs`）へ移し、Given / When / Then の区切りを持たせる。既存テストの移設は行わない。根拠: `docs/architecture/TEST.md`「配置」「テスト構造」、Thread f2cea3f3-3bf3-464f-b342-a4dd74025c64。ルート: 委任。
- child 待ち中の再 Submit の拒否理由: child 待ち中の親 Session（存在し Running）へ同じ NodeExecution ID で Artifact を再提出したとき、Node の状態による拒否を表すエラーを返し、NodeExecution の消失（disappeared）と区別する。二重 Submit の受理仕様は変えない。根拠: R-003 / R-004、B-005、Thread 4f27811b-d370-4625-8900-f1f674e95616。ルート: 委任。
- child の失敗と手動 Retry のテスト: delegate child 自身の失敗 → 手動 Retry → 完了の経路について、親 Session が待機を保つこと、Retry 後の child の結果が親に注入されること、事実の再生で live と同じ delegate 状態（待つ child と発火回数の扱い）が得られることを検証するテストを追加する。根拠: R-007 / R-012 / R-013、B-010 / B-017 / B-018、`docs/architecture/TEST.md`、Thread 8e9309ee-3e76-466d-982b-8e1f04de152e。ルート: 委任。
- 非空 `inputs` の保存・復元のテスト: 非空の `completion.delegate.inputs` を持つ定義の保存形式が往復で保たれること、およびその定義の WorkflowExecution を再起動・resume した後に起動する child の bindings が `inputs` の配線どおりに解決されることを検証するテストを追加する。根拠: R-011 / R-013、B-016 / B-018、`docs/architecture/TEST.md`、Thread 1d78dc31-fd4a-4850-9018-f6140a041250。ルート: 委任。
- 注入済み fact の decoder の前方互換: `delegate_result_injected` の detail に有効な childExecutionId と非文字列の未知 field が併存しても `NodeFact::DelegateResultInjected` として復元されるようにし、その境界を検証するテストを追加する。childExecutionId の欠落・空文字が DetailMismatch になることは維持する。根拠: R-013、B-018 / B-019、既存の NodeFact 前方互換契約（`node_fact_test.rs`）、Thread 638922f3-a00f-420f-a648-1605c217dcb6。ルート: 委任。
- `artifact` 省略の isolated Session child のテスト: `artifact` を宣言せず `worktree: isolated` だけを宣言した Session を child とする delegate 定義が Error Diagnostic なく load できること、およびその child の完了時に `worktree` キーを持つ Artifact が親 Session の `child` キーとして注入されることを検証するテストを追加する。根拠: R-001 / R-016、B-001 / B-022、`docs/architecture/TEST.md`、Thread 46d68fc7-61a3-4224-8d08-0b27d0869f9b。ルート: 委任。
- 初回選択候補の走査の Session 配下への降下: Session 配下に delegate child を持つ実行木で、初回表示および選択消失時の fallback の候補走査が Session の子へ降下し、実行中・待機中の child が存在するとき既存の running / waiting 優先規則に従ってそれが選ばれるようにする。子を持たない Session / Command が候補になる既存挙動は維持する。根拠: R-012、B-017、`docs/specs/milestone-82/design.md` の初回選択候補の既存契約、Thread 7d78602f-3013-4c30-8620-7ada48a178ff。ルート: 委任。
- 子を持つ Node の分類の一元化: 「子 NodeExecution を持つ Node（合成子、または `completion.delegate` を宣言した Session）」の判定を domain の一つの規則に集約し、Artifact 再生の親開始受理・部分木 Artifact の再生・子の再生選択と、包含 cycle 検査の降下がその規則を参照するようにする。B-011 / B-012 / B-017 / B-028 / B-029 の結果は維持する。根拠: R-008 / R-012、B-011 / B-017、Thread 97ae2378-39e2-4aec-9f89-3a4d95715a5c。ルート: 委任。
- `when` の型エラーの message（design-02 から継続）: `completion.delegate.when` の型エラーの Diagnostic の message が辺（`rules.when.on`）の構文名を名指さず、field・span と同じくその Session の `completion.delegate.when` の宣言を指すようにする。field・span は design-02 の実装で既に宣言位置を指しており、message の分岐が残っている。B-008 の Error 拒否は維持する。根拠: R-006、B-008、Thread c78912d0-de86-4b82-b588-2e6478a943e2（`[STILL_OPEN]`）。ルート: 委任。
- delegate 結果の再送防止（design-02 から継続）: child 結果の送信成功後・注入済み事実の保存前に中断した WorkflowExecution を resume しても、同じ child 結果が親 provider session へ再送されないようにする。design-02 で追加された注入入口の排他条件は並行送信には効くが、保存前の中断からの再注入経路は残っている。B-018 の未注入結果の注入は維持する。根拠: R-013、B-018 / B-019、Thread fabbe34c-8127-4e47-b3d8-6d07abc199f3（`[STILL_OPEN]`）。ルート: 委任。

## 固定するルート

固定する実装上の指定なし。design-01 / design-02 も固定するルートを持たないため、維持するルートも無い。

## 変えないもの

- design-01 / design-02 の「変えないもの」（Lua 表面の `completion` table、#1731 / #1732 / #1733 で確定した規則、Node が一つの親だけを持つ規則と包含 cycle の拒否、builtin 8本の定義本文、delegate child 固有の resume・Retry 経路と復旧経路を持たないこと）を維持する。
- 二重 Submit の受理仕様。child 待ち中の再 Submit は拒否のまま、その拒否理由の表し方だけを変える。理由: Thread 4f27811b の `[FIX_POLICY]` が受理仕様の変更を求めておらず、Requirements がその条件の結果を定めていない。
- backend の `ExecutionParentRefView` と presenter。理由: Thread 223280a7 の取り消し範囲は frontend の宣言だけであり、公開契約に delegate 真偽値を加える要求はない（R-012 は親子関係の観測だけを求める）。
- `node_fact.rs` が NodeFact の serde 形を永続形として domain に置く既存契約。理由: Thread 8de80fee は成立しない指摘として `[REJECTED]` で resolve され、`DelegateResultInjected` はその契約に従う追加である。

## 未確定・リスク

- 自動判断（人間の確認を経ていない仮定）は Requirements の Assumptions が列挙する10件であり、今周の変更は無い。今周に新たに自動判断した事項は、Thread 8de80fee-c203-4a1f-a41a-13fd9afaabf4 を成立しない指摘として `[REJECTED]` で resolve したこと（NodeFact の永続形変換を domain が所有する既存契約を維持し、この variant だけの移設を採らない）である。人間が異なる判断をした場合、NodeFact の encode / decode の所在が変わる。
- `[DEFERRED]` で人間へ渡した件: design-01 から継続する Lua 表面での `completion.delegate` の受理形（親 Session 自身の Artifact field を指す Lua の参照手段を含む）。今周も決定は無く、今周に新たに `[DEFERRED]` とした Thread は無い。
- 再送防止と注入保証の両立（design-02 から継続）。provider session への送信と注入済み事実の保存は一つのトランザクションにならず、design-02 で委任した方式（送信の結果単位の冪等化、または送信と保存の順序と失敗時の復旧の組み合わせ）は今周の開始状態でも選ばれていない。選んだ方式が B-018 と B-019 のどちらかを損なうと R-013 を満たせない。
- `completion` 未知キーの message の形。Lua が `delegate` を受理しない（Non-goals / B-025）まま YAML と Lua の message を一致させるには、受理キーを列挙しない表面中立の文言にするか、Lua でも `delegate` を未知キーとは別の理由で拒否する形にするかを実装が選ぶ。選んだ形が Lua の `delegate` 拒否を WFS002 parse_shape の Error から変えると B-025 を満たせない。
