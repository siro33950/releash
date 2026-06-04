# Design

## The actual design

### Architecture

4 ワークフローは役割ベースの統一命名（番号なし）とする。実行順を名前に埋め込まず、役割で識別する。

| 役割 | workflow name | 由来 |
|---|---|---|
| Spec作成 | `spec-authoring` | 既存（変更なし） |
| 実装 | `spec-implement` | 既存を再構築（軽量レビューループ追加） |
| フルレビュー | `full-review` | `multi-agent-code-review` を rename |
| フルレビュー後修正 | `full-review-fix` | `implement-from-threads` を rename |

- 番号プレフィックス（`01-` 等）は付けない。listing 上の順序を名前で強制すると実行順の誤解を生むため、役割の分かりやすさは name と description で表現する。

#### spec-implement（実装）のノードトポロジー

唯一新規に組むワークフロー。全 agent node で構成し approval を置かない（Human-in-the-Loop なし）。

```
implement(agent)
  → review_parallel(parallel: review_spec / review_design, 軽量モデル, 指摘は Thread 投稿)
       指摘なし → 完了 / 指摘あり → fix
  → fix(agent: Open Thread を読み修正し resolve)
       → review_parallel へループ
  cycle_guard で最大 2 周
```

review_parallel の 2 観点:

- **review_spec** — Spec を満たしているか（受け入れ基準・要求充足）。
- **review_design** — 最低限の実装方針・設計の妥当性。責務分割・基本的な設計の健全性・アーキテクチャ適合を含む（「アーキテクチャ照合」だけに限定しない）。

quality / test / security などの高精度・全観点レビューはこのワークフローに含めず、後段の `full-review` に集約する（軽量・高速に要求充足へ到達させるという役割分担）。ループ終了シグナルと軽量モデルの選定は Algorithm を参照。

#### 既存資産の流用と廃止（R5）

新 4 ワークフローから参照される facet のみを残し、参照されない既存 yml / facet は全廃する。

**流用は「そのまま流用」ではなく、必ず Rename を伴う。** rename される workflow（full-review ← multi-agent-code-review、full-review-fix ← implement-from-threads）に専用の旧名 facet も、新 workflow 名に揃えて rename する（`mar-*` instruction、`multi-agent-reviewer` / `multi-agent-summary` policy、`implement-from-threads` instruction など）。旧名のまま残さない。

- **流用（Rename して再利用）**: spec-authoring 一式（planning policy / write-* / spec-finalize / spec-directory contract）、full-review（multi-agent-code-review を rename）一式（旧 mar-* / multi-agent-reviewer・summary policy を full-review 名に rename）、full-review-fix（implement-from-threads を rename）の旧 implement-from-threads instruction（full-review-fix 名に rename）、coding policy、releash-thread-cli knowledge、spec-implement の implement instruction。
- **廃止**: bug-fix / spec-driven-development(-gpt-5-5) / spec-plan / spec-review(-auto) の各 yml と、それらにのみ参照される facet（bug-* 一式、plan-review policy、fix-policy-auto、review-summary、spec-driven 系 review-* / implement-fix / implementation-* instruction、review-verdict / approved-fix-policy / bug-investigation-result の各 Contract）。

spec-implement の review / fix 用 instruction は新規作成する。spec-implement は Thread 方式・軽量・2 観点で、既存 review-*（review-verdict Contract 方式・全観点）とは前提が異なり、流用は全面改稿になるうえ廃止対象の review-verdict Contract への依存を残してしまうため。

### Interface

workflow listing（CLI / UI のワークフロー一覧）に出る 4 ワークフローの description は日本語で統一し、1〜2 文の簡潔な役割説明に揃える。現状の既存 description は英日混在で粒度もばらついている（spec-authoring=英語、multi-agent-code-review=英語長文、implement-from-threads=日本語）ため、新 4 構成では一覧の可読性を優先して揃え直す。

### Data Model

spec-implement の review_parallel → fix → review_parallel ループでは、指摘の受け渡しを Thread のみで行う。review_spec / review_design は `releash review create` で指摘を Thread として投稿し、fix は `releash review list --state open` で未対応 Thread を取得して修正・`releash review resolve` で解決する。step 間 Contract（旧 review-verdict / approved-fix-policy 等）は経由しない。aggregate（指摘ありなしの判定）は既存 builtin の規約に揃え、review_parallel 各子は出力末尾に必ず `LGTM` または `NEEDS_FIX` のどちらかを出力する（指摘ありのときは加えて Thread に投稿し `NEEDS_FIX` を出す）。aggregate は `all_match: LGTM` で完了に分岐する。

