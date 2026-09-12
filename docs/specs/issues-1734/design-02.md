# Design 02

## 開始状態

base は `main`、派生点は `c4a9f36c`（#1733 取り込み後）。直前の Design は `docs/specs/issues-1734/design-01.md`。branch `feat/issues/1734` の作業ツリーには design-01 の「変える部分」を実装した未コミットの変更（Rust の domain / usecase / gateway、`workflows/examples/full-cycle-development.yml`、`docs/glossary/WORKFLOW.md` / `DOMAIN.md`、frontend の型と表示）があり、これを今周の開始状態とする。

Review で 19 件の Thread が起票され、いずれも open のまま `[FIX_POLICY]` が付いている。resolve 済み、`[DEFERRED]`、`[REJECTED]` の Thread は無い。今周の Spec 工程では、Thread ed4034bc に対応して Requirements の R-011、Behavior の B-015、Non-goals、Assumptions が更新された（delegate の `inputs` に固有の型互換検査を持たない）。他の Requirement・Behavior は変更されていない。

## 変える部分

- delegate 固有の型互換検査の撤去: `inputs` の供給元 Contract と配線先の型付き input Contract の構造的互換を判定して Error にしている検査を取り除き、`inputs` の load 時検査を children エントリの配線と同じ集合（未宣言パラメータ、受理形でない供給元、曖昧な供給元、Artifact を持たない供給元、解決できない field path）に揃える。children エントリの配線へ型互換検査を足さない。根拠: R-011「供給元の Contract と配線先の型付き input の Contract の互換は、children エントリの配線と同じく load 時に検査せず、両者が異なる Contract である配線も Error Diagnostic にならない」、B-015、Thread ed4034bc-faf6-4014-865b-61a59041889b。ルート: 委任。
- delegate 結果の再送防止: child 結果の送信成功後・注入済み事実の保存前に中断した WorkflowExecution を resume しても、同じ child 結果が親 provider session へ再送されないようにする。根拠: R-013「注入が済んだ後の中断から resume した場合は二重に注入されない」、B-019、Thread fabbe34c-8127-4e47-b3d8-6d07abc199f3。ルート: 委任。
- 正本サンプルの `task` の Contract 照合の回復: `implement_all` の受け手 `implement_task` で、`items` 要素 Contract `implement-task` と `task` パラメータの Contract 照合が load 時に成立するようにし、正本サンプルが Diagnostic ゼロで load できる状態を保つ。根拠: R-017、B-023、Thread 81952cce-6742-4c38-afec-9c14bf607a59（base では `implement_and_verify` が `task: implement-task` を宣言し照合されていた regression）。ルート: 委任。
- 注入入口の排他条件の統一: resume 後段の pending 注入を含む全ての注入入口が、pending 確認から送信・commit までの排他条件を activation gate 経由の入口と同じく満たし、同一 pending を並行に読んで両方送信する順序が生じないようにする。根拠: R-013、B-019、Thread d57f8f44-c7e9-44ca-a15e-53c69331a30f。ルート: 委任。
- delegate 継続 usecase の組み立て場所: `DelegateContinuationUsecase` とその gateway port 実装の組み立てを composition root（controller）の責務へ移し、gateway が自ら usecase を組み立てて実行する経路を残さない。根拠: Thread 46db9bb0-c17f-4911-8def-3becaa18d3be（`docs/architecture/README.md` / `CONTROLLER.md` の composition root 規約）。ルート: 委任。
- `when` の型エラーの Diagnostic 位置: `completion.delegate.when` の型エラーの message・field・span がその Session の `completion.delegate.when` の宣言位置を指し、`rules.when.on` や無関係な辺の位置を指さないようにする。B-008 の Error 拒否は維持する。根拠: R-006、B-008、Thread c78912d0-de86-4b82-b588-2e6478a943e2。ルート: 委任。
- child 共有・包含 cycle の Diagnostic 文言: delegate の child 共有と包含 cycle の Diagnostic message が親 Session を合成 Node（composite）と表示せず、実際の Node kind とドメイン語彙に一致するようにする。B-028 / B-029 の Error 生成は維持する。根拠: R-002、B-028、B-029、Thread 1efdc93e-064c-445e-8a89-5823225cddf0。ルート: 委任。
- `max_iterations` の値域規則の所在: 1以上という値域規則を判定する場所を domain の実行経路の一箇所にし、gateway の parse は外部値の形の変換に留める。YAML で `max_iterations: 0` を書いた定義は引き続き load 時 Error Diagnostic になる。根拠: R-002、B-030、Thread 4fac6a06-7bc1-45f9-a281-67a6909a25ea。ルート: 委任。
- delegate 検査結果の構造化: 残る delegate の各検査結果（宣言可能 kind、`artifact` を宣言しない Session、child の Artifact 有無、`max_iterations` の値域）を自由文の集約でなく識別可能な構造化 Diagnostic で返し、既存の段階分類に沿って分類する。構文が正しい定義の意味的な違反を parse_shape として表示しない。根拠: R-002、Thread e2ad6b93-eec9-438a-b879-70c628027dce。ルート: 委任。
- 親参照の型: 構築・復元された `ExecutionParentRef` が sequence / fanout / delegate のいずれか一つにだけ分類され、delegate と fanout_slot を同時に持つ値を表現できないようにする。既存の永続化済み親参照の復元経路と再生テストを通す。根拠: R-012、Thread cb5f6b23-d8ae-435a-a1a5-5309b100ee67。ルート: 委任。
- 起動準備と注入の分離: Node 起動準備（worktree 準備と LeafStart の返却）と、稼働中 Session への delegate 結果注入を API・モジュールの責務として区別し、isolated worktree 準備の経路を通すだけで provider への続行指示と control-plane 更新が発生しないようにする。B-007 / B-018 の注入挙動は維持する。根拠: Thread 9d4769b9-96d9-4a15-aee6-66b9f9224125。ルート: 委任。
- terminal 投入形式の共有: 初期指示と継続指示の terminal 投入形式（末尾 CR/LF 除去、bracketed paste と CR の付加、terminal への write）を一つの実装で共有し、admission の差だけを分岐として残す。根拠: Thread a6cedc5f-4651-4551-abe2-ddfa91502079。ルート: 委任。
- Sequence / Fanout child の起動経路テスト: 親 Session の提出を起点に Sequence child と Fanout child をそれぞれ実際の起動経路で起動・完了させ、統合 map が `child` キーとして注入され親の述語評価へ接続することを検証するテストを追加する（Started fact の手動投入による再生だけに依らない）。根拠: R-008、B-012、R-003 / R-005、Thread 36880d01-579e-4186-8b4d-d8a6a808c5b3。ルート: 委任。
- 親 input の child への伝播テスト: 合成子から親 Session へ渡された input 値が、初回と再発火の delegate child の bindings に保持されることを検証するテストを追加する。根拠: R-011、B-016、Thread d3244571-90d7-4487-a677-936fb93ae6e6。ルート: 委任。
- 未完了 child を持つ resume のテスト: 未完了の delegate child を持つ WorkflowExecution を中断・再起動・resume したとき、既存の child が再開され、追加の child が作られず、親 Session は待機のまま保たれ、child 完了後に注入へ進むことを検証するテストを追加する。根拠: R-013、B-018、Thread d52246ba-4d40-4e00-bc05-25e4997c991e。ルート: 委任。
- 過去 attempt の child 部分木投影のテスト: retry 履歴を持つ親 Session の過去 attempt 配下に、その attempt の delegate child 部分木が保持されて投影されることを検証するテストを追加する。根拠: R-012、B-017、Thread 356673c7-c494-45e9-b429-fd0e0d8ad01c。ルート: 委任。
- 承認境界のテスト: `require: approval` と `delegate` を併記した Session が、child 待ち・述語未成立かつ上限未達の間は WaitingApproval にならず Approve が受理されないことを検証するテストを追加する。根拠: R-015、B-021、Thread e5a44958-9657-4fe6-99e1-ddb3c327d932。ルート: 委任。
- 下流配線 `<親Session>.child.<field>` のテスト: 後続の children エントリが `<親Session>.child.<field>` を `inputs` に配線した定義が Error Diagnostic なく load でき、実行時にその input 値が最後の delegate 結果に一致することを検証するテストを追加する。根拠: R-010、R-008、B-014、Thread 70d89426-5f77-4462-82c9-2933e1ed571d。ルート: 委任。
- Diagnostic テストの識別性: 宣言制約・未知 child・包含制約の各ケースが到達不能 Error（`WFC001`）だけでは成功せず、当該の Diagnostic が発生したことを識別して検証するようにテストを直す。根拠: R-002、B-002 / B-003 / B-028 / B-029、Thread 5012cd26-58d7-4ad6-9e67-ba47d2d67837。ルート: 委任。

