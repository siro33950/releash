# Design 04

## 開始状態

base は `main`、派生点は `c4a9f36c`（#1733 取り込み後）。直前の Design は `docs/specs/issues-1734/design-03.md`。branch `feat/issues/1734` の作業ツリーには design-01 から design-03 までの「変える部分」を実装した未コミットの変更（Rust の domain / usecase / gateway / controller / presenter、`workflows/examples/full-cycle-development.yml`、`docs/glossary/WORKFLOW.md` / `DOMAIN.md`、frontend の型と表示）があり、これを今周の開始状態とする。

design-03 の「変える部分」に対応する Thread のうち 14 件（3fc50e32、223280a7、cda02e8b、2be5109d、efc9f717、f2cea3f3、4f27811b、8e9309ee、1d78dc31、638922f3、46d68fc7、7d78602f、97ae2378、c78912d0）は解消済みとして resolve された。fabbe34c は design-02 / design-03 の `[FIX_POLICY]` のまま三度目の `[STILL_OPEN]` で継続し、今周の Review で 8 件が新たに起票されて `[FIX_POLICY]` が付いた。open Thread は 9 件であり、いずれも分類 implementation で、今周の Spec 工程で Requirements・Behavior は変更されていない。`[REJECTED]`・`[DEFERRED]` の Thread は無い。

## 変える部分

- 初回選択候補からの Running / Waiting 親 Session の除外の解消: 子を持つ Session を初回表示および選択消失時の fallback の候補から無条件に外す条件を改め、実行中・待機中の child があればそれを選ぶ design-03 の条件を保ちつつ、Running の親 Session と完了済み child だけの可視部分木では親 Session が選ばれるようにする。child が全て hidden でも可視の Running / Waiting 親が候補になる。根拠: R-012「親 Session の NodeExecution は child の実行中も完了しておらず、child の完了後も同じ NodeExecution・同じ attempt・同じ AgentSession のまま続行する」、B-007 / B-017、design-03「初回選択候補の走査の Session 配下への降下」（子を持たない Session / Command が候補になる既存挙動の維持）、`docs/specs/milestone-82/design.md` の Running / Waiting 優先の既存契約、Thread 70b3cd2e-3531-4002-bfa2-759a51785160（blocking）。ルート: 委任。
- 過去 attempt 行の kind 誤判定の解消: backend が返す実際の JSON（過去 attempt に kind tag が無く、children が空なら field 自体が省略される形）を受け取っても、retry history の展開が例外終了せず、過去 Session / Command 行と配下の delegate 部分木が表示・選択できるようにする。frontend にロジックを足さない。根拠: R-012「UI、local API の execution 取得、CLI の `releash workflow status --json` のそれぞれで、親 Session の下に発火ごとの行として観測できる」、B-017、Non-goals「実行木の UI への新しい表示要素の追加」「frontend へのロジック追加」、Thread 4b8fafcb-99e5-433e-80db-f44bccd738b5（blocking）。ルート: 委任。
- delegate `inputs` 解析の children 配線との共通化: delegate の `inputs` の解析を children エントリの配線と共通の操作に基づかせ、同じ配線を書いたときに child の prompt へ並ぶ input の順序が children エントリの配線と一致する（宣言順が保持される）ようにする。根拠: R-011「`inputs` は `<パラメータ名>: <供給元>` の map であり…children エントリの配線と同じ規則で load 時 Error Diagnostic になる」、B-016、`docs/architecture/README.md` の同一操作の実装集約、Thread f6a349db-7b90-4f1e-82be-1cdb7b2a6c2d。ルート: 委任。
- delegate 提出遷移の live / replay の一元化: delegate 提出の受理（提出内容の記録、`child` キーの合成、phase の更新と述語の評価）を一つの操作にし、live 提出と fact replay の両入口がその操作を適用して同じ遷移を得る構造にする。根拠: R-003 / R-004、B-004 / B-005、`docs/architecture/README.md` の同一操作の実装集約、`docs/glossary/DOMAIN.md` の事実受理と記録を一つの操作にする規約、Thread 1fa37967-d6ba-439f-98b1-e062fd1a81e6。ルート: 委任。
- delegate 続行 gateway の control plane 操作との一元化: delegate 続行が行う実行状態の取得と control plane commit への変換を、既存 control plane の同じ操作と一つの実装に基づかせ、取得と candidate 構築の再記述を残さない。根拠: R-013 / R-014「注入後の続行、および resume 後の続行は親 Session の provider session を復元して行う」、B-018、`docs/architecture/README.md` の同一操作の実装集約、Thread 979c9667-fefb-4643-b042-117803f02473。ルート: 委任。
- false child が親 Stop より先に完了する順序のテスト: child が `passed: false` で親 provider の Stop より先に完了する順序について、Stop 前は注入待ちにならず、Stop 後に同じ結果が一度だけ注入されること、およびその Stop 待ちの間の中断・再開で完了済み child が再起動されず結果が引き継がれることを検証する domain / gateway のテストを追加する。根拠: R-005 / B-007、R-013 / B-018、`docs/architecture/TEST.md` の domain / gateway 必須検証、Thread 61076825-09cf-4458-bccc-8dc12938f8e3。ルート: 委任。
- 親提出済み・child Started 未保存からの復旧前進のテスト: 親 Session の Artifact 提出は保存済みで delegate child の Started fact が未保存の fact 列から、child 起動の前進が導出・適用されること、開始事実の追記後は同じ前進が再導出されないこと、および復元不能な依存の扱いを検証するテストを追加する。根拠: R-004「偽なら child を起動し、Session は完了せず child の完了を待つ」、R-013、B-018、`docs/architecture/TEST.md`、Thread b0d83437-061e-4661-a6af-1a68019d8efa。ルート: 委任。
- 過去 attempt 配下の delegate child の選択維持のテスト: 過去 attempt 配下の delegate child（およびその部分木）を選択した状態が新しい snapshot との照合で維持され、snapshot に実際に存在しない Node は不在と判定されることを検証する usecase テストを追加する。根拠: R-012、B-017、`docs/specs/milestone-82/design.md` の選択維持契約、`docs/architecture/TEST.md`（usecase テスト必須）、Thread 4dc9f589-32e3-4088-9254-8b531443b290。ルート: 委任。
- delegate 結果の再送防止（design-02 / design-03 から継続）: child 結果の送信成功後・注入済み事実の保存前の中断または保存失敗から resume しても、同じ child 結果が親 provider session へ再送されないようにする。送信前に中断した未注入結果は B-018 に従い届ける。送信成功後に注入済み事実の記録と commit を行う順序と、送信経路に結果単位の重複排除が無い構造は今周の開始状態でも残っている。根拠: R-013「注入が済んだ後の中断から resume した場合は二重に注入されない」、B-018 / B-019、design-01「注入の事実化と resume」、Thread fabbe34c-8127-4e47-b3d8-6d07abc199f3（`[STILL_OPEN]`、blocking）。ルート: 委任。

