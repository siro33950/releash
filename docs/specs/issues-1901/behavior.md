## B-001: 実行の状態の購読

GIVEN workflow の実行が存在する
WHEN operator の token でその実行の ID を指定して実行の状態を購読する
THEN 最初の状態として、その実行の ID・workflow 名・状態・現在の Node・token の使用量を含む実行の状態が配信される

## B-002: 存在しない実行の状態の購読

GIVEN 指定する実行の ID の実行が存在しない
WHEN その ID を指定して実行の状態を購読する
THEN 最初の状態として「無い」が配信され、読み取りの失敗にはならない

## B-003: 提出済みの output の購読

GIVEN 実行の Node に output が提出されている
WHEN 実行の ID と Node 名を指定して output を購読する
THEN 最初の状態として、提出済みであることと、その contract・値・時刻が配信される

## B-004: 未提出・実行なしの output の購読

GIVEN 実行の定義にある Node に output がまだ提出されていない
WHEN 実行の ID と Node 名を指定して output を購読する
THEN 最初の状態として「未提出」が配信される
AND 存在しない実行の ID を指定したときは「無い」が配信される

## B-005: 定義に無い Node の output の購読

GIVEN 実行の定義に無い Node 名
WHEN 実行の ID とその Node 名を指定して output を購読する
THEN 読み取りの失敗が配信される

## B-006: worktree を指定しない output の提出

GIVEN Artifact を待っている node-execution が存在する
WHEN operator の token で、worktree を指定せず node-execution の ID と Artifact を指定して output を提出する
THEN 提出が受け付けられ、その node-execution に Artifact が記録される

## B-007: directory を指定した診断

GIVEN workflow の定義を置いた directory が存在する
WHEN operator の token でその directory の絶対 path を指定して診断を呼ぶ
THEN その directory の定義に対する診断の結果が返る
AND 相対 path を指定したときは `InvalidArgument` になる
AND 存在しない directory を指定したときは `NotFound` になる

## B-008: Session を指定した review の一覧の購読

GIVEN Session の agent が作成した Thread と、人が作成した Thread が、その Session の worktree にある
WHEN その Session の ID と author=self を指定して review の一覧を購読する
THEN 最初の状態として、その Session の agent が作成した Thread だけが配信される
AND state・file・thread を指定したときは、それに合う Thread だけが配信される
AND 存在しない Session の ID を指定したときは「無い」が配信される

## B-009: worktree の path を指定した review の一覧の購読

GIVEN Node の隔離 worktree が、ある workspace の worktree に属し、その workspace の worktree に Thread がある
WHEN 隔離 worktree の path と state=open を指定して review の一覧を購読する
THEN 最初の状態として、その workspace の worktree の open な Thread が配信される
AND author または unread を指定した購読は受け付けられない

## B-010: Session を指定した Thread とその履歴の購読

GIVEN open でない Session の worktree に、Comment の追記と解決を経た Thread がある
WHEN その Session の ID と Thread の ID を指定して、Thread と履歴をそれぞれ購読する
THEN Thread の購読には、その Thread の状態と Comment が配信される
AND 履歴の購読には、作成・追記・解決の各記録が起きた順に配信される
AND 存在しない Thread の ID を指定したときは、どちらの購読にも「無い」が配信される

## B-011: Session を指定した review の作成・追記・解決

GIVEN open な Session がある
WHEN operator の token でその Session の ID を指定して、Thread を作成し、Comment を追記し、解決する
THEN それぞれの応答に変更後の Thread が返り、作成者・追記者・解決者はその Session の agent になる
AND 画面の review の一覧の購読にも、その変更が配信される

## B-012: open でない Session・存在しない Session での review の変更

GIVEN open でない Session がある
WHEN その Session の ID を指定して Thread を作成する
THEN `FailedPrecondition` になり、Thread は作られない
AND 存在しない Session の ID を指定したときは `NotFound` になる

## B-013: provider の信号の受け取り

GIVEN provider の agent が、サーバが発行した slot・binding・capability で起動されている
WHEN hook の token で、SessionStart の生の payload を指定して provider の信号を送る
THEN 「適用」が返り、Session の開始が記録される
AND 同じ信号をもう一度送ったときは「重複」が返る

## B-014: subagent の payload と不正な payload

