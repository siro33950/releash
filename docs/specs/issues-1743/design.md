# Design

## The actual design

### Architecture

#### Command 実行結果を node へ載せるかは集約が node の kind で決める

現行は、判定が値の形だけで行われる場所（`command_result_from_value`、`projection.rs:1-10`）と、その結果を無条件に node へ書く場所（`entities/mod.rs:943` の `node.command_result = result;`）が分かれ、kind との整合は書いた後の検査（`node_shape_is_valid` の `command_result.is_none()`、`entities/mod.rs:1053` / `1074` / `1086` / `1096`）でしか見ていない。検査に落ちると `WorkspaceTree::restore` / `WorkspaceTreeProjector::project` が失敗し、`invariant_query_error` を経て workspace 全体が `corrupt_stored_state` になる。

`command_result` を書く経路は上記の1箇所だけである。そこで、Command 実行結果を受理するかどうかの判断を集約のこの操作へ置き、node 自身の kind が `WorkspaceNodeKind::WorkflowCommand` のときだけ `command_result` を設定する。kind は先行する `NodeStarted` の適用（`apply_node_started`、`entities/mod.rs:463-468`）で確定しているため、fact に kind を運ばせる必要はない。

この置き方により、`node_shape_is_valid` の3つの `command_result.is_none()` は検査ではなく構築上の帰結になる。どんな fact 列を与えても Sequence / Fanout / Session の node が Command 実行結果を持つ木は作れず、R-003 が「fact を組み立てる側が毎回正しく分岐すること」に依存しなくなる。

`command_result_from_value` は値の形だけを見る関数のまま変えない（R-002）。この関数が返すのは node が持つ結果ではなく、Artifact の値から読み取れた **候補** であり、それを node が持てるかは集約が決める。kind の分岐を抽出側へ足さないのは、規則を一箇所に保つためである。

非 Command node で候補が落ちることによる情報の欠落は無い。Sequence の統合 map のキーがたまたま `exit_code` / `duration` / `stdout` / `stderr` に一致したことや、Session の Contract がその4つを宣言したことは、Command の実行結果ではない。

#### kind 不一致を error にしない

集約には、kind に紐づく fact を不一致の node へ当てたとき `WorkspaceTreeError::InvalidNode` で拒否する先例がある（`NodeActivityProjected` / `NodeSessionDisplayNameProjected`、`entities/mod.rs:889` / `905`）。artifact の適用では同じ形を採らない。

- あの2つの payload は Session node の形の必須要素（`activity.is_some()`）であり、生成側も kind で分岐しているため、不一致は fact 列が壊れていることを意味する。
- Command 実行結果は Command node でも省略され得る（R-002）。不一致を作る入力は Artifact の値であり、これは workflow の作者が決める。作者が決めた Artifact の形を読み出しの error にすることは R-003 が禁じている当のものである。

したがって、非 Command node に対しては候補を捨てて `command_result` を `None` のままにする。

同じ fact 系統で kind の分岐を持たない `NodeCommandPrepared`（`display_command`）は本件の対象にしない。`display_command` の出所は Command 実行が出す `CommandPrepared` fact であり、発生源の側で kind に縛られている。Artifact の値の形から導出されるのは Command 実行結果だけである。

#### `has_artifact` は kind と Command 実行結果の双方から独立に保つ

R-004。集約は artifact の適用で `has_artifact` を真にし（`entities/mod.rs:942`）、`runtime_snapshot_nodes` はその後 `runtime.artifact.is_some()` で上書きする（`projection.rs:195`）。どちらも kind にも候補の有無にも条件を付けない。今回の kind 分岐は `command_result` の代入だけに掛け、`has_artifact` の2つの代入とは分ける。

#### 主要な変更対象

| path | 担う変更 |
| --- | --- |
| `src-tauri/src/domain/workspace_tree/entities/mod.rs` | 集約が artifact fact を適用するとき、node の kind に応じて Command 実行結果を受理する |
| `src-tauri/src/domain/workspace_tree/value_objects/mod.rs` | `NodeArtifactProduced` の payload が「候補」であることを表す field 名にする |
| `src-tauri/src/domain/workspace_tree/projection.rs` | 上記 field 名への追随、および Command node / 非 Command node を対象にした単体テストの追加 |
| `src-tauri/src/adaptor/gateway/workspace_tree/repository_test.rs` | fact log 水準での workspace 全体読み出しのテスト追加 |

### Interface

外部から観測できる契約は変わらない。`WorkspaceNodeDetailDto` の `content` は現行も `WorkflowCommand` node にだけ `Command` variant を返し、それ以外は `Session` variant を返す（`query_service.rs:448-461`）。DTO、Tauri command、local API のいずれも形を変えない。本件が変えるのは「読み出しが成立するか」だけである。

内部境界: `WorkspaceStructureFact::NodeArtifactProduced` の payload の意味を「Artifact の値から読み取れた Command 実行結果の候補」に確定し、field 名をその意味にする。現行の `result` という名は「node の結果」と読めるため、集約側で無条件に代入する形へ戻りやすい。fact 型自体は読み出しごとに組み立てる in-memory の projection 入力であり、公開契約でも永続形式でもない（構築箇所は `runtime_snapshot_nodes` と `repository.rs:216` の `execution_summary_fact` の2つだけ）。

