# Design

## The actual design

### Architecture

#### host 借用を module lookup に閉じる

この修正の責務 owner は `install_require`（`src-tauri/src/infrastructure/lua/evaluator.rs:340`）が組み立てる `require` callback である。gateway 以上の層は変更しない。

現状の分岐は次の形をとる（`evaluator.rs:361`）。

```rust
let result = if let Some(module) = host.borrow().module(&module_name) {
    module_to_lua(lua, module, Rc::clone(&host))
} else {
    load_file_module(lua, &base_dir, &module_name, location.clone())
};
```

crate の edition は `2021`（`src-tauri/Cargo.toml:7`）であり、if-let の scrutinee が作る一時値は else 分岐を含む if-let 式全体の終端まで生存する。したがって `load_file_module`（`evaluator.rs:379`）が module 本体を評価している間、`host` の不変借用 guard が保持されたままになる。module 本体が host 関数を呼ぶと `module_to_lua`（`evaluator.rs:441`）が生成した closure が `borrow_mut()`（`evaluator.rs:458`）へ入り、二重借用で panic する。

決定: lookup の結果を分岐前に束縛し、guard を lookup 文の終端で落とす。分岐は束縛済みの値に対して行う。

```rust
// borrow guard を分岐へ持ち込むと、file module の評価中に host 関数が
// borrow_mut() へ入り二重借用になる。lookup 結果は必ず先に束縛する。
let host_module = host.borrow().module(&module_name);
let result = match host_module {
    Some(module) => module_to_lua(lua, module, Rc::clone(&host)),
    None => load_file_module(lua, &base_dir, &module_name, location.clone()),
};
```

`LuaHost::module` は `&self` を取り所有値 `Option<LuaModule>` を返す（trait は `evaluator.rs:139`、実装は `adaptor/gateway/workflow/lua/mod.rs:491`）。lookup を先に完了させても戻り値の意味は変わらない。束縛を戻す refactor で同じ欠陥が再発するため、理由をコメントとして残す。

修正後、module 本体からの host 呼び出しは通常の host call として arena へ Node を積む。R-001 / B-001 が要求する「module が返した Node が workflow の Node として含まれる」は、builder が node 名を木の構造から決める（`lua/mod.rs:1294` の `build` と `visit_node`）ため、Node が module 由来であることに依存せず成立する。

#### 借用不変条件の適用範囲

規則: host の借用 guard を、Lua へ制御が戻りうる処理をまたいで保持しない。

現行コードでこの規則に反するのは上記の 1 箇所だけである。`module_to_lua` の call closure（`evaluator.rs:457`）と `HostUserData` の index metamethod（`evaluator.rs:213`）は、いずれも `let` 文で `borrow_mut()` の結果を受け取り、guard を落としてから `data_to_lua` を呼ぶ。`loading` と `cache` の借用は Lua 評価をまたがない。したがって変更対象は `install_require` の分岐だけであり、`LuaHost` trait の形状と他の借用点は変えない。

#### 失敗経路と一覧・診断への波及

決定: gateway、usecase、controller のいずれにも変更を入れない。

修正後、module 本体の評価が失敗した場合は既存どおり `CallbackFailure` として `LuaFailure` に載り、`map_evaluation_error`（`adaptor/gateway/workflow/lua/mod.rs:148`）が `WFS009` / `WFS010` / `WFS011` / host category へ写す。

- 一覧: `list_workflows_with_facets`（`storage.rs:373`）が `diagnose_workflow_file` の結果に error があれば当該 1 件だけを `Invalid workflow definition` の summary へ落とし、`list_file_summaries`（`storage.rs:311`）が他のファイルの走査を続ける。`source_format` はファイル拡張子から決まる（`storage.rs:510`）ため、診断エラーを持つ YAML 定義と同じ扱いになる。R-007 / B-004 はこの既存経路のまま成立する。R-002 / B-002 が要求する「当該定義が有効な定義として一覧に含まれる」は、借用修正により当該定義が診断エラーを持たなくなるため、同じ経路で成立する。
- 診断: `diagnose_lua_workflow_source`（`diagnostics.rs:198`）が load 失敗を単一の Diagnostic 項目へ写し、`load_workflows_in_scope`（`diagnostics.rs:1683`）がファイル単位で `Err(diagnostics)` として蓄える。R-007 / B-006 も同様に成立する。R-004 / B-005 が要求する「当該定義に load 失敗の Diagnostic が報告されない」は、借用修正により当該定義の load が成功するため成立する。

一覧が全体として失敗していたのは、panic が Diagnostic ではなく `spawn_blocking` の JoinError（`adaptor/controller/command/workflow/definition.rs:24`）として抜けていたためである。panic が消えれば入口側の整形に手を入れる必要はない。