## 固定するルート

固定する実装上の指定なし。design-01 も固定するルートを持たないため、維持するルートも無い。

## 変えないもの

design-01 の「変えないもの」（Lua 表面の `completion` table、#1731 / #1732 / #1733 で確定した規則、Node が一つの親だけを持つ規則と包含 cycle の拒否、builtin 8本の定義本文、delegate child 固有の resume・Retry 経路と復旧経路を持たないこと）を維持する。今周に人間が新たに明示した維持条件は無い。

## 未確定・リスク

- 自動判断（人間の確認を経ていない仮定）は Requirements の Assumptions が列挙する10件である。今周に追加されたのは、`inputs` の load 時検査を children エントリの配線と同じ集合に限り、供給元 Contract と配線先の型付き input Contract の互換を検査しないこと（R-011 / B-015 / Non-goals）。design-01 までの9件（正本サンプルの適用先を `implement_task` に限ること、Lua 表面を未決として Non-goals に置くこと、`inputs` で親の Artifact を参照する名前を親 Session の Node 名にすること、`artifact` を宣言した Session だけが `delegate` を宣言できること、`child` 予約キーの検査を delegate 親の Contract に限ること、`max_iterations` を1以上とすること、child を Artifact を持つ Node に限ること、child の共有禁止と自己包含禁止、delegate だけからの参照を到達可能とみなすこと）は変更されていない。人間が異なる判断をした場合、R-001 / R-002 / R-009 / R-011 / R-017 / R-019 と対応する Behavior が変わる。
- `[DEFERRED]` で人間へ渡した件: design-01 から継続する Lua 表面での `completion.delegate` の受理形（親 Session 自身の Artifact field を指す Lua の参照手段を含む）。今周も決定は無く、今周に新たに `[DEFERRED]` とした Thread は無い。
- 再送防止と注入保証の両立。R-013 は「注入前に中断すれば resume で注入する」と「注入後に中断すれば二重に注入しない」の両方を求めるが、provider session への送信と注入済み事実の保存は一つのトランザクションにならない。事実を送信前に保存すれば送信前の中断で注入が失われ、送信後に保存すれば現行と同じ再送が残る。Thread fabbe34c の受入条件を満たす方式（送信の結果単位の冪等化、または送信と保存の順序と失敗時の復旧の組み合わせ）は委任されており、選んだ方式が B-018 と B-019 のどちらかを損なうと R-013 を満たせない。