### Data Model

新しい record は無い。`WorkspaceTreeNode.command_result` の型と保持項目（exit code / duration / stdout / stderr）は変えない。`WorkspaceStructureFact` は永続化されず `node_events` にも現れないため、保存側の形は一切変わらない。R-005 は fact log への操作を必要とせず、保存済みの実行はそのままの fold から R-001〜R-004 を満たす木になる。

### Database

該当なし。新しい access path は要らない。

### UI/UX

該当なし。

### Algorithm

該当なし。判定は node の kind による分岐一つであり、方式の選択肢は無い。

### Infra

該当なし。

### 必要な検証

- B-002 は既存の `projection.rs` 単体テストが固定している。Command node に4項目を欠く Artifact を与え、`command_result` が `None` になることを見ている（`projection.rs:326` / `377`）。
- B-001 を固定するテストは無い。`command_result` が `Some` になることを確認するテストはリポジトリ内に存在せず、今回の変更は `command_result` の代入そのものに kind 分岐を掛けるため、Command 側の導出が落ちても既存テストは全て通る。`runtime_snapshot_nodes` の単体テストとして、Command node に4項目を揃えた Artifact を与え、`command_result` がその値になることを見るテストを足す。
- B-003 / B-004 / B-005 は `runtime_snapshot_nodes` の単体テストで固定する。Sequence / Fanout / Session の `RuntimeNodeExecution` に4つ揃った Artifact を与え、戻り値が `Ok`（= 集約の検証を通る）であることを見る。kind ごとに必要な付随 fact（Session の activity と表示名）は既存の生成経路がそのまま出すため、node の kind を変えるだけで作れる。
- 上と同じテストで、集約が所有する不変条件（非 Command node は `command_result` を持たない）も固定する。これは読み側 DTO に現れず（`query_service.rs:448-461` の kind 分岐）受入条件では判定できないため、ここでしか固定できない。
- B-006 / B-007 / B-009 は gateway の `repository_test.rs` の水準で固定する。ここは `WorkflowEvent` 列を store へ入れて fold する既存のテスト構成があり、同じ workspace に健全な実行木と当該 Artifact を持つ実行木を同居させられる。`workspace_tree_from_folded` 経由の workspace 読み出しと `load_node` の双方が `LocalEventQueryError::Corrupt` にならず、全実行木の node が読めることを見る。保存済み fact log を触らずに読めることがそのまま B-009 になる。
- B-008 の非 Command node の `has_artifact` は `runtime_snapshot_nodes` の単体水準に足す。

## Alternatives Considered

- **fact を組み立てる `runtime_snapshot_nodes` の側で kind により抽出を止める**: 変更は1箇所で済み、非 Command node での候補抽出も起きない。しかし集約は依然として kind 不一致の `command_result` を受理でき、受理した場合の結末は workspace 全体の corrupt のままである。#1743 が問題にしているのは「1 node の形の食い違いが workspace 全体を落とす」構造であり、規則を集約の外に置いたままではこの罠が残るため採らない。
- **fact が Artifact の値そのものを運び、集約が抽出まで行う**: 判断も抽出も集約に閉じるが、fact が node ごとに Artifact 全体の複製を持つことになる。Artifact は stdout / stderr を含んで大きくなり得るため、`AGENTS.md` が禁じる full-retention 経路を増やす。採らない。
- **kind 不一致を `WorkspaceTreeError::InvalidNode` で拒否する**: `NodeActivityProjected` と形は揃うが、拒否は `invariant_query_error` を経て `corrupt_stored_state` になり R-003 に反する。採らない。
- **`node_shape_is_valid` から `command_result.is_none()` を外す**: 構築で保証されるため冗長に見えるが、この検査は集約が宣言する node の形そのものであり、`command_result` を書く経路が将来増えたときに規則が残る唯一の場所になる。残す。

## Cross-cutting concerns

- 候補の抽出は Artifact の top-level 4 field だけを複製し、Artifact 全体は複製しない。非 Command node で候補が立つのは Artifact がその4つを揃えた形のときに限られ、複製された値は木の組み立て中に捨てられる。読み出しごとの一時的な複製量は現行の Command node と同じ order であり、新しい保持経路は増えない。
- 読み側の入口は Tauri command と loopback local API の双方が同じ query service と `SqliteWorkspaceTreeRepository` を通り、`tree_nodes` が両者の唯一の合流点である。入口ごとの追加作業は発生しない。
- `invariant_query_error` の log 出力と `corrupt_stored_state` への写しは変えない。本件で当該経路へ到達しなくなるだけである。

## Risks

- B-005 を fact log 水準で作れるかは、Session の Contract が Command 予約 field を宣言できるという読解（`validation.rs:1511-1519` の `if is_command` 分岐）に依存する。この読解はコードの確認のみで、実行による再現は行っていない。別経路が実際には宣言を拒否する場合、B-005 は `runtime_snapshot_nodes` へ直接 `RuntimeNodeExecution` を与える単体水準でしか固定できない。R-001 と R-003 の成立自体には影響しない。
