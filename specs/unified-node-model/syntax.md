# Workflow YAML 構文（統一 Node モデル・確定分）

作成日: 2026-07-16

統一 Node モデルの構文の確定事項を記録する。モデルの決定は [decisions.md](decisions.md)。例は [examples/](examples/)（実際に使う開発フロー full-cycle-development を新構文で書いたもの）。現行正本 [`docs/workflow-yaml-syntax.md`](../../docs/workflow-yaml-syntax.md) からの変更点を中心に書き、変更のない部分は現行を参照する。未確定の論点は末尾に列挙する。

記法: YAML はフロースタイル（`{}`）を使わず、常にブロックスタイルで書く。

## トップレベル

```yaml
name: full-cycle-development
description: 入力収集 → spec 作成 → 実装 → レビューを human checkpoint 付きで一気通貫に実行する
schemas:
  # Contract 定義（現行のまま）
nodes:
  main:
    # root の Sequence
  # ...（全 Node のカタログ）
```

- **root は `main` という名前の node（規約）**。nodes に `main` が無ければ load 時 Diagnostic。WorkflowExecution は `main` から始まる。トップレベルに entry フィールドは持たない（C / Rust の main 関数と同じく、エントリポイントは規約名で決まる。root 名を変える実需が生じたら `entry:` を互換追加する）。
- 慣例: `main` は nodes の先頭に書く。
- root をインラインで書く形は無い。書き方は1つ（カタログ + main 規約）に揃える。

## nodes（カタログ）

- 全 Node のカタログ。**名前をキーにしたマップ**（リスト + `name` フィールドではない）。合成子（fanout / sequence）も葉（command / session）と同格に並ぶ。
- 単一名前空間。名前の一意性はマップ構造が保証する。
- Node の値は **kind ブロックちょうど1つ**（`command` / `session` / `fanout` / `sequence`）+ Node 共通フィールド。
- **Node 共通フィールド（`input` / `artifact` / `completion` / `worktree`）は kind ブロックの外（node レベル）に書く。kind ブロックの中は kind 固有の設定のみ**（session: model / permission / facets、fanout: children / items、sequence: entry / output / children）。
- **Node は遷移を持たない**。配線は合成子の `children` にのみ存在する。

## Node の Interface

- `input`: パラメータの**リスト**。要素は文字列（型なし）または名前キーのマップ（`- <パラメータ名>: <Contract 名>` = 型あり）。children の要素と同じ規則。**Contract を書くのは検証が要る場面**（fanout child の items 要素型検査など）だけで、session への文脈注入のように受けるだけなら型なし。供給元の型は kind / 宣言から自明。
- `artifact`: 産出する Artifact の Contract 名。
- **本文（command / テンプレート補間）はパラメータ名だけを参照する**（`{{ reviews }}` / `{{ thread.thread_id }}`）。兄弟 node 名の直書きはしない。特定 Node の Artifact を指す名前は配線（合成子側）にのみ現れる。
- fanout の `{{ item }}` 特殊名は廃止。items が供給されるパラメータの名前で参照する。

```yaml
judge:
  command: "echo '{{ reviews }}' | jq '{all_lgtm: all(.[].lgtm)}'"
  input:
    - reviews: review_verdicts
  artifact: judge_result
```

## sequence

```yaml
main:
  sequence:
    children:
    - run_tests
    - fix_tests:
        inputs:
          test_result: run_tests
        rules:
        - loop_guard:
            max_iterations: 3
            on_exhausted: give_up
        - next: run_tests
```

