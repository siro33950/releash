## B-001: 繰り返し処理の駆動の置き場所

GIVEN daemon の 5 つの繰り返し処理（購読の worker、terminal の読み直し、launch の記録の保持期限、terminal の checkpoint の書き出し、repository の scan の worker）がある
WHEN それぞれの task の起動、きっかけを待つ `loop`、時刻の出どころを読む
THEN usecase の中に task の起動、きっかけを待つ `loop`、`sleep`・interval・遅延が無い
AND 時刻は infrastructure の駆動部だけが作っている
AND task の起動ときっかけを待つ `loop` は controller にあり、controller はきっかけごとに Usecase を呼んでいる

## B-002: 判断の置き場所

GIVEN 5 つの繰り返し処理の controller がある
WHEN controller の中身を読む
THEN 業務の判断（対象が知らせの影響を受けるか、外部の情報を取り直すか、作り直しを続けるか、保持期限が来た記録を消すか、scan を続けるか）を controller が持たず、usecase または domain に問うている
AND 今購読されている対象の集合を controller が別に持っていない

## B-003: 購読の対象が変化の知らせで読み直される

GIVEN ある対象を購読している client がいる
WHEN その対象に影響する変化の知らせが続けて届く
THEN 対象は読み直され、新しい状態が client へ配信される
AND 溜まった知らせは 1 回の読み取りにまとめられる

## B-004: 外部の情報の定期の取り直し

GIVEN 外部の情報を含む対象を購読している client がいる
WHEN 外部の情報の取り直しの間隔（`CacheTtl::EXTERNAL_INFORMATION`）が経つ
THEN 外部の情報が取り直され、対象が読み直されて配信される

## B-005: notify_and_wait が読み取りの終わりを待つ

GIVEN ある対象を購読している client がいる
WHEN その対象を指定して `notify_and_wait` が呼ばれる
THEN `notify_and_wait` は、その対象の読み直しと配信が終わってから戻る

## B-006: 購読が無くなった対象の読み直しが止まる

GIVEN ある対象の購読者が全員いなくなった
WHEN その対象に影響する変化の知らせが届く、または取り直しの間隔が経つ
THEN その対象は読み直されず、配信されない

## B-007: terminal の読み直し

GIVEN terminal の対象を購読している client がいて、送り待ちが溢れた
WHEN 読み直しが要求される
THEN 対象ごとに作り直しの処理は 1 つだけ動き、作り直しの記録が空になるまで snapshot を作り直して配信する
AND 読み直しに失敗したら失敗を配信して終わる
AND 購読が無くなった対象の作り直しは止まる

## B-008: launch の記録の保持期限

GIVEN workflow node の launch の記録が `Activated` になった
WHEN 300 秒が経つ
THEN その記録は消える

## B-009: terminal の checkpoint の書き出し

GIVEN ある session が dirty になった
WHEN 250ms が経つ
THEN その session の checkpoint が flush される
AND 待っている間に再び dirty になっていれば、さらに 250ms 後に再び flush される

## B-010: repository の scan の worker

GIVEN repository の状態を監視している
WHEN scan の知らせが続けて届く
THEN debounce（300ms）を待った後、溜まった理由をまとめて 1 回 scan する
AND shutdown になった後は scan しない

## B-011: テストで時刻を差し替えられる

GIVEN 5 つの繰り返し処理のテストがある
WHEN テストが、駆動部の作る時刻のきっかけの代わりに偽のきっかけを渡す
THEN 実際の時間を待たずに、偽のきっかけで繰り返し処理を動かせる

## B-012: 要らなくなった時刻の口が無い

GIVEN この変更が入っている
WHEN 購読の Usecase と repository の状態の trait を読む
THEN `SubscriptionTimer` と `adaptor/gateway/subscription_timer.rs` は存在しない
AND `RepositoryStateWorkerRuntime` は `sleep` と `spawn_worker` を持たない

## B-013: 購読の開始の直後の変化を取りこぼさない

GIVEN client がある対象の購読を始めた
WHEN 最初の値ができる前に、その対象に影響する変化の知らせが届く
THEN 最初の値の後に、その変化を反映した値が client へ配信される

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-001 |
| R-003 | B-002 |
| R-004 | B-003, B-004, B-005, B-006, B-013 |
| R-005 | B-007 |
| R-006 | B-008 |
| R-007 | B-009 |
| R-008 | B-010 |
| R-009 | B-011 |
| R-010 | B-012 |
