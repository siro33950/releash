## B-001: 開く経路によらないタブの同定

GIVEN ある Session のタブが開いている
WHEN 利用者が一覧、New Session の結果、または History の結果から同じ Session を開く
THEN 既存の同じ Session タブが選択される
AND 同じ Session のタブは追加されない

## B-002: 保存したタブの復元

GIVEN 会話が紐付いた Session タブを含むレイアウトが保存されている
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

## B-013: 一部の Session の情報取得に失敗した場合の復元

GIVEN Session タブを含むレイアウトが保存されている
WHEN 復元中に一部の Session の情報取得に失敗する
THEN ほかのタブと配置は復元される
AND 取得に失敗したタブの参照は保持される
AND 取得に失敗したことが利用者に表示される

## B-014: 会話の準備中にタブを開く

GIVEN 一覧に会話の準備中の Session がある
WHEN 利用者がその Session を開く
THEN タブが開き、準備中であることが表示される

## B-015: 会話の起動失敗の表示

GIVEN 会話が紐付く前の Session タブを開いている
WHEN 会話の起動に失敗した状態を受け取る
THEN そのタブに失敗の理由が表示される

## B-016: 準備完了後の会話の表示

GIVEN 会話が紐付く前の Session タブを開いている
WHEN その会話が利用可能になる
THEN 同じタブに会話の TUI が表示される

## B-017: 復旧後の起動エラー表示

GIVEN Session タブに会話を開く処理の失敗が表示されている
WHEN 会話を開く処理が成功するか、購読で会話が利用可能になったことを確認する
THEN 以前の起動エラーは表示されなくなる

## B-018: 閉じた復元エラーの継続

GIVEN 利用者がレイアウト復元時の取得失敗の表示を閉じ、同じ workspace を表示している
WHEN 同じ失敗が購読で継続して届く
THEN 閉じた失敗は再表示されない

## B-019: 新しい復元エラーの表示

GIVEN 利用者がレイアウト復元時の取得失敗の表示を閉じ、同じ workspace を表示している
WHEN 購読で新しい取得失敗が届く
THEN 新しい失敗が表示される

## B-020: 復元エラーの再発

GIVEN 利用者がレイアウト復元時の取得失敗の表示を閉じ、同じ workspace を表示している間に、その失敗の解消を購読で確認している
WHEN その失敗の再発が購読で届く
THEN 再発した失敗が表示される

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
| R-008 | B-013 |
| R-009 | B-014, B-015, B-016 |
| R-010 | B-017 |
| R-011 | B-018, B-019, B-020 |
