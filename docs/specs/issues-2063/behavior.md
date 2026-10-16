## B-001: 開く経路によらないタブの同定

GIVEN ある Session のタブが開いている
WHEN 利用者が一覧、New Session の結果、または History の結果から同じ Session を開く
THEN 既存の同じ Session タブが選択される
AND 同じ Session のタブは追加されない

## B-002: 保存したタブの復元

GIVEN Session タブを含むレイアウトが保存されている
WHEN そのレイアウトを復元する
THEN Session タブは保存前と同じ AgentSession を対象とする

## B-003: Session の ID による購読

GIVEN 対象の AgentSession の ID がある
WHEN client がその ID だけを指定してタブに必要な情報を購読する
THEN 名前、状態、操作可否を受け取れる
AND Root 一覧との突き合わせなしでタブに反映できる

## B-004: タブ名の表示と更新

GIVEN 一覧、New Session、History、または保存したレイアウトから Session タブを開いている
WHEN 購読から Session 名または変更後の Session 名が届く
THEN タブ名は届いた Session 名になる

## B-005: 状態の色

GIVEN Session タブが開いている
WHEN 購読から Session の状態の分類が届く
THEN タブのアイコンは、一覧と同じ分類の色になる
AND 人の番は黄、システムの番は青、誰の番でもない状態は緑になる

## B-006: タブからの Rename

GIVEN Rename できる Session のタブが開いている
WHEN 利用者がタブ部分から Rename を実行して名前を変更する
THEN タブ名と一覧の Session 名に変更が反映される

## B-007: 中止可能な実行の Archive

GIVEN 中止可能な実行に属する単独 Session のタブが開いている
WHEN 利用者がタブ部分から Archive を選ぶ
THEN 実行を中止して Archive することの確認が表示される

## B-008: Archive の確認への了承

GIVEN 単独 Session のタブから Archive を選び、実行を中止して Archive することの確認が表示されている
WHEN 利用者が了承する
THEN 実行は中止され、Archive される

## B-009: Archive の取りやめ

GIVEN 単独 Session のタブから Archive を選び、確認が表示されている
WHEN 利用者が取りやめる
THEN その操作による実行の中止と Archive は行われない

## B-010: 中止可能でない実行の Archive

GIVEN 中止可能でない実行に属する単独 Session のタブが開いている
WHEN 利用者がタブ部分から Archive を選ぶ
THEN 確認なしで Archive される

## B-011: Workflow 内の Session の Archive

GIVEN Workflow 内の Session タブが開いている
WHEN 利用者がタブ部分の操作を開く
THEN そのタブから Archive を実行できない

## B-012: 判定不能と Archive 済みの表示

GIVEN Session タブが開いている
WHEN サーバが Session を開けるか判定できない状態、または Archive 済みの状態を配る
THEN TUI の代わりに、判定できないこと、または Archive 済みであることを表示する

## 要件IDとBehavior IDの対応表

| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001, B-002 |
| R-002 | B-003 |
| R-003 | B-004 |
| R-004 | B-005 |
| R-005 | B-006 |
| R-006 | B-007, B-008, B-009, B-010, B-011 |
| R-007 | B-012 |
