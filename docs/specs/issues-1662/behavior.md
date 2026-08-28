## B-001: workflow 実行ルート行の表示名

GIVEN workflow 定義から起動された workflow 実行が Workspace ツリーに存在する
WHEN 利用者が Workspace ツリーを表示する
THEN その実行を表すルート行の表示名は、その実行の workflow 名である

## B-002: ルート行の Node 種別に依らない表示名

GIVEN workflow 実行のルート行にあたる Node の種別が Sequence、Fanout、Session、Command のいずれかである
WHEN 利用者が Workspace ツリーを表示する
THEN Node の種別に依らず、ルート行の表示名はその実行の workflow 名である

## B-003: 並存する複数実行の判別

GIVEN 同一 workspace に workflow 名の異なる複数の workflow 実行が同時に存在する
WHEN 利用者が Workspace ツリーを表示する
THEN 各ルート行の表示名は、それぞれの実行の workflow 名である
AND 各ルート行の表示名は互いに異なり、行ごとにどの workflow の実行かを判別できる

## B-004: 実行中の表示名とアーカイブ済み履歴の名前の一致

GIVEN workflow 実行がルート行として Workspace ツリーに表示されている
WHEN その実行がアーカイブされ、利用者がアーカイブ済み workflow 履歴を表示する
THEN 履歴に表示されるその実行の名前は、アーカイブ前に同じ実行のルート行へ表示されていた名前と一致する

## B-005: 単独 AgentSession のルート行表示名の維持

GIVEN workflow を介さずに起動された単独 AgentSession の実行木が Workspace ツリーに存在する
WHEN 利用者が Workspace ツリーを表示する
THEN その実行木のルート行の表示名は、本変更の前と同じである

## B-006: ルート行以外の行の表示名の維持

GIVEN workflow 実行の実行木に、ルート行以外の行（子 Sequence、Fanout、Session Node、Command Node）が存在する
WHEN 利用者が Workspace ツリーを表示する
THEN ルート行以外の各行の表示名は、本変更の前と同じである

## B-007: Node 詳細に表示される名前の維持

GIVEN workflow 実行の実行木の行が Workspace ツリーに表示されている
WHEN 利用者がルート行または他の行を選択して Node 詳細を表示する
THEN Node 詳細に表示される名前は、本変更の前と同じである

## B-008: ルート行の識別子、状態表示、workflow 操作可否の維持

GIVEN workflow 実行のルート行が Workspace ツリーに表示されている
WHEN 利用者がルート行を参照し、workflow 操作を行う
THEN ルート行の識別子は、本変更の前と同じである
AND ルート行の状態表示は、本変更の前と同じである
AND 停止、中止、再開、アーカイブの各 workflow 操作の可否は、本変更の前と同じである

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-002 |
| R-003 | B-003 |
| R-004 | B-004 |
| R-005 | B-005 |
| R-006 | B-006 |
| R-007 | B-007 |
| R-008 | B-008 |