## 固定するルート

固定する実装上の指定なし。design-01 から design-03 も固定するルートを持たないため、維持するルートも無い。

## 変えないもの

- design-01 から design-03 の「変えないもの」（Lua 表面の `completion` table、#1731 / #1732 / #1733 で確定した規則、Node が一つの親だけを持つ規則と包含 cycle の拒否、builtin 8本の定義本文、delegate child 固有の resume・Retry 経路と復旧経路を持たないこと、二重 Submit の受理仕様、backend の `ExecutionParentRefView` と presenter、`node_fact.rs` が NodeFact の serde 形を永続形として domain に置く既存契約）を維持する。
- 実行中・待機中の child があればそれを選ぶ初回選択の条件。理由: design-03「初回選択候補の走査の Session 配下への降下」で定めた条件であり、Thread 70b3cd2e の `[FIX_POLICY]` はその条件を保ったまま親の除外だけを解消することを求める。
- Requirements の Assumptions が列挙する自動判断 10 件。今周に新たな自動判断は無い。

## 未確定・リスク

- 自動判断（人間の確認を経ていない仮定）は Requirements の Assumptions が列挙する 10 件と、design-03 で Thread 8de80fee を `[REJECTED]` で resolve した判断であり、今周の変更は無い。
- `[DEFERRED]` で人間へ渡した件: design-01 から継続する Lua 表面での `completion.delegate` の受理形（親 Session 自身の Artifact field を指す Lua の参照手段を含む）。今周も決定は無く、今周に新たに `[DEFERRED]` とした Thread は無い。
- 再送防止と注入保証の両立（design-02 / design-03 から継続）。provider session への送信と注入済み事実の保存は一つのトランザクションにならず、委任した方式（送信の結果単位の冪等化、または送信と保存の順序と失敗時の復旧の組み合わせ）は三周続けて選ばれていない。選んだ方式が B-018 と B-019 のどちらかを損なうと R-013 を満たせない。
- 過去 attempt 行の kind の補い方。過去 attempt の kind を backend の DTO 側で付与するか、frontend が受け取った形をそのまま既存の Node 行として扱うかを実装が選ぶ。前者は usecase / gateway の DTO と既存テストの fixture に影響し、後者は Non-goals「frontend へのロジック追加」に触れない範囲でなければならない。
- 初回選択候補の条件の形。子を持つ Session を候補に戻す際、実行中・待機中の child がある場合の優先と、child が全て hidden の場合の親の候補化を同時に満たす走査順を実装が選ぶ。design-03 の条件を崩すと Thread 7d78602f の解消が退行する。