### Database
<!-- 永続化先・スキーマに関する設計判断 -->

### UI/UX

workflow listing の表示順は、概念上の利用フロー順（spec-authoring → spec-implement → full-review → full-review-fix）に揃える。`BUILTINS` 配列の登録順がそのまま listing 順となるため、配列の並びをこの順に保つ。

### Algorithm

#### spec-implement のループ終了シグナル

review_parallel の各子（review_spec / review_design）は出力末尾に必ず `LGTM` または `NEEDS_FIX` のどちらかを出力する規約に従う。aggregate は `all_match: LGTM` を完了分岐の条件、else を fix 分岐の条件とする。これは multi-agent-code-review / spec-driven-development / bug-fix が採る既存 LGTM 規約と統一し、独自語彙を持ち込まない。

#### spec-implement のループ検知（既存指摘の確認）

ループ検知は review_spec / review_design 側で行う。各 reviewer は Thread を投稿する前に、投稿先ファイルに紐づく既存 Thread（Resolved を含む全件、`releash review list` / `releash review get` / `releash review history` で取得）を確認し、同一指摘・競合指摘がないか点検する。重複指摘は新規投稿しない（既存 Thread の文脈で扱う）。`cycle_guard: max_iterations: 2` の hard cap と併用してループの上限も担保する。

#### spec-implement の軽量モデル選定

review_spec / review_design は `gpt-5.4-mini` 同一とする。利用可能な軽量モデル群（claude-haiku-4-5-20251001 / claude-sonnet-4-6 / gpt-5.4-mini）のうち、SWE-bench Pro で gpt-5.4-mini が 54.4% と他の軽量モデル（haiku 4.5 が 39.5%、sonnet 4.5 系が約 43.6%）を明確に上回り、軽量クラスでは Claude opus 4.5 のスコアすら超える。R2 の「軽量・低コストで実装精度を保証する」方針と最も整合する。Claude/Codex のバックエンド多様性は full-review 側で担保されているため、spec-implement で多様性のために別 backend を混ぜる必要はない。

#### spec-implement の実装系モデル

実装系ステップ（implement / fix）は `gpt-5.5` で統一する。R2 は fix のモデルを指定していないが、implement と fix は同じ「実装」作業であり同一モデルで揃える方が自然。Codex backend の最高性能モデル（SWE-bench Pro 58.6%）で実装精度を確保し、レビュー側は軽量 gpt-5.4-mini で素早く回す。Codex backend に統一されることでバックエンド切替コストが下がり、軽量レビューループ全体が同 backend で完結する。

### Infra
<!-- インフラ構成・デプロイに関する設計判断 -->

## Alternatives Considered

### spec-implement の review 軽量モデル
候補：claude-haiku-4-5-20251001 / claude-sonnet-4-6 / gpt-5.4-mini / Claude+Codex 混在。SWE-bench Pro スコアで gpt-5.4-mini が 54.4% と他軽量モデル（haiku 4.5: 39.5%、sonnet 4.5 系: 約 43.6%）を明確に上回り、軽量クラスで Claude opus 4.5 のスコアすら超えた点が決め手。Claude/Codex の backend 多様性は full-review 側で担保されているため、spec-implement で多様性のために別 backend を混ぜる費用対効果は薄いと判断し、両 reviewer を gpt-5.4-mini 同一とした。

### spec-implement の review→fix 受け渡し方式
候補：Thread のみ / Thread + 既存 review-verdict Contract 併用 / spec-implement 専用の新規 Contract 定義。Thread + review-verdict 案は廃止対象の Contract への依存を残してしまうこと、新規 Contract 案は facet 追加コストが要求の軽量方針と不整合だったことから、Thread のみとした。aggregate の機械的判定は既存 `LGTM` 規約の終端文字列で十分担保できる。

## Cross-cutting concerns

### テスト方針

新 4 ワークフローへの再編に伴い、ビルトインワークフロー固有のテスト（builtin.rs 配下に存在するワークフローごと・facet ごとの検証テスト、yml と Rust 側メタデータの一致テスト、特定 instruction 名前の文面検査テスト、ワークフロー個別のトポロジー検証テスト、全 builtin 横断のロードや prompt 合成チェック等）は新規追加せず、既存のものも全て削除する。yml の構造妥当性は `validation::validate` に委ね、ワークフロー固有の振る舞いはビルトイン yml 自体が仕様書として機能する。

## Risks
<!-- 既知のリスク・不確定要素・追加調査が必要な点 -->
