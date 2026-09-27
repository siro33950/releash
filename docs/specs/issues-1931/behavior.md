## B-001: 配信の仕組みの置き場

状態の配信の仕組み（版、送り待ち、bookmark、overflow の判定、続きからの再開、次に送る要素の取り出し、送る量の制御）は domain と usecase に無い。presenter と infrastructure にある。送り待ちと送る量の制御は infrastructure にある。

## B-002: usecase が外へ出す口

usecase が状態と結果を外へ出す口は Output Boundary の trait だけである。その実装は adaptor/presenter にある。usecase から配信の仕組みの実体と転送の形への参照は無い。Output Boundary の引数は Output Data である。

## B-003: domain が定義する trait の種類

domain が定義する trait は、Repository と、外部世界を必要とするドメインサービスの 2 種類だけである。画面へ届ける口は domain に無い。

## B-004: 転送の形に変える場所

Output Data を転送の形（Connect のメッセージ、HTTP local API のレスポンス、push のメッセージ）に変えるのは presenter だけである。controller と gateway と infrastructure では転送の形が組み立てられていない。

## B-005: 転送のメッセージ型の置き場

`src-tauri/src/adaptor/protocol/` は存在しない。そこにあった部品は、規約の部品の一覧のいずれかの部品として置かれている。

## B-006: HTTP local API の型の置き場

HTTP local API のリクエスト型は `src-tauri/src/adaptor/controller/api/protocol.rs` にある。レスポンスの型と、Output Data からレスポンスへの変換は presenter にある。usecase の Output Data は転送の都合を持たない。

## B-007: 購読の手順の置き場

購読の開始・停止・再読み取り・watch の調整の手順は usecase の Usecase にある。読み取りは usecase の trait を通して行われる。

## B-008: 画面が受け取る購読のメッセージ

GIVEN 画面が状態の購読を開始している
WHEN daemon が状態の変化を配信する
THEN 画面が受け取るメッセージの種類、フィールド、版の値は、この変更の前と同じである

## B-009: CLI が受け取る応答

GIVEN CLI が HTTP local API を呼ぶ
WHEN 応答を受け取る
THEN JSON のフィールド名と形は、この変更の前と同じである
AND 受け取る HTTP status は、この変更の前と同じである

## B-010: 続きからの再開

GIVEN 画面が、前に受け取った版を指定して購読を開始し直す
WHEN その版から続きを配信できる
THEN 画面が受け取るのは、その版より後ろの変化だけである
AND snapshot は受け取らない

## B-011: 続きから再開できないときの配信

GIVEN 画面が、前に受け取った版を指定して購読を開始し直す
WHEN その版から続きを配信できない
THEN 画面が受け取るのは今の snapshot である

## B-012: bookmark

GIVEN 購読に送るものが無い状態が続く
WHEN bookmark の間隔が経過する
THEN 画面は今の版を持つ bookmark を受け取る

## B-013: terminal の送る量の制御

GIVEN 画面が terminal の出力を購読している
WHEN 画面が処理していない出力の量が上限を超える
THEN provider の出力は止まる
AND 画面の処理の報告により未処理の量が下限を下回ると、provider の出力は再開する

## B-014: push

GIVEN 画面が push を購読している
WHEN daemon が file-change、git-status-changed、または review-comments-changed を送る
THEN 画面が受け取る内容は、この変更の前と同じである
AND 画面が push を取りこぼしたとき、受け取るのは resync である

## B-015: 使われなくなるコードの不在

この変更で使われなくなったコードは残らない。

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
| R-008 | B-008, B-009 |
| R-009 | B-010, B-011, B-012 |
| R-010 | B-013 |
| R-011 | B-014 |
| R-012 | B-015 |