- `entry`: この sequence の開始 node（**children のエントリ名**を指す）。**省略時はリスト先頭**。先頭以外から始める場合に書く。
- `output`: `artifact` を宣言した部品 sequence が、**どの子の Artifact を自分の Artifact として返すか**の名指し（**children のエントリ名**。entry と対になる出口の指定）。artifact 宣言があれば必須、無ければ書かない。
- `children`: **リスト**。子ごとの扱い（データの配線と制御の配線）を1箇所に持つ。
  - **rules を持たないエントリの既定の辺は「リストの次のエントリへ（auto）」。リスト末尾なら終端**。分岐もループも無い直列は、名前・entry・rules なしで書ける:

    ```yaml
    sequence:
      children:
        - command: "cargo test --workspace"
        - command: "cargo clippy -- -D warnings"
    ```

  - `inputs`: `<パラメータ名>: <供給元>` のマップ。子のどのパラメータに何を渡すか。供給元は合成子の種別で決まる:
    - **sequence の子**: 兄弟エントリ名（field パス `<兄弟>.<field>` 可）/ 自分（この sequence）の input パラメータ名 / `request`
    - **fanout の子**: 自分（この fanout）の input パラメータ名（field パス可）/ `request` / `items`（展開の各要素）。fanout の子は並走し兄弟を持たないため、兄弟や他 node の直接参照は無い。外の値は親が fanout の input パラメータへ配線して渡す
    - `request` は起動時入力の**定義スコープの予約供給元**であり、定義内のどの合成子の配線からも直接参照できる
  - `rules`: 辺定義のリスト。中身（`when` / `switch` / `next` / `loop_guard`）と検証（排他・網羅・ループ健全性）は現行のまま。**辺に承認は置かない**（human が進行を止めたい箇所は Node 側の `completion: approval`）。**`rules: []`（空リスト）は出る辺なしの明示 = 終端**（リスト中間に終端を置く場合に使う）。
  - `on_failure`: この子が失敗したときの扱い。**省略時は中断**（resume で失敗した node を再実行 — 失敗は直すべきもの、が既定）。`ignore` = 失敗しても続行する（fanout では失敗子を結果の配列から除く。失敗 node の artifact に依存する下流があれば load 時 Diagnostic）。`retry: <n>` = 新しい attempt で最大 n 回自動再実行し（isolated なら attempt ごとに worktree 再生成）、尽きたら既定（中断）へ。失敗の重要度は文脈の性質なので、Node 定義ではなく扱い（children エントリ）に書く。
- **終端 = 出る辺（rules またはリストの次）が無い node**。children に載らず行き先参照だけされる node は次を持たないため終端。
- 配線の原則: **配線は、その node を子として扱う合成子が書く**。sequence が孫（fanout の child 等）に配線することはない。供給元の名前解決は合成子のスコープに閉じる（sequence = 自分の children と自分の input パラメータ、fanout = 自分の input パラメータ）。外の値は各階層が input パラメータとして受けて1段ずつ引き渡す。

## children の要素（4形式）

sequence / fanout の children は同じリスト形式で、要素は4形式。

```yaml
children:
  - review_opus                  # ① 文字列 = カタログ参照
  - fix_tests:                   # ② 参照 + 扱い（kind なしのマップ）
      inputs:
        test_result: run_tests
      rules:
        - next: run_tests
  - quick_check:                 # ③ インライン宣言（kind ありのマップ、キーが新名）
      command: "cargo check"
  - session:                     # ④ 無名エントリ（kind キーで始まるマップ）
      model: claude-opus-5
      permission: read
      facets:
        instruction: review-diff
    artifact: review_verdict     #    Node 共通フィールドは kind と並べて書く
```

