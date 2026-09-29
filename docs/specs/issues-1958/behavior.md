## B-001: 紐づく前の Session は青

GIVEN Workflow の Session Node が起動し、Session がまだ紐づいていない
WHEN 開発者が Workspace ツリーを見る
THEN その行は青で表示される

## B-002: 動作中の Session は青

GIVEN Workflow の Session Node に紐づく Session が動作中である
AND その Node は承認待ちでも失敗でもない
WHEN 開発者が Workspace ツリーを見る
THEN その行は青で表示される

## B-003: 回答待ちの Session は黄

GIVEN Session Node に紐づく Session が、質問または許可への回答を待っている
WHEN 開発者が Workspace ツリーを見る
THEN その行は黄で表示される

## B-004: Stop して Node が完了した Session は緑

GIVEN Session Node が Submit と Stop の両方を受け取って完了している
AND 紐づく Session は Stop した状態である
AND 子の行はない
WHEN 開発者が Workspace ツリーを見る
THEN その行は緑で表示される

## B-005: 承認待ちの Node は黄

GIVEN Node が承認を待っている
WHEN 開発者が Workspace ツリーを見る
THEN その行は黄で表示される

## B-006: 裏で起きた失敗は黄で親にも上がる

GIVEN Sequence の子の Node に、裏で起きた失敗が記録されている
WHEN 開発者が Workspace ツリーを見る
THEN その Node の行は黄で表示される
AND 親の Sequence の行も黄で表示される

## B-007: プロセスが消えた実行中の Command は黄

GIVEN Command Node が実行中のまま、プロセスが消えている
WHEN 開発者が Workspace ツリーを見る
THEN その行は黄で表示される

## B-008: Stop したが Submit がない Session Node は黄

GIVEN Workflow の Session Node が Stop を受け取り、Submit を受け取っていない
AND その Node は Delegate の子の完了を待っていない
WHEN 開発者が Workspace ツリーを見る
THEN その行は黄で表示される

## B-009: 作業待ちの Session Node でプロセスが消えると黄

GIVEN Workflow の Session Node が完了・中断していない
AND 紐づく Session のプロセスが消えている
WHEN 開発者が Workspace ツリーを見る
THEN その行は黄で表示される

## B-010: 実行中の Command は青

GIVEN Command Node がプロセスを持って実行中である
WHEN 開発者が Workspace ツリーを見る
THEN その行は青で表示される

## B-011: 中断した Node は緑

GIVEN Node が中断されている
AND 紐づく Session は Stop した状態である
AND 子の行はない
WHEN 開発者が Workspace ツリーを見る
THEN その行は緑で表示される

## B-012: 子の完了を待つ Delegate の親は、子が動作中なら青

GIVEN Delegate を持つ Session Node が Artifact を提出して Stop し、子の完了を待っている
AND 子の Session が動作中である
WHEN 開発者が Workspace ツリーを見る
THEN 親の行は青で表示される

## B-013: 子の完了を待つ Delegate の親は、子が回答待ちなら黄

GIVEN Delegate を持つ Session Node が Artifact を提出して Stop し、子の完了を待っている
AND 子の Session が質問または許可への回答を待っている
WHEN 開発者が Workspace ツリーを見る
THEN 親の行は黄で表示される

## B-014: Sequence と Fanout は最も重い子の色になる

GIVEN Sequence または Fanout の子に、青の行と黄の行がある
WHEN 開発者が Workspace ツリーを見る
THEN その Sequence または Fanout の行は黄で表示される

## B-015: Workflow の外の Session は Node が完了した状態で起動する

GIVEN 開発者が Workflow の外で Session を新しく起動した
WHEN 開発者がその Session Node の状態を見る
THEN Node は完了している

## B-016: Stop した Workflow の外の Session は緑

GIVEN Workflow の外で新しく起動した Session が、Submit を受け取らずに Stop した
WHEN 開発者が Workspace ツリーを見る
THEN その行は緑で表示される

## B-017: プロセスが消えた Workflow の外の Session は緑

GIVEN Workflow の外で新しく起動した Session のプロセスが消えている
WHEN 開発者が Workspace ツリーを見る
THEN その行は緑で表示される

## B-018: 表示状態は3値だけ

GIVEN Workspace ツリーに、紐づく前の Session、失敗した Node、完了した Node がある
WHEN client が Workspace ツリーを受け取る
THEN 各行の表示状態は、黄・青・緑を表す3値のどれかである

## B-019: Session Node の詳細のヘッダーに状態アイコンがない

GIVEN 開発者が Session Node を選んでいる
WHEN 詳細が表示される
THEN ヘッダーに状態アイコンは表示されない

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001, B-006, B-018 |
| R-002 | B-001, B-002, B-003, B-004 |
| R-003 | B-005, B-006, B-007, B-008, B-009, B-010, B-011, B-012 |
| R-004 | B-006, B-012, B-013, B-014 |
| R-005 | B-015, B-016, B-017 |
| R-006 | B-019 |
