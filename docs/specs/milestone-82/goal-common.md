# 全 goal 共通の前提・規約（milestone 82）

Releash（Tauri 2 デスクトップアプリ）の workflow engine を GitHub milestone 82「Workflow Engine 新モデル移行」
https://github.com/siro33950/releash/milestone/82 に従って移行する。各 goal はその 1 issue 分である。

## 必読ドキュメント（実装前に全て読むこと）

1. `CLAUDE.md`（リポジトリルート）と `AGENTS.md`、`src-tauri/AGENTS.md`
2. `docs/architecture/` の README.md / DOMAIN.md / USECASE.md / GATEWAY.md / CONTROLLER.md / TEST.md / **GLOSSARY.md（正規語彙・使用禁止語）**
3. `docs/workflow-engine-evolution-plan.md`（戦略・中核モデル・テスト方針の一次 Owner）
4. `docs/workflow-yaml-syntax.md`（目標構文の正本）
5. `docs/examples/full-pipeline.yml`（完成形の参照例。整合確認に使う。通常改修対象にしない）
6. `docs/specs/milestone-82/plan.md`（実装計画。設計判断 D1〜D8 / P1〜P15 と現状実装マップ）
7. **`docs/specs/milestone-82/design.md`（詳細設計の正本。型定義・検証規則・event 語彙・API・削除一覧はここに従う。設計をやり直さない）**
8. 対象 issue 本文: `gh issue view <issue番号> --repo siro33950/releash`
9. milestone 説明: `gh api repos/siro33950/releash/milestones/82 --jq .description`

注意: `docs/workflow-engine-model-boundary.md` は旧 north star であり、モジュールパス（`src-tauri/src/workflow/` 等）と語彙（WorkflowRun / `type:`）が stale。現在の実体は `src-tauri/src/{domain,usecase,adaptor}/**/workflow/` 配下にある。矛盾したら evolution-plan / GLOSSARY / issue 本文を正とする。

## 確定済み設計判断（覆さないこと。詳細は plan.md §2〜§3）

- **D1**: CLI の正は Tauri アプリ内に新設する最小 local API（localhost + 認証トークン）。#1332 で実装。
- **D2**: `schemas:` は JSON Schema subset（`type`/`properties`/`required`/`items`/`enum`/`additionalProperties`）の自前実装。routing 参照 field は required かつ boolean/enum。scalar(string) Contract を許可（request 用）。配列要素型は inline 不可・名前付き Contract 参照（`items: <名前>`）。
- **D3**: command の標準結果（`ok`/`exit_code`/`stdout`/`stderr`/`duration`）は Artifact の予約 field。`artifact:` Contract field と単一名前空間に合成し、予約名衝突は load 時 Diagnostic。
- **D5**: session の permission 許可値は ask/edit/full の現行 3 値のまま。`read` は追加しない。
- **D6**: `{{ project_name }}` / `{{ path_alias.* }}` / `{{ vars.* }}` / `{{ task }}` / workflow の `variables:` は全廃。template 参照は `{{ request }}` / `{{ <node>.<field> }}` / `{{ item.<field> }}` のみ。
- **D7**: workflow 編集 UI はフォーム編集を廃止し、YAML 直接編集 + Rust が返す Diagnostic 表示に簡素化する。
- **D8**: Workspace UI はNode中心の再帰treeと単一NodeContentViewに統一する。Workflow/FanoutはNodeを束ねるbranchであり、独自中央viewを持たない（#1454）。
- **P2**: WorkflowExecution.status は running/waiting_approval/completed/failed/aborted を維持（#1335 で interrupted を追加）。
- **P3**: fanout 実行中は child の rules を無視する（Diagnostic にしない）。
- **P4**: event log（NDJSON）/ 実行 state の在庫は破棄前提。schema 変更で互換 reader・変換層を書かない。
- **P6**: 予約 Artifact 名は `request` / `item` のみ。`tasks` は予約語ではない。Task Entity / WorkflowExecution-owned tasks[] / `releash task ...` は実装しない（issue #1333 は撤回済み）。
- **P11**: rules の `on:` 参照 field が実行時に不在なら no-match とし catch-all `next` に落ちる。artifact 検証が失敗しうる node が Contract field を rules で参照する場合、`next` catch-all を必須とする（網羅検証に含める）。command の `ok` は `exit_code==0 && (artifact 未指定 || validation 成功)`。
- **P13**: `session` + `artifact:` は Contract 検証済み提出まで node 完了しない（既存 repair 機構を踏襲）。
- **P15**: WorkflowExecution / NodeExecution domain read modelは維持し、Workspace UIにはRust-ownedの再帰tree summaryと選択Node detailを提供する。attempt・fanout座標・内部IDをUIへ露出しない（#1454）。