- **名前は配線（entry / rules / inputs）から参照されるためにある。参照されないエントリは無名でよい**。fanout の子は配線されないため④が自然に書ける（無名の子でも `artifact` は fanout の子 artifact 配列に集約されるため意味を持つ）。sequence 内の無名エントリも、隣接辺（リストの次へ）で到達できるため合法。
- 判別: マップ要素のキーが単一の非予約語なら名前付き（②③。kind ブロックの有無で判別）、予約語で始まれば無名（④）。**予約語 = kind 名（`command` / `session` / `fanout` / `sequence`）とフィールド名（`input` / `artifact` / `completion` / `worktree` / `inputs` / `rules` / `on_failure` / `items` / `entry` / `output` / `children`）。予約語は node 名として使用禁止**。
- ③は**純粋な糖衣**である。インライン宣言された node の名前は、カタログに置いたのと同じ定義内の単一名前空間に登録され、load 時に「カタログ + 参照」へ正規化される。意味論は1つ。名前衝突は Diagnostic。
- 別の WorkflowDefinition を参照する構文は持たない。定義を跨ぐ部品化は定義言語の外（Lua・[#1591](https://github.com/siro33950/releash/issues/1591)）で行い、load 時には単一の WorkflowDefinition になっている（decisions.md「定義と展開」）。

## fanout

```yaml
fix_each:
  fanout:
    children:
    - fix_one:
        inputs:
          thread: items          # 展開の各要素をこのパラメータへ
          plan: plan             # 全子共通の供給（この fanout の input パラメータ）
    items: list_threads.threads
  input:
  - plan                         # 共通供給の受け口。親が fix_each のエントリで配線する
```

- `children`: 展開対象のリスト（sequence の children と同形式。上記「children の要素」参照）。子は配線されないため無名エントリ（④）も書ける。
- `items`: 展開する配列の定義（リテラル配列、または定義内の Artifact を産出する node の `<node>.<field>` 参照。fanout の子の Artifact は親の配列へ集約されるため参照不可）。**宛先と共通供給は children の inputs で書く**（sequence と同一の構文。供給元は自 fanout の input パラメータ / `request` / `items`）。展開の各要素は予約供給元名 `items` として配線する。child のパラメータが1つで items がある場合は宛先が一意なので inputs を省略できる。
- 失敗した子の扱いは children エントリの `on_failure`（sequence と共通。上記「sequence」参照）で書く。
- 子を隔離して並走させたい場合は、**fanout node の `worktree: isolated`**（Node 共通フィールド・kind の外）で宣言する。child の node 定義には書かない。`isolated` は子の実行ごとに親の worktree HEAD から branch + worktree を生成し、diff は branch に残る。

## command / session

- command: 現行のまま（シェルコマンド、標準結果、`artifact:` で stdout を Contract 検証）。
- session ブロックの中は実行設定のみ: **`provider`（必須。`claude` | `codex`）** / **`model`（省略可）** / **`permission`（省略可）** / `facets`。`gate` という語は使わない（完了の定義と進行のトリガーの混同を招くため廃止。completion に改名・意味論は現行実装のまま・Node 共通フィールドとして kind の外へ）。
- **`model` / `permission` の値域は provider CLI が定める**。Releash は値の写像・検証をせず、session 起動時に provider CLI への起動設定として注入する。workflow の再現性（誰が実行しても同じ構成で走る）のため、実行構成の正は workflow 定義に置く。session 開始後に CLI 内で設定が切り替えられても、起動時構成の正は定義側に残る。examples の `permission` 値を各 CLI の実語彙へ写像する作業は W1（#1461）実施時に確定する。
- 旧 milestone 84 AgentSessionConfiguration への従属（AgentMode 値域 / `goal` / `effort`）は廃止（同構成は MS87 で消滅）。
- `worktree`: `shared`（既定）| `isolated`。Node 共通フィールド。単独の session / command なら自身の実行を、fanout なら子ごとの並走を隔離する。

## completion（完了の定義）

全 Node 種別で宣言可能・省略可能。`completion: approval` は「**本来の完了条件を満たした後、human が承認するまで完了しない**」。承認までは観測・介入（session なら対話で指示し直し）ができ、却下や再実行という別操作は無い。承認主体は human のみ。

| Node | 既定（省略時） | `completion: approval` |
| --- | --- | --- |
| session | Submit と provider 停止が揃って完了（Artifact は任意添付。`artifact` 宣言がある node は検証済み Artifact を含む Submit のみ有効） | 二信号が揃った後、human の承認で完了 |
| command | exit code で完了 | 終了後、human の承認で完了 |
| fanout | 全子完了 | 全子完了後、human の承認で完了 |
| sequence | 終端 node への到達 | 到達後、human の承認で完了 |

## Contract / schemas / Diagnostic

現行正本（`docs/workflow-yaml-syntax.md`）のまま変更なし。schemas は WorkflowDefinition ローカル。

## 名前空間

| 空間 | スコープ |
| --- | --- |
| WorkflowDefinition 名 | アプリ内グローバル（builtin + user 定義）。人間が起動する単位 |
| node 名 | WorkflowDefinition 内フラット単一 |
| Contract（schemas）名 | WorkflowDefinition ローカル |
| facets 名 | アプリ管理の共有空間（現行のまま） |

## 未確定

なし（構文論点はすべて収束済み）。
