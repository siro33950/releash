# Goal: #1325 Contract / Artifact を schemas と artifact に統一

まず `docs/specs/milestone-82/goal-common.md` を読み、そこに書かれた必読ドキュメント・設計判断・横断ルール・品質ゲートに従うこと。`gh issue view 1325 --repo siro33950/releash` で issue 本文を読むこと。

旧 `output_contract` / `input_contracts`（markdown contract facet + contract-validation メタブロック方式）を廃止し、YAML 内 `schemas:` と node の `artifact:` / `input:` を正にする。

## 実装内容

1. **schemas: セクション**（schema.rs + domain）: workflow 直下に `schemas: { <名前>: <JSON Schema subset> }` を追加。D2 の subset（type/properties/required/items/enum/additionalProperties、scalar string、配列要素は名前付き参照）を Rust の型として定義し、subset 外の構文は load 時 Diagnostic。
2. **Contract 検証エンジン**（domain service）: JSON value を subset schema で検証する validator を新設。`domain/workflow/services/contract.rs` の contract-validation メタブロック解析・`spec-directory` ハードコード（52-67 行）を置換・撤去する（spec_dir の generic な後継は #1326 の inputs 参照）。
3. **artifact: / input:**: node 共通 field `artifact: <Contract 名>` / `input: <Contract 名>` を実装。未定義 Contract 参照は Diagnostic。各 NodeExecution は最大 1 つの Contract 検証済み Artifact を産出し、**Node 名で参照**できるよう projection / state に artifact 保管を実装する（参照 path に Contract 名を出さない）。
4. **提出経路の統一**: session の CLI submit（`workflow output submit`）と、（#1328 で実装される）command stdout-JSON が同じ Artifact 検証・保存経路を通る構造にする。OutputSubmitted event / repair 機構（ContractRepairRequested、StructuredOutputRepairPolicy）を schemas 検証に接続し、repair prompt の文面を schemas 語彙に更新する。P13: session + artifact は検証済み提出まで完了しない（既存挙動踏襲）。
5. **routing 制約の部品**: 「Contract の field が routing 可能（required かつ boolean/enum）か」を判定する検査ユーティリティを domain に実装する（rules 側からの接続は #1327。本 goal では判定関数 + 単体テストまで）。
6. **contract facet の廃止**: FacetKind::Contract、`builtin_facets/contracts/*.md`、ResolvedFacets の output_contract / input_contracts を撤去。facet は policy / knowledge / instruction の 3 種にする。facet 編集 UI / Tauri command / CLI に contract facet が残らないこと。
7. **built-in 移行**: 各 built-in の output_contract / input_contracts を `schemas:` 宣言 + `artifact:` / `input:` 参照へ書き換える。contract facet md のメタブロック内容を JSON Schema subset に変換して YAML に埋め込む。instructions facet 本文が `<workflow_output>` や contract facet 名を参照していれば schemas 語彙に更新する。
8. **DTO / protocol / frontend 型**: `usecase/workflow/dto.rs`、`adaptor/protocol/workflow.rs`、`src/types/workflow.ts` から output_contract / input_contracts を撤去し、schemas / artifact / input を通す。

## 削除対象

- `output_contract` / `input_contracts`（schema / domain / DTO / protocol / frontend / built-in / facet）
- contract facet（FacetKind::Contract、builtin_facets/contracts/、facet CRUD の contract 経路）
- contract-validation メタブロック解析、`spec-directory` ハードコード

## テスト

- `schemas:` の validation（subset 準拠 / subset 外拒否 / scalar string / 名前付き items 参照）。
- `artifact:` 産出、Contract validation success / failure、失敗時 repair 経路。
- session / CLI submit が同じ Artifact 機構に書き込むこと。
- routing 可能 field 判定（boolean/enum required は可、それ以外は Diagnostic）の単体テスト。
- 旧 `output_contract` / `input_contracts` を受理しない regression test。
- built-in 12 本が schemas 語彙で load / validate できること。