## 実装原則（最優先: 最もシンプルな実装にする）

出来上がる実装は「最もシンプルで、型の表現力で正しさを保証する」ものにする。移行の都合でコードを複雑にしない。

1. **型で不正状態を表現不能にする**。syntax doc「文法健全性の担保」節を実装原則とする: kind は `NodeKind = Command | Session | Fanout` の enum、rule は `When | Switch | LoopGuard | Next` の tagged enum + deny_unknown_fields。「kind block はちょうど1つ」「rule はいずれか1つ」を実行時チェックではなく型で保証する。optional field の組み合わせで種別を表現しない。
2. **パッチではなく置換**。対象表現の旧実装を修正して延命せず、新実装に置き換えて旧コードを削除する。旧実装の構造（regex 評価、aggregate 分岐等）を新語彙に改名しただけのコードを残さない。
3. **互換層・変換層・feature flag・新旧併存を作らない**。schema が直接の deserialize 先であり、正規化レイヤーや新旧ブリッジを新設しない。
4. **先回りの抽象化をしない**（YAGNI）。将来用の trait・拡張点・使われない汎用化・未使用 field を追加しない。timeout / retry / 並列度などスコープ外の予約構文も作らない。
5. **削除が正**。旧表現の削除で不要になった型・関数・テスト・DTO・frontend コード・facet は同じ PR で消す。dead code を残さない。
6. **ロジックは一箇所に置く**。validation は Diagnostic pipeline、Artifact 検証は Contract エンジンの一箇所に集約し、同じ検査を複数層で重複実装しない。schema ↔ domain の鏡像型は層規約上必要な最小限に留める。
7. **巨大 module を肥大させない**。runtime_engine_impl.rs（4,820 行）等に追記して育てず、触る goal では該当部分を kind 単位の明確な実行経路として切り出し・置換する。

## 横断ルール

- 本 goal の対象表現について「新語彙の実装 → runtime / event log / projection / CLI・API / UI / built-in workflow / tests の移行 → 対応する旧語彙・不要ロジックの削除または loader での拒否」までを本 goal 内で完了する。**スコープ縮小・部分対応での完了報告は不可**。
- 長期互換 adapter を持たない。一時 adapter を作った場合も本 goal 完了時に撤去する。
- まだ移行していない別表現を先に壊さない。他 issue の担当表現は現状のまま新構造に内包して通す。
- 全ロジックは Rust に置く。frontend は表示・入力・invoke・表示用フォーマットのみ（`.claude/rules/rust-first-logic.md`）。
- GLOSSARY.md の使用禁止語（WorkflowRun / Run / StepExecution / WorkflowStep / ParallelRun / NodeType 等）を新規コードに使わない。
- Diagnostic は lifecycle state ではなく validation result。frontend に validator を実装しない。
- built-in workflow（`src-tauri/src/adaptor/gateway/workflow/builtin/*.yml` 12 本）と builtin_facets の本文は、本 goal の対象表現について新語彙に移行し、load / 実行できることをテストまたは fixture で確認する。
- `docs/examples/full-pipeline.yml` は参照例として整合確認に使い、#1337 以外では改修しない。
- Trigger / timer / external event 起動設定は milestone #81 の領分。WorkflowDefinition 文法に含めない。

## 品質ゲート・テスト・報告

- 新表現の behavior test と旧表現を拒否する regression test を同じ PR に置く。
- Rust: `src-tauri/` で `cargo fmt --check` / `cargo clippy -- -D warnings`（一括 allow 禁止）/ `cargo test`。
- frontend: リポジトリルートで `pnpm lint` / `pnpm test` / `pnpm build`。
- テスト失敗時に期待値を実装へ合わせて書き換えない（仕様を確認し、実装が誤っていれば実装を直す）。
- 完了報告には issue の受け入れ基準ごとに「実装内容 + 検証方法（テスト名 or 手動手順）」を列挙する。対応不能・仕様矛盾と判断した項目は勝手に読み替えず、理由付きで全件列挙する。
