# Context

## 入力文書

- 正本: [Issue #1690](https://github.com/siro33950/releash/issues/1690) `[workflow engine] Lua module 評価中の host API 呼び出しが RefCell 二重借用で panic し、workflow 一覧まで壊れる`（labels: `bug` / state: OPEN / milestone: なし）
- 配置先: `docs/specs/issues-1690`

## 参照した既存実装・既存文書

Issue が名指しした箇所、および調査で確認した関連箇所。いずれも実在を確認済み。

| 参照先 | 内容 |
| --- | --- |
| `src-tauri/src/infrastructure/lua/evaluator.rs:340` | `install_require`。`require` の Lua 関数を組み立てる。 |
| `src-tauri/src/infrastructure/lua/evaluator.rs:379` | `load_file_module`。workflows ディレクトリ配下の `.lua` を読み、その場で評価する。 |
| `src-tauri/src/infrastructure/lua/evaluator.rs:441` | `module_to_lua`。host module の member を Lua 関数へ変換する。生成される関数は呼び出し時に host を `borrow_mut()` する。 |
| `src-tauri/src/adaptor/gateway/workflow/lua/mod.rs` | Lua 定義の load。評価失敗を `WFS009` / `WFS010` / `WFS011` / host category へ写す。 |
| `src-tauri/src/adaptor/gateway/workflow/diagnostics.rs:198` | `diagnose_lua_workflow_source`。load 失敗を Diagnostic 項目へ変換する。 |
| `src-tauri/src/adaptor/gateway/workflow/storage.rs:373` | `list_workflows_with_facets`。診断でエラーを持つ定義を `Invalid workflow definition` として一覧に残す。 |
| `src-tauri/src/adaptor/gateway/workflow/diagnostics.rs:1683` | `load_workflows_in_scope`。定義ファイルごとに結果を `Ok` / `Err(diagnostics)` として蓄える。 |
| `src-tauri/src/adaptor/controller/api/workflow.rs:61` | `GET /v1/workflows` → `list_workflows`。 |
| `src-tauri/src/adaptor/controller/command/workflow/definition.rs:19` | Tauri command `list_workflows`。 |
| `docs/glossary/WORKFLOW.md:413` | 「部品は Sequence を返す関数として作り、再利用時は関数を再度呼んで独立した Node 群を得る」。文法正本における推奨形の記述。 |

## 確定済みの前提

- Lua workflow 定義の `require` は workflows ディレクトリ配下だけを探索する（`docs/glossary/WORKFLOW.md:413`）。
- workflow 定義の入口は Tauri command、loopback HTTP local API、CLI の3つで、いずれも同じ usecase を共有する（`AGENTS.md`「構成で押さえる点」）。一覧は Tauri command `list_workflows` と `GET /v1/workflows` の双方が usecase の `list_workflow_summaries` を通る。
- Lua の評価環境は外部 I/O を持たず、メモリ量と命令数に上限がある。この上限は緩めない（`AGENTS.md`「セキュリティ」）。
- 一覧経路には、定義ファイル単位でエラーを Diagnostic として扱い当該定義だけを不正表示に落とす仕組みが既にある（`storage.rs:373` の `Invalid workflow definition`、`diagnostics.rs:1683` のファイル単位 `Err`）。この仕組みは Diagnostic として返る失敗にしか働かない。
- 本 crate の Rust edition は `2021`（`src-tauri/Cargo.toml:7`）。
- 現在 `workflows/` に `.lua` の定義ファイルは存在しない。再現には定義ファイルの追加が要る。

# Outcome

## 対象者

- workflow 定義を Lua で書き、`require` で定義を複数ファイルに分割する開発者。
- 同じ workflows ディレクトリを参照して workflow を観測・実行する Releash の全利用者（desktop UI、local API client、CLI）。

## 現在の問題

1. Lua module が module 本体の評価中（トップレベル）に host API を呼ぶと、その module を `require` する定義は load されず panic になる。定義が成立しないうえ、失敗が Diagnostic として返らないため、利用者は原因の位置も種類も分からない。
2. その定義ファイルが workflows ディレクトリに存在する限り、workflow 一覧そのものが失敗する。他の正しい定義まで観測できなくなり、UI から復旧操作にたどり着けない。

## 変更後に実現する状態

1. module 評価中の host API 呼び出しが正当なパターンとして動作し、その module を `require` する定義は正常に load される。
2. 1つの定義ファイルの load 失敗が、その定義の不正表示に閉じる。一覧および他の定義には波及しない。

# Current Behavior

## 再現手順

`workflows/` に次の2ファイルを置く。

```lua
-- workflows/broken.lua
local r = require("releash")
local nodes = require("broken-parts.nodes")
return r.workflow{ name = "broken", description = "x", main = nodes.main }
```

```lua
-- workflows/broken-parts/nodes.lua
local r = require("releash")
return { main = r.command{ command = "true" } }
```

この状態で workflow 一覧を取得する（desktop UI の workflow 一覧表示、または `GET /v1/workflows`）。

## 実際の出力

- Issue に記載された観測結果: `task join error: task 262726 panicked with message "RefCell already borrowed"`。この文言は Tauri command `list_workflows` の join error 整形（`src-tauri/src/adaptor/controller/command/workflow/definition.rs:26`）と一致する。
- Issue の記載: `GET /v1/workflows` も同じ load 経路で壊れ、一覧が配列でなくエラーを返す。

## 経路の説明

- `install_require`（`evaluator.rs:340`）は、host 提供 module と file module の分岐を `if let Some(module) = host.borrow().module(&module_name) { … } else { load_file_module(…) }` の形で書いている。
- edition 2021 では if-let の scrutinee の一時値（`host.borrow()` が返す借用 guard）が、else 分岐を含む if-let 式全体の終わりまで生存する。したがって `load_file_module` が module を評価している間、host の不変借用が保持されたままになる。
- `load_file_module`（`evaluator.rs:379`）は読み込んだ source をその場で評価する。module 本体が `r.command{…}` のような host 関数を呼ぶと、`module_to_lua`（`evaluator.rs:441`）が作った closure が host を `borrow_mut()` し、二重借用で panic する。
- module が host を呼ばない値（純関数など）だけを返す形に書き換えると、評価中に `borrow_mut()` が起きないため発生しない。
- 一覧経路の失敗の扱い: `list_workflows_with_facets`（`storage.rs:373`）は診断でエラーを持つ定義を `Invalid workflow definition` に落として一覧へ残す。しかし panic は Diagnostic として返らないため、この分岐に到達せず `spawn_blocking` の JoinError として一覧全体の失敗になる。
- 上記の経路解析は、Issue 記載の見立てと本 spec 作成時のコード確認によるものであり、修正方針の確定ではない。

## 影響範囲の確認結果

- Tauri command `list_workflows`（`definition.rs:19`）と local API `GET /v1/workflows`（`api/workflow.rs:61`）は、ともに usecase の `list_workflow_summaries` を経由する。
- workflow diagnostics（`GET /v1/workflow/diagnostics`、CLI `releash workflow diagnostics`）も `diagnose_workflow_file` を通り、同じ Lua 評価経路を使う。

# Scope / Non-goals

## 変更する対象

- Lua module の評価中に host API が呼ばれた場合の扱い。
- 1つの workflow 定義の load 失敗が、workflow 一覧および他の定義に波及しないこと。
- 上記が Tauri command、local API、CLI のいずれの入口でも成り立つこと。

## 変更しない対象

- Lua 文法および host API（`r.session` / `r.command` / `r.input` 等）の仕様追加・削除。
- `docs/glossary/WORKFLOW.md` が示す推奨形（部品は Sequence を返す関数として作る）そのものの変更。推奨は推奨のまま残す。
- YAML 形式の workflow 定義の load・診断の挙動。
- workflow の実行（execution）系の挙動。
- Diagnostic code 体系（`WFS00x` の割り当て規則）の再設計。
- Lua 評価環境の制限値（メモリ上限、命令数上限、`require` の探索範囲）の変更。

# Requirements

- R-001: workflows ディレクトリ配下の Lua module が module 本体の評価中に host API を呼び、その結果を module の返り値に含めて返したとき、その module を `require` する workflow 定義は正常に load される。module が返した Node は、当該 workflow の Node として定義に含まれる。
- R-002: R-001 の形の定義ファイルが workflows ディレクトリに存在する状態でも、workflow 一覧の取得は成功する。一覧は定義の配列を返し、当該定義は有効な定義として一覧に含まれる。
- R-003: R-002 の一覧取得の成功は、Tauri command `list_workflows` と local API `GET /v1/workflows` の双方で成り立つ。
- R-004: R-001 の形の定義ファイルが存在する状態でも、workflow diagnostics の取得は成功する。当該定義について load の失敗を示す Diagnostic 項目は報告されず、他の定義の診断結果は通常どおり返る。これは local API `GET /v1/workflow/diagnostics` と CLI `releash workflow diagnostics` の双方で成り立つ。
- R-005: 現在正常に load できる Lua 定義および YAML 定義の load 結果（一覧の内容、定義の内容、診断結果）は変わらない。
- R-006: Lua 評価環境の制限は緩まない。`require` の探索範囲は workflows ディレクトリ配下のままであり、メモリ上限と命令数上限は現在の値のまま有効である。
- R-007: workflow 定義 1 件の load 失敗は、当該定義が不正な定義として一覧に含まれることに閉じる。workflow 一覧の取得は成功し、他の定義の load 結果および診断結果には波及しない。当該定義の一覧上の扱いは、既存の不正な定義（診断エラーを持つ YAML 定義）と同じである。

# Assumptions / Open Questions

## Open Questions

- なし。

## Assumptions

- なし。ユーザーが明示的に受け入れた仮定は存在しない。
