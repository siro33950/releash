# Design

## The actual design

### Architecture

#### permission の責務 owner と型境界

`session.permission` の語彙と受理規則は workflow domain が所有する。`src-tauri/src/domain/workflow/value_objects/definition.rs` に `SessionPermission` 値オブジェクトを置き、`SessionSpec.permission` を `Option<SessionPermission>` に変更する。値オブジェクトは `manual`、`auto`、`bypass`、`read-only` だけを表現し、文字列からの構築と serde 表現でも同じ4値だけを受理する。これは `docs/architecture/DOMAIN.md` の「一つの概念に一つの表現」と、`docs/glossary/DOMAIN.md` が Session と WorkflowDefinition の owner を workflow と定めていることに従う。

`src-tauri/src/adaptor/gateway/workflow/node_session_boundary.rs` の `WorkflowSessionLaunchConfig`、`src-tauri/src/usecase/agent_session/agent_session_launch.rs` の `WorkflowAgentSessionLaunchRequest`、`src-tauri/src/domain/agent_session/provider_launch.rs` の `ProviderLaunchOptions` は、permission を文字列へ戻さず同じ `SessionPermission` として受け渡す。既存の `WorkflowAgentSessionPort` は workflow NodeExecution と AgentSession 起動の境界を引き続き所有し、permission に関する判断や変換は持たない。

provider 固有の CLI 引数列への変換は `src-tauri/src/adaptor/gateway/provider_lifecycle/launch_spec.rs` だけが所有する。`ProviderLaunchSpec` は `ProviderKind` と `SessionPermission` の組み合わせを R-005 の引数列へ変換し、permission が `None` なら引数を追加しない。外部 CLI の語彙への変換を gateway に閉じる判断は `docs/architecture/GATEWAY.md` に従う。

#### WorkflowDefinition loader と Diagnostic

YAML の serde 境界と `src-tauri/src/adaptor/gateway/workflow/lua/mod.rs` の Lua builder は、どちらも `SessionPermission` の同じ構築規則を使う。provider 固有値や未知値を一時的な有効 `SessionSpec` として保持しない。

`src-tauri/src/adaptor/gateway/workflow/diagnostics.rs` は、この共通の不正 permission を既存の shape error `WFS002`、`parse_shape` stage、`permission` field として表現する。YAML は `YamlSpanMap`、Lua は builder 呼び出しの `LuaSourceLocation` を使って当該 field の位置を付ける。これにより入力形式ごとの parse 実装は異なっても、Diagnostic の分類と原因は同一になる。Diagnostic が workflow definition loader に属することは `docs/glossary/DOMAIN.md` の定義を維持する。

#### 定義、補完、正本文書の同期

次の変更を同じ契約変更として扱う。

- `workflows/01_author-spec.yml` から `workflows/06_handle-pr-review-manual.yml` までの builtin 8本は、58個の Session をすべて `permission: auto` にする。
- `workflows/examples/full-cycle-development.yml` は、builtin と同じ既存値である `acceptEdits` / `danger-full-access` を `auto`、確認なしを明示する `bypassPermissions` を `bypass`、codex の既定承認方式と書き込み範囲を組み合わせていた `workspace-write` を `manual` へ置き換え、定義 loader を通る完成形の例として維持する。
- `src-tauri/src/adaptor/gateway/workflow/lua/stubs.rs` は `ReleashPermission` を4つの文字列リテラルの union として公開し、`ReleashSessionOptions.permission` から参照する。
- `docs/glossary/WORKFLOW.md` は `model` の無変換伝播と `permission` の抽象値からの写像を分離して説明し、R-006 と R-008 の差異を記録する。

#### 検証境界

domain test では4値の YAML/JSON 往復と未知値・provider 固有値の拒否を検証する。workflow gateway test では YAML と Lua が同じ `WFS002` を返して `permission` の位置を持つこと、完成形の例が load できること、生成 stub が4値 union を持つことを検証する。provider lifecycle gateway test では2 provider × 4値の引数列、permission 省略時に引数を追加しないこと、`model` の引数が従来どおり無変換で共存することを検証する。builtin test では登録対象8本を実際に load し、Session 数が58で全件 `auto` であることを domain 値として確認する。

B-009 と B-010 の provider CLI 自身によるコマンド許可・拒否は、長時間外部プロセスを単体テストで起動しない `docs/architecture/TEST.md` の境界に従い、実 provider CLI を使う統合確認で検証する。単体テストは、その前提となる正確な起動引数列までを担当する。

### Interface

公開する workflow 定義 interface は YAML の `session.permission` と Lua の `r.session{ permission = ... }` である。入力は省略または `manual` / `auto` / `bypass` / `read-only` のいずれかで、出力は provider 固有語彙を含まない `WorkflowDefinition` となる。省略時は provider CLI に permission 引数を渡さない。

既存の Tauri command、local API、CLI の名前・入出力は変更しない。`WorkflowAgentSessionPort` の責務も「型付け済み Session 起動設定を AgentSession 起動へ渡す」のままとし、新しい port は追加しない。

この変更は workflow 定義の互換性を意図的に壊す。provider 固有値は alias として受理せず、利用者定義は4値のいずれかへ明示的に置き換える。builtin と完成形の例は同じ変更内で移行し、自動変換経路は設けない。

永続化済みの実行履歴との互換も壊す。`SessionSpec` は `WorkflowRootFact.definition` と `SessionRootFact.session` として event store の detail へ直列化されるため、provider 固有 permission 値を含む既存 fact は `NodeFact::decode` で失敗し、当該実行木の履歴読み出しと起動時 reconciliation はエラーになる（R-013）。fact の decode だけ旧値を許容する経路は設けず、移行手段も提供しない。

### Data Model

`SessionPermission` は workflow domain が所有する値オブジェクトで、identity は4つの列挙値そのものである。`SessionSpec`、workflow 実行開始時の定義 snapshot、AgentSession の provider 起動設定は同じ値を保持する。provider CLI のフラグ列は保持せず、起動直前に gateway が導出する。

永続表現は既存 `permission` field の省略可能な文字列 scalar を維持し、schema version は追加しない。受理値だけを4値へ限定し、provider 固有値を保持する legacy variant は追加しない。

### Database

該当なし。SQLite schema、index、access path は変更しない。

### UI/UX

該当なし。

### Algorithm

該当なし。provider 別の引数列は R-005 で一意に決まっており、追加の処理方式選択はない。

### Infra

該当なし。

## Alternatives Considered

`permission` を `Option<String>` のまま残し、loader と provider gateway で別々に値検証する案は採らない。有効値の規則が複数箇所へ分かれ、domain 内で provider 固有値や未知値を表現できる状態が残るため、R-001、R-003 と `docs/architecture/DOMAIN.md` の型・単一表現の原則を満たさない。

provider ごとの permission 型を持つ案も採らない。workflow 定義の段階で provider 固有語彙へ分岐し、provider を差し替えても意味を保つ R-001 の契約を失うためである。

## Cross-cutting concerns

該当なし。

## Risks

該当なし。