GIVEN provider の agent が起動されている
WHEN hook の token で、Claude の subagent の payload を送る
THEN 「無視」が返り、何も記録されない
AND 解釈できない payload、または 65,536 byte を超える payload を送ったときは `InvalidArgument` になる

## B-015: ClientService に定義されていない method

GIVEN ClientService に定義されていない method（削除した `WorkflowGetOutput`・`WorkflowValidateOutput` を含む）
WHEN 正しい operator の token または hook の token でその method を呼ぶ
THEN `Unimplemented` になり、`PermissionDenied` にはならない

## B-016: token の scope

GIVEN サーバが起動している
WHEN hook の token で `GetServerInfo` を呼ぶ
THEN サーバの情報が返る
AND hook の token で operator の RPC を呼んだときは `PermissionDenied` になる
AND operator の token で `ReceiveProviderSignal` を呼んだときは `PermissionDenied` になる
AND どの token とも一致しない token で呼んだときは `Unauthenticated` になる

## B-017: scope の付け忘れ

WHEN ClientService の RPC のどれかが、受け付ける scope を持たないか `SCOPE_UNSPECIFIED` を含む、または同じ scope を重複して持つ
THEN サーバのビルドが失敗する

## B-018: hook の token の受け渡しと失効

GIVEN サーバが起動している
WHEN サーバが provider の agent を起動する
THEN agent のプロセスの env `RELEASH_PROVIDER_LIFECYCLE_TOKEN` に hook の token が入る
AND hook の token は data dir のどのファイルにも書かれない
AND サーバの停止の後は、その token の要求は `Unauthenticated` になる

## B-019: operator の token と発見ファイル

GIVEN サーバが起動している
WHEN `client-api.json` の token で operator の RPC を呼ぶ
THEN 受け付けられる
AND `client-api.json` と `local-api.json` の名前と項目は今と同じである

## B-020: Origin の扱い

GIVEN 正しい operator の token
WHEN Origin を付けずに operator の RPC を呼ぶ
THEN 受け付けられ、応答に CORS の header は付かない
AND allowlist にある Origin を付けたときは受け付けられ、応答に CORS の header が付く
AND allowlist に無い Origin を付けたときは 403 になる

## B-022: proto の検査

GIVEN proto を変更する PR
WHEN CI が走る
THEN `buf lint`（STANDARD。既存の違反として外した規則とファイルの組を除く）と、PR の base と比べた `buf breaking`（WIRE_JSON）の結果で検査が決まる
AND PR に `buf skip breaking` ラベルが付いているときは breaking の検査が行われない

## B-023: 足した proto の要素の命名

GIVEN この開発で足した RPC・message・enum
WHEN proto を読む
THEN RPC は `<Rpc>Request`／`<Rpc>Response` を RPC ごとに持ち、enum の値は enum 名を接頭辞とする UPPER_SNAKE_CASE で 0 番が `_UNSPECIFIED` である
AND 削除したフィールドと oneof の項目は、番号と名前が reserved になっている

## B-024: 画面と既存の入口の維持

GIVEN 画面が `review-threads` を購読し、`CreateReviewThread` などで review を変更している
WHEN この開発の後に同じ操作をする
THEN 引数・応答・書き手（人）は今と同じである
AND HTTP `/v1` の入口は master token で今と同じに応答し、CLI の `--json` の出力の形は今と同じである

## B-025: AGENTS.md の記載

WHEN AGENTS.md を読む
THEN 「hook の token は hook のプロセスにしか渡さない」と書かれ、「local API の master token を renderer JS へ渡さない」は無い
AND 「ビルド・テスト・Lint」に、CI と同じ `buf lint` と `buf breaking` のコマンドと、`buf skip breaking` ラベルを効かせる手順がある

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001, B-002 |
| R-002 | B-003, B-004, B-005 |
| R-003 | B-006 |
| R-004 | B-007 |
| R-005 | B-008 |
| R-006 | B-009 |
| R-007 | B-010 |
| R-008 | B-011, B-012 |
| R-009 | B-013, B-014 |
| R-010 | B-015 |
| R-011 | B-016, B-017 |
| R-012 | B-018 |
| R-013 | B-019 |
| R-014 | B-020 |
| R-015 | B-022, B-023 |
| R-016 | B-024 |
| R-017 | B-025 |