### Interface

外部から観測できる契約は変更しない。

- Tauri command `list_workflows`（`adaptor/controller/command/workflow/definition.rs:19`）、local API `GET /v1/workflows`（`adaptor/controller/api/workflow.rs:61`）、`GET /v1/workflow/diagnostics`、CLI `releash workflow diagnostics` の入出力を変えない。両一覧入口は同一の usecase `list_workflow_summaries`（`usecase/workflow/mod.rs:94`、呼び出しは `definition.rs:24` と `api/workflow.rs:110`）を通るため、R-003 / B-003 は入口ごとの分岐を設けずに成立する。
- crate 内部の `LuaHost` trait、`evaluate` の signature、`LuaFailure` / `LuaFailureKind` / `LuaWorkflowError` も変えない。
- Lua 側から観測できる契約（`require` の解決規則、`releash` / `facets` module の member）を変えない。

### Data Model

該当なし。追加・変更する record はない。

### Database

該当なし。

### UI/UX

該当なし。frontend に変更はない。

### Algorithm

該当なし。処理方式の選択は Architecture の借用範囲の決定に尽きる。

### Infra

該当なし。

## Alternatives Considered

- **load 経路を `catch_unwind` で包み、panic を Diagnostic へ写す**: 却下。panic の原因である借用範囲が残るため、当該定義は Diagnostic を伴う不正な定義のままになり、R-001 が要求する正常な load を満たさない。加えて Lua 評価中の任意の panic を Diagnostic へ変換することになり、Requirements が条件として定めていない範囲の外部挙動を設計で決めてしまう。
- **module 本体の評価中の host 呼び出しを検出して拒否する（非対応化）**: 却下。R-001 が、当該 module を `require` する定義の正常な load と、module が返した Node の取り込みを要求している。加えて現行コードに「module 本体を評価中か」を判定できる点がなく、検出機構の新設を要する。
- **crate の edition を 2024 へ上げ、if-let 一時値の drop 位置の変更で解消する**: 却下。同形の一時値の生存範囲が crate 全体で変わるため、影響が本件と無関係な箇所へ広がり、R-005（現在正常に load できる定義の結果を変えない）に対する risk が増える。
- **`Rc<RefCell<H>>` を再入可能な所有形へ置き換える**: 却下。`LuaHost::call` / `index` は `&mut self` を要求し（`evaluator.rs:141`、`evaluator.rs:148`）、host は評価中に arena を伸ばす。置換の影響は host 実装全体へ及ぶ一方、本件の失敗は借用範囲だけで解消できる。

## Cross-cutting concerns

- セキュリティ: `require` の探索範囲（`resolve_module_path` の canonicalize と `starts_with(base_dir)` 判定、`evaluator.rs:410`）、`scrub_globals`（`evaluator.rs:279`）、`LuaLimits::default` のメモリ上限と命令数上限（`evaluator.rs:29`）のいずれにも触れない。R-006 / B-009 / B-010 は現行実装のまま成立する。
- 失敗位置の観測: module 本体で失敗した場合、`caller_location` は `load_file_module` が `set_name` へ渡した module ファイルの絶対パスを source として返し、`lua_location_span`（`diagnostics.rs:286`）が workflows ディレクトリ相対へ正規化する。既存の `lua_component_error_uses_workflow_relative_source_and_line`（`diagnostics.rs:5264`）と同じ経路であり、B-006 の失敗位置はこの経路で得る。
- 検証の置き場: 借用範囲そのものの回帰は `infrastructure/lua/evaluator.rs` の既存 test module へ置く。`TestHost` は `require('test')` で host module を返す（`evaluator.rs:689`）ため、`TempDir` に「トップレベルで `test.value(...)` を呼ぶ module」を書いて `evaluate` を通せば、修正前は panic、修正後は成功として区別できる。panic は `Result` に現れないため、host 呼び出しを含む module fixture が唯一の検出点になる。B-001 / B-002 / B-004 / B-005 / B-006 は必須レイヤーである `adaptor/gateway/`（`docs/architecture/TEST.md`）で、module ファイルを `TempDir` へ書いてから `load_lua_workflow` / `list_workflows_with_facets` / `diagnose_lua_workflow_source` を呼ぶ既存の書き方（`lua/mod.rs:2316`、`storage.rs:1227`、`diagnostics.rs:5264`）に合わせる。B-003 と B-007 は入口ごとに実装が分岐しない（一覧は同一 usecase、CLI diagnostics は local API 経由）ため、controller 層にテストを新設せず既存の `src-tauri/tests/workflow_diagnostics_cli_test.rs` の範囲を保つ。

## Risks

該当なし。
