# Design 02

## 開始状態

base は `main`、派生点は `94963548d`（#1731 取り込み後）。branch `feat/issues/1732` は派生点と同一 commit で、直前の Design は `docs/specs/issues-1732/design-01.md` である。design-01「変える部分」の各項目は worktree 上の未コミット変更として実装済みであり、Spec 工程ではコードを変更していないため、この実装を今周の開始状態とする。design-01「未確定・リスク」が挙げた YAML と Lua の `completion` Diagnostic の文言不一致は、開始状態では両表面が同じ形の規則と文言を共有して解消している。

開始状態の非テストコードで、廃止した文字列形式 `completion: approval` を引用する箇所は、`src-tauri/src/domain/workflow/entities/workflow_execution/mod.rs` の承認対象検証エラー本文（2119〜2124 行目）の1箇所だけである。この本文は usecase と gateway の変換で保持され、local API の 403 応答（`adaptor/controller/api/error.rs`）と CLI の入力エラー（`cli/file_direct.rs`）へそのまま渡る。

この周までの Thread は次のとおりである。

- `[FIX_POLICY]` 付き open Thread: `eadbdff6-bc67-49b6-aea5-642c905b801a`（承認対象検証エラーの本文が廃止構文 `completion: approval` を案内する）。
- `[DEFERRED]` で resolve 済み: `2553a4d0-e1d0-4e36-a028-7180718cad00`（`NodeCompletion` の serde 実装の adaptor 配置）、`c422e0d7-0650-4d75-9fed-48cd899c0760`（Command 完了シナリオの新規テストの inline test module 配置）。
- `[OUT_OF_SCOPE]` で resolve 済み: `e3ca6e59-dfe3-4f3a-b98f-b380fd064308`、`06f483c1-c519-4782-9c82-a64d835989ed`、`34222467-e974-4650-9047-5a803af3cad9`。いずれも派生点から存在する構造で、今回の変更が導入も悪化もしていない。

## 変える部分

- 承認要求未宣言エラー本文の変更: 承認対象の検証で、対象 NodeExecution が WaitingApproval であることを確認した後に、その Node の定義が承認要求を持たないときに返す `UnauthorizedApprovalTarget` の本文（`src-tauri/src/domain/workflow/entities/workflow_execution/mod.rs:2119-2124` の `"node does not declare completion: approval"`）を、廃止した文字列形式 `completion: approval` を引用せず、承認要求が未宣言であることを変更後の受理形 `require: approval` と矛盾しない形で示す本文に変える。変更後も同じ本文が local API の 403 応答と CLI の入力エラーへ渡る。根拠: Thread `eadbdff6-bc67-49b6-aea5-642c905b801a`、R-004「文字列形式の `completion: approval` と `completion: auto` は load 時に Error Diagnostic になり、その定義は load されない」、B-004。ルート: 委任（具体的な文字列は Impl が選ぶ）。変更対象は当該本文の文字列に限る。

## 固定するルート

- design-01「固定するルート」の2件（`NodeCompletion` 型を「要求の有無」に変える／Lua は `completion = { require = r.completion.approval }` の table 形だけを受理する）は今周も維持する。解除・変更はない。
- Thread `eadbdff6` の本文について人間が固定したのは文言の制約だけである。制約は「廃止した文字列形式 `completion: approval` を引用せず、承認要求が未宣言であることを変更後の受理形 `require: approval` と矛盾しない形で示す」こと。変更対象は当該本文の文字列に限る。具体的な文字列は委任する。理由: 制約は R-004 から直接導け、文字列の選択は design-01 が Diagnostic・UI の文言を委任した粒度と同じ扱いにする。他の表面の語は、WORKFLOW.md と workflow 定義詳細の badge が `require: approval`、設定画面の説明文が `completion.require: approval` である。

## 変えないもの

- 承認の実行時経路（Approve / Reject の操作、Approval auto-approve 設定、WaitingApproval の状態遷移、承認事実の記録）と、各 Node 種別の本来の完了条件（design-01「変えないもの」の維持）。今周は特に、承認対象の検証の順序（WaitingApproval の検査の後に定義の承認要求を検査する）、返す error の種別 `UnauthorizedApprovalTarget`、その本文を local API の 403 応答と CLI の入力エラーへ渡す変換を変えない。理由: 人間の決定により、変更対象は本文の文字列に限る。
- 旧形式（文字列 `completion`）の解釈経路と自動移行を、YAML / Lua の loader にも定義 snapshot の serde 経路にも追加しない（design-01「変えないもの」の維持）。
- Thread `2553a4d0`（domain の `NodeCompletion` に対する serde 実装を adaptor の `completion_wire.rs` に置く配置）は変えない。理由: blocking でなく、同じ構造への #1731 Thread `0baeb041` を人間が定義全体の wire 境界整理として本 Issue の範囲外と判断済みで、completion 型だけの部分修正は述語・失敗方針との配置の不揃いを新たに作る。
- Thread `c422e0d7`（Command 完了シナリオの新規テストが `workflow_host.rs` の inline test module にある配置）は変えない。理由: blocking でなく、新規テストは派生点から存在する module の既存テストと同じ構成に従い R-002 / R-003 の検証内容を満たしている。根本対応は module 全体の配置整理で本 Issue の範囲を超える。

## 未確定・リスク

なし
