# Design

## The actual design

### Architecture

#### 入力契約のownerと主要な変更対象

入力契約そのものは変更しない。CLI の strict parse、Workflow YAML の未知 field 拒否、現在の検証経路はいずれも現状の owner が保持する。変更するのは、拒否の理由を過去仕様の名前で分類している箇所と、過去仕様を名指しで固定しているテスト資産だけである。

| Path | 変更の要旨 |
| --- | --- |
| `src-tauri/src/adaptor/gateway/workflow/diagnostics.rs` | `check_allowed_fields` から旧 field 一覧を撤去し、許可 field 外の key を単一の code で拒否する。deserialize エラー分類から旧構文メッセージ判定を撤去する。許可 field が空で旧 field の名指しだけを目的とする fanout aggregate の検査を撤去する。kind 数、必須 field、kind 固有制約など、現在の意味上の shape Diagnostic は維持する。 |
| `src-tauri/src/cli/cli_test.rs`、`src-tauri/src/cli/output_test.rs`、`src-tauri/src/adaptor/gateway/workflow/{schema_contract_tests.rs,storage.rs,definition_repository.rs,builtin.rs,facet.rs,workflow_host/prompt_rendering.rs}` | 削除済みの具体名を拒否・不在確認する assertion と source scan を削除する。現在の正規入力を確認する assertion は維持し、未知入力の拒否と現在の検証契約は一般名を使って検証する。 |
| `src-tauri/src/adaptor/gateway/workflow/fixtures/invalid/` | `WFS005_legacy-*` 23件を invalid fixture 集合から削除する。未知 field の拒否を固定する fixture と、残る invalid fixture の code / stage 契約は変更しない。 |
| `src-tauri/src/cli/mod.rs`、`src-tauri/src/domain/workflow/value_objects/definition.rs`、`src-tauri/src/domain/workflow/services/contract_schema.rs` | 変更しない。clap の strict parse、`deny_unknown_fields`、schema subset 外 keyword の拒否をそのまま維持する。 |
| `docs/workflow-yaml-syntax.md`、`docs/workflow-engine-evolution-plan.md` | 変更しない。両文書が定義する unknown field rejection は現行仕様である。 |

根拠は、CLI ingress と dispatch を所有する `src-tauri/src/cli/mod.rs`、typed YAML grammar を所有する `src-tauri/src/domain/workflow/value_objects/definition.rs`、schema 解釈を所有する `src-tauri/src/domain/workflow/services/contract_schema.rs`、source から typed definition までの Diagnostic pipeline を所有する `src-tauri/src/adaptor/gateway/workflow/diagnostics.rs` および `storage.rs`、ならびに `docs/architecture/{DOMAIN,GATEWAY,TEST}.md` の既存責務境界である。

#### 検証境界

未知入力の拒否は、CLI では clap の argv parse、Workflow YAML では raw shape 検査と typed deserialize が担う。この 3 つの境界は変更しない。変更後も未知入力は typed model、WorkflowExecution、Artifact へ到達しない。Diagnostic の生成位置と stage も変更せず、拒否理由の分類だけが過去仕様から独立する。

### Interface

公開 CLI command、既知 option、positionals、出力、exit-code mapping は変更しない。未知 option、未知 subcommand、必須引数欠落、既知引数の型不正は、clap の現在の error のまま返す。

Workflow YAML の公開契約は変更しない。top-level、node、command / session / fanout kind block、session facets、rule、schema map の未知 field / keyword は引き続き拒否される。

外部から観測できる差分は Diagnostic code の集合だけである。旧構文を理由とする `WFS005` は発行されなくなり、未知 field / keyword はその由来によらず `WFS002` として拒否される。現在の既知 shape / value / reference / type / control-flow failure に対する Diagnostic code、stage、span の契約は維持する。

新しい公開 command / API / protocol、および新しい内部 trait は追加しない。

### Data Model

`WorkflowDefinition`、`SchemaDef`、CLI command enum の既存構造を維持する。unknown field / keyword / option 用の extension map や互換 record は追加しない。既存の Workflow YAML source 保存方式は変更せず、永続 version の追加も行わない。

### Database

該当なし。

### UI/UX

該当なし。

### Algorithm

raw shape 段階の許可 field 検査は、map の key が許可 field 列挙に含まれない場合に単一の code で Error Diagnostic を生成する。旧 field 一覧の参照と、それによる code / message の出し分けを行わない。許可 field 列挙が空になる検査は、親 map の許可 field 検査が同じ key を既に拒否するため置かない。

typed deserialize のエラー分類は、rule shape と kind block に関するメッセージを `WFS003`、YAML syntax に関するメッセージを `WFS001`、それ以外を `WFS002` とする。旧構文メッセージの判定を行わない。

CLI と Workflow YAML の parse そのもののアルゴリズムは変更しない。

### Infra

該当なし。

## Alternatives Considered

- 旧 field 一覧と `WFS005` を残し、message だけを一般化する案は採用しない。過去仕様の blacklist が validation に残り続け、R-001 / R-002 を満たさない。
- `WFS005_legacy-*` fixture を一般名へ改名して残す案は採用しない。fixture の内容は過去の具体構文そのものであり、改名しても過去仕様を拒否契約として固定し続ける。未知 field の拒否は一般名の fixture と test で固定できる。
- 未知 option / field を無視する案は採用しない。CLI では未知 option の値 token と既知 positional を token 表記だけで判別できず、未知 option を含む argv の解釈が曖昧になる。未知入力の拒否は、その曖昧性を持たない現行仕様である。
- 削除済み option / field 名だけを ingress で除外する案は採用しない。過去仕様の blacklist を別の場所へ移すことになり、R-001 を満たさない。

## Cross-cutting concerns

- 互換性: 入力契約を変更しないため、既知入力・未知入力ともに受理と拒否の判定は変更前と同じである。差分は Diagnostic code の分類だけであり、`WFS005` を返していた入力は `WFS002` を返す。
- 検証: B-004 / B-005 は、一般名の未知 option を含む argv と、対象となる各 YAML map に一般名の未知 field / keyword を追加した source が拒否されることを、CLI ingress test、deserialize test、Diagnostic contract test、storage test で確認する。B-002 は、未知 field を含む source が過去仕様由来かどうかによらず同一 code で拒否されることを Diagnostic contract test で確認する。B-006 は残存 valid / invalid fixture suite と既存の validation test で確認する。B-001 / B-003 は恒久的な blacklist test を追加せず、変更差分と対象 directory の一回の repository inspection で確認する。
- 性能: parse 経路と Diagnostic 生成の回数を変更しない。旧 field 一覧の線形探索が減るだけである。

## Risks

該当なし。
