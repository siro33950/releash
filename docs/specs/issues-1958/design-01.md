# Design 01

## 開始状態

初回。基準は `main` の `9524a6fa`（`feat/issues/1958` の派生点）で、変更前の状態は requirements.md の Current Behavior に記録している。閉じた Thread はない。

## 変える部分

- 表示状態の値を3値にする: `WorkspaceNodeStatusClassification` と proto の `WorkspaceStatusClassification` から、Session が紐づく前を表す独立した値をなくす。残りは黄・青・緑の3値にする。根拠: R-001「失敗の赤と、Session が紐づく前の灰は、独立した値として存在しない」。ルート: ツリー DTO と proto にはこの3値だけを載せる（ISSUE の方針）。値の名前と proto の番号の扱いは委任。
- Session が出す色の導出: agent の状態だけから、紐づく前・動作中＝青、回答待ち＝黄、Stop した状態＝緑を出す。Node の状態は見ない。根拠: R-002。ルート: 下の「固定するルート」の domain の一つの関数。
- Node が出す色の導出: 承認待ち・失敗＝黄、agent の作業を待っているのに agent が止まっている（Stop したが Submit がない、プロセスが消えた）＝黄、Command 実行中＝青、完了・中断＝緑、Delegate の親が子の完了を待っている間は色を出さない。根拠: R-003。ルート: 下の「固定するルート」の domain の一つの関数。
- Delegate の親が子の完了を待っている段階を、ツリーの導出に届ける: 今は `#[cfg(test)]` の判定（`delegate_waits_for_child`）を実行経路で使える状態にし、その事実を Workspace ツリーの導出へ渡す。根拠: R-003「Delegate の親が子の完了を待っている間は、Node 自身は色を出さない」、B-012、B-013。ルート: 委任。
- 行の色の集約: Node が出す色・紐づく Session が出す色・子の行の色のうち最も重いもの（黄 ＞ 青 ＞ 緑）を採る。Session・Sequence・Fanout で規則を分けない。今の Sequence・Fanout だけの集約、Session と Command が裏で起きた失敗だけを子から受け取る特別扱い、子がすべて紐づく前なら親も紐づく前にする規則は、この一つの規則に置き換える。根拠: R-004。ルート: 下の「固定するルート」の domain の一つの関数。
- Workflow の外で新しく起動する Session の Node を、完了した状態で起動する。根拠: R-005、B-015。ルート: 委任。
- ツリー行のアイコン: Session が紐づく前の行に出していた灰色の回転するアイコンの分岐を削除し、3値の色だけで表示する。根拠: R-001。ルート: 委任。
- Session Node の詳細のヘッダーの状態アイコンを削除する。根拠: R-006。ルート: `src/components/panels/NodeContentView/NodeContentView.tsx` の `WorkflowNodeStatusIcon` を削除する（ISSUE の方針）。
- ヘッダーのアイコンの削除で使われなくなるものを削除する: 詳細 DTO の `statusClassification`（proto の `WorkspaceNodeDetailDto` の該当フィールドと usecase の DTO）と、`WorkflowNodeStatusIcon` の部品。根拠: R-006 の変更で参照元がなくなる。ルート: 委任。

## 固定するルート

- 事実から表示状態を導出する処理は、domain の一つの関数に置く（ISSUE の方針）。Session が出す色、Node が出す色、行の色の集約は、すべてこの関数の側で決める。client は受け取った3値を色に対応付けるだけにする。

## 変えないもの

- すでに記録されている Workflow の外の Session で、Node が Submit を待ったまま完了していないものは、記録を移行しない。記録どおり Submit 待ちの Node として扱い、この Design の Node が出す色の規則をそのまま当てはめる。人が「新しく起動するものだけを完了状態で起動する」と決めたため。

## 未確定・リスク

- Workflow の外の Session の Node が完了した後も、resume・archive・hook からの Stop と Submit の受信・activity の反映が今までどおり動くことは、コードを読んでいない。今でも Submit と Stop がそろった後にはこの状態になる（`src-tauri/src/domain/workflow/entities/workflow_execution/mod.rs:2755-2760`）ので、既存の経路にあると想定している。ただし起動時点から完了している場合は通っていない経路がありうる。想定が外れると、R-005 と R-002 を満たせない。
