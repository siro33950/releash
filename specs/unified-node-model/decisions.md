# 統一 Node モデル

この文書は、Session / Command / Fanout / Sequence を単一の Node 概念に統一し、Worktree 配下の実行木として実行・観測・永続化するモデルを定義する。本モデルのマイルストーン・ISSUE は本書を元に作る。

構成: 本書（モデルの決定）／[syntax.md](syntax.md)（YAML 構文の確定分）／[examples/](examples/)（実例: full-cycle-development を新構文で書いた単一定義）。

語彙は [`architecture/GLOSSARY.md`](../../docs/architecture/GLOSSARY.md) を正とする。本書が導入する語彙は §語彙 に定義し、GLOSSARY への反映は実装マイルストーンで行う。

## 目的

Releash は Claude の Dynamic Workflows を決定論的な実行レールとして扱う。agent が実行時に作業を分割・展開しても、制御フロー（誰をいつ実行し、何を待ち、どう受理するか）は engine が決定論的に実行し、記録し、再開できる。

そのために、これまで別々に扱われてきた実行単位を単一の Node 概念に統一する。

- Session: 1 agent との対話
- Command: 1コマンドの実行
- Fanout: 複数の Node の並列実行単位
- Sequence: 複数の Node の時系列実行単位

「Workflow」の語は木全体の単位（WorkflowDefinition / WorkflowExecution）に残し、木の中の合成子とは区別する。WorkflowDefinition のトップレベルは root の Sequence を定義する。

Node は好きなように組み合わせられる。Sequence の子に Sequence を置けば部品化、Fanout の子に Sequence を置けば並列パイプラインになる。WorkflowExecution に属さない単独の Session も、1ノードの実行木として同じモデルに載る。

## プロダクト方針

- workflow engine は制御フローの唯一の権威である。agent がフローに影響できる経路は、記録される typed command（Submit。Artifact は任意添付で、delegate の発火もこれに乗る）のみ。agent の暗黙の振る舞いでフローは変わらない。
- human checkpoint は2箇所で担保する。Node の完了定義 `completion: approval`（承認主体は human のみ）と、Session への観測・介入（いつでも可能）。
- 隔離環境の成果の統合（merge）はフローではなく作業状態への操作である。engine は機械的・無条件の merge を行わない。統合は判断主体（親 session の agent または human）が Artifact と diff を確認した上で、通常の Git 操作として行う。
- 機械が勝手に回り続ける経路は静的に有界にする。非有界になれるのは agent の判断による反復だけであり、それはターン境界ごとに記録され、human が観測・中断できる。

## 中核モデル

### Node

実行の構成単位。種別は4つ。

```text
Session   -> 1 agent との対話（葉）
Command   -> 非対話の一回実行（葉）
Fanout    -> 子 Node 群の並列実行（合成子）
Sequence  -> 子 Node 列の時系列実行（合成子）
```

- 葉（Session / Command）は自分の仕事を終えるだけで、「そのあと何が起きるか」を知らない。
- 合成子（Fanout / Sequence）が子の実行順序・受理を定義する。
- 合成子の子には任意の Node を置ける。WorkflowDefinition のトップレベルは root の Sequence を定義する。

### 実行木

Node の実行インスタンスが成す再帰木。木は実行のレイヤーにのみ存在する。

- WorkflowDefinition（YAML）は Sequence / Fanout 部分のテンプレートであり、木ではない。分岐・ループは定義側の規則で、実行木には展開結果（実際に起きたこと）だけが載る。ループが3回回れば実行木には Node が3つ並ぶ。
- WorkflowExecution に属さない単独の Session は、定義なしで直接生える1ノードの実行木。workflow 配下の Session と単独の Session を実装上二分する区別（`AgentSessionOrigin` の Standalone / WorkflowNode、および実行木と別系統の standalone 一覧・別 query）は廃止する。区別は実行木上の位置で決まる。
- 実行木は root を植えた Worktree に所属する。階層は `Workspace → Worktree → 実行木（複数）`。実行木の集合に固有名は与えない。

### 完了の定義と辺

Node の完了と、完了後の進行は別の概念であり、所有者が異なる。

**完了の定義（completion）は Node 自身が持つ。** 何をもってこの Node が完了するかの宣言である。

```text
Session   -> completion: auto     … agent の Submit と provider CLI の停止が揃って完了
             （Artifact は任意添付。artifact 宣言がある node は
              検証済み Artifact を含む Submit のみ有効）
           | completion: approval … 二信号が揃った後、human の承認で完了。承認までは
             対話で指示し直せる。承認主体は human のみ。却下や再実行という別操作は無い
Command   -> exit code（宣言不要）
Fanout    -> 全子完了
Sequence  -> 終端 node への到達
```

**辺は進行の定義であり、Sequence が rules として持つ。** 前の Node の完了を受けてどこへ進むかの条件分岐（when / switch / next / loop_guard）のみで、承認を置かない。human が進行を止めたい箇所は、Node 側の `completion: approval` で表す。

- approval を独立した node 種別として持たない。
- 「gate」という語は使わない。「門 = 進行」を連想させ、完了の定義（Node の関心）と進行のトリガー（Sequence の関心）の混同を招いたため、completion に改名する。
- completion は全 Node 種別で宣言可能・省略可能。`completion: approval` は「本来の完了条件を満たした後、human が承認するまで完了しない」で、どの種別にも同じ意味論が適用される（Fanout なら全子完了 + 承認）。

改訂対象: evolution-plan / yaml-syntax の `gate`（auto / approval）を `completion` に改名する。意味論（session は Submit + provider 停止の二信号で完了・approval は承認されるまで完了しない）は現行実装のまま維持し、語と宣言位置だけを変える。

### 定義と展開

- 合成子の子には任意の Node を書ける。fanout 先・子 Node を制限する子専用型は持たない。
- **WorkflowDefinition を跨ぐ参照（サブワークフロー参照）は持たない。** 部品化と再利用は定義言語の外で行い、engine が受け取るのは常に単一の WorkflowDefinition である。部品は `input` パラメータと `output` を持つ Sequence として書き、親の children に置く。実行木にも子 Sequence として現れるため、折り畳み・`completion: approval` の単位は失われない。
- 定義言語の外での合成手段は Lua とする（[#1591](https://github.com/siro33950/releash/issues/1591)）。ファイル分割と再利用は Lua の `require` が担い、生成時に解決される。engine・永続化・resume は Lua を知らない。
- 定義の健全性は load 時に検証する: 合成子の包含循環（Sequence の children に自分自身が現れる）の拒否、未定義参照の Diagnostic。深さの上限は `MAX_NODES_PER_WORKFLOW` が与えるため、別途の深さ制限は設けない。
- ループの有界化ガード（loop_guard）は必須にしない。「条件が成立するまで回り続ける」定義は正当であり、その監督は実行中の観測・abort で行う。

実行木の有界性:

| 成長の経路 | 有界化の手段 |
| --- | --- |
| 定義の静的構造（合成子の入れ子） | load 時の包含循環検出 + `MAX_NODES_PER_WORKFLOW`（定義は単一なので、これが静的構造全体の上限になる） |
| 辺の後方参照ループ | loop_guard（オプショナル） |
| 実行時の子展開（Fanout items / delegate） | 実行時に決まるのは幅のみ（items の件数 / delegate の発火回数）。深さは定義の静的構造で固定され、展開された子が定義に無い子を生むことはない。delegate の child はさらに delegate できない |
| 反復 | agent の判断。ターン境界ごとに記録・観測・中断可能 |

### Fanout と delegate の境界

並列の動的さには出所が2つあり、別の機構として扱う。

| | Fanout | delegate |
| --- | --- | --- |
| 何者か | 定義に書かれる合成子 Node | Session の宣言的能力（Node 種別ではない） |
| 動的さの出所 | データ由来。items（前 Node の Artifact 配列）で子の数が決まる。YAML に記載された決定的振る舞い | 判断由来。親 session の Submit のたびに、宣言された child Node が起動する（発火回数は実行時に決まる） |
| 子の置き場所 | 実行木のノード | 実行木のノード。親 Session Node の部分木として発火ごとにぶら下がり、Workspace の実行木ツリーで観測する |

子展開・`worktree: shared | isolated` の機構は共有する。delegate の仕様は milestone #85 が正本であり、#85 は本モデルの構文（children / completion / 合成子の再帰解禁）に依存する（本モデル完了後に着手）。

### Worktree 実行コンテキスト

worktree は Node が親から継承する実行コンテキストであり、木の構造ではない。

- 子 Node は既定で親の worktree を継承する（`shared`）。
- `worktree: shared | isolated` の意味論（isolated = 実行ごとに親の worktree HEAD から branch + worktree を生成。diff は branch に残り後続 Node には見えない）は milestone #85 を継承する。
- 宣言は関心の所有者に置く。単独の Session / Command が隔離で実行される場合は自身の node 定義に書く。**子を隔離して並走させるかは Fanout の関心なので、Fanout ブロックに書く**（child の node 定義には書かない）。
- 新しい worktree が生まれるのは isolated が宣言された実行だけ。Workspace が勝手に worktree を取得することはない。
- worktree の出自は2種で、ライフサイクルと操作主体を分ける。

```text
① 人間が作る作業の場
   root Node を植える先。人間が第一級に選択・作成する。長寿命。
② isolated 宣言により生まれる隔離実行環境
   実行（attempt）ごとに親の worktree HEAD から自動生成。ephemeral。
   その実行が所有し、Worktree 管理 UI の一覧に混ぜない。
```

- isolated な子が別 worktree で実行されても、それは実行場所であって所属ではない。実行木の所属は root の Worktree に固定される。
- 「1 worktree に active WorkflowExecution は1つ」という現行制約は、shared worktree 上の実行中 Node 同士の書き込み競合の扱いとして再定義する。

### Worktree 出自の台帳と突合

①/②の判定はディスク上の worktree からの推測ではなく、生成時に記録された所有情報と実体の突合で行う。

- 台帳は事実ログに記録された worktree 関連の事実（隔離環境の生成・解放・喪失）とその導出 = 実行木の状態の一部。delegate の child も実行木の Node であり、その②も同じ台帳に載る（親 Session の delegate 状態という第二の台帳は持たない）。所有の実体と記録の場所が一致する。別台帳は新設しない。
- ②は専用パス + branch 命名規則で生成する（可読性と、台帳が読めない異常時のフォールバック判定）。
- 起動時に台帳と `git worktree list` を突合する:
  - 台帳は②と言うが実体が無い → 該当 Node を「隔離環境喪失」としてマークし、resume 不可を明示する。
  - 実体はあるが所有 Node が完了・破棄済み → 掃除候補として人間に提示する。成果未統合の worktree を機械的に削除しない。
  - 台帳に記録が無い worktree → ①として通常一覧に扱う。

### 永続化

- 正は SQLite 上の単一の正規化された事実ログテーブル。統一 Node の語彙でカラム化する:

  ```text
  node_events(tree_id, seq, node_execution_id, parent_id, node_name,
              kind, attempt, event_type, detail, timestamp)
  ```

- **行は純粋な事実のみ**（外部入力・人間の行動・実行した副作用の記録: started / submit_received / stop_received / approval_granted / abort_requested / process_exited など）。遷移イベント（completed 等）や導出結果（status カラム）は書かない。「事実は揃っているのに遷移記録が無い」という不整合が定義上存在せず、原子性への依存は単一行 append のみになる。
- **状態は読み取り側の導出**。完了・進行の規則（session の Submit + 停止、fanout の全子完了、sequence の前進）は Rust の domain が一元所有し、tree 単位の fold で導出する。SQL で状態判定を書かない（規則の二重実装禁止。カラムは tree / node / kind の絞り込み用）。導出結果は in-memory に保持してよいが、永続化された写し（projection 表・スナップショット文書・別台帳）は持たない。
- **engine の駆動は冪等な reconciliation ループ**: 導出された状態を見て、まだ実行していない行動を実行し、実行した事実を追記する。起動時復旧はこのループの1周目と同一であり、復旧専用経路を持たない。行動と記録の間の窓（副作用実行後・記録前のクラッシュ）は、provider lifecycle の識別子による実世界との突合で潰す。
- 規則の変更は過去ログの解釈に遡及する（「当時完了と判定した」という記録は持たない）。事実のみを記録する設計の代償として許容する。
- 単独 Session の生成も同じテーブルへの started の追記であり、木（tree_id）ごとの行の集合がそのまま実行木。実行木が持つのは構造・状態・参照（session 参照 / Artifact 参照 / worktree 参照）のみ。Session 本文（会話の正本は provider CLI の transcript）・Artifact 本体・Command output は各 store が所有したまま複製しない。

## 採用するもの

| 項目 | 方針 |
| --- | --- |
| Node 統一 | Session / Command / Fanout / Sequence の4種。葉と合成子。「Workflow」は木全体の単位（Definition / Execution）に残す。 |
| 実行木 | 実行インスタンスのみが木を成す。Worktree に所属。単独 Session も1ノードの木。 |
| completion | 完了の定義は Node 自身が持つ（Session: auto / approval 等）。gate から改名。session の完了は Submit + provider 停止の二信号（Artifact は任意添付）。 |
| 辺 = 条件分岐のみ | 辺（rules）は Sequence が所有し、条件分岐（when / switch / next / loop_guard）のみ。承認は辺に置かない。 |
| 再帰定義 | 合成子の子に任意の Node。定義を跨ぐ参照は持たず、部品化は定義言語の外（Lua）で行う。包含循環の検出で防御。 |
| worktree 継承 | 実行コンテキストとして親から継承。隔離の宣言は関心の所有者に置く（単独 Node は自身の定義、並走の隔離は Fanout ブロック）。出自2種を分離。 |
| 台帳突合 | 永続化された実行状態を台帳に、起動時に実体と突合。未統合成果は機械的に削除しない。 |
| 純粋事実ログ | 正は node_events 単一テーブル（木 = tree_id の行集合）。遷移イベントなし。状態は読み取り側の Rust fold で導出し、engine は冪等 reconciliation ループで駆動する。 |
| 実行木 UI の完全性 | 起きた実行はすべて行に出す。retry は attempt ごと、delegate は発火ごとに行が並ぶ。番号ラベルは表示しない（順序は並びでわかる）。決着済みの過去はデフォルト折り畳み。 |

## 採用しないもの

| 項目 | 理由 |
| --- | --- |
| 「forest」の命名 | Worktree（木）の下に森は比喩が逆立ちする。「Worktree 配下の実行木」で足りる。 |
| 「gate」の命名と辺の approval | gate は完了の定義（Node の関心）と進行（Sequence の関心）の混同を招いた。completion に改名し、辺には承認を置かない。 |
| 定義を跨ぐ参照（`ref`） | 合成機構を engine に持ち込むと、合成子境界で loop_guard 検知・ステップ計算・静的検証・再開位置が壊れることが先行実装で実証されている（TAKT / Argo Workflows）。合成は定義言語の外に置き、engine には常に単一定義を渡す。部品 Sequence は `input` / `output` を持つため、`ref`（入口 `request` の String 1個・出口なし）の上位互換になる。 |
| loop_guard の必須化 | 「終わるまでやれ」が正当なユースケース。監督は観測・abort で行う。 |
| merge の typed command 化 | merge はフローに影響しない作業状態への操作。通常の Git 操作でよい。 |
| workspace 横断監督 view | 実行木の所属が root Worktree に固定されるため、Worktree 単位の view で監督が完結する。 |
| 遷移イベント・status カラムの記録 | 導出結果の記録は書き込み側に判断とクラッシュ窓（事実は揃ったが遷移が未記録）を残す。事実のみを記録すれば、その不整合クラスが定義上存在しない。 |
| 永続化された導出状態（projection 表・スナップショット） | 正が2つになる。導出は in-memory・再構築可能に限る。ログテーブル自体が正規化されており検索もそこで足りる。 |
| 再帰 delegate | 無限増殖の防止。milestone #85 で確定済み。 |

## 既存文書・実装との関係

| 資産 | 関係 |
| --- | --- |
| milestone 82（新モデル移行） | 前提。command / session / fanout、Contract 検証済み Artifact、typed command、resume の成果の上に立つ。MS82 由来の blob イベント + replay projection 構成は §永続化 の事実ログへ置き換える。 |
| milestone 87（Agent TUI 移行） | 前提。session の実体は provider CLI の TUI（Terminal Surface）であり、会話の正本は provider の transcript。完了の二信号（Submit + provider Stop）と「Workflow 全体は Running / Completed / Aborted の3値・詳細状態（WaitingApproval / Paused / Failed）は Node 所有」を継承する。**Standalone AgentSession を実行木と別系統（別 query・別一覧）に置く MS87 の扱いは、本モデルが上書きする**（単独 Session も1ノードの実行木）。 |
| [#1454](https://github.com/siro33950/releash/issues/1454)（Node 中心再帰ツリー UI）/ [#1512](https://github.com/siro33950/releash/issues/1512)（実行開始済み Node のみ表示） | UI 骨格の先行実装。合成子は branch、中央表示は単一 NodeContentView、木には起きた実行だけが載る（#1512 契約）。本モデルはその backend 正本化と一般化。ただし「retry で行を増やさない（最新 attempt のみ関連付け）」の既定は本モデルで改訂 — 起きた実行はすべて行に出す（§採用するもの）。MS87 後に standalone AgentSession が木と別リストになっている点は本モデルで木へ統合する。 |
| milestone 85（delegate + worktree 隔離） | **#85 は本モデル完了後に着手する（依存: children 構文・completion・合成子の再帰解禁）**。delegate の発火は親 session の Submit、child は任意の Node、completion の条件合成（when / and / or）は #85 の文法追記。`worktree: shared \| isolated` の意味論は #85 の確定判断を継承する。 |
| milestone 84（Agent チャット安定化） | 制御フローは独立。旧記述「session の実行設定の語彙は MS84 の AgentSessionConfiguration に従う」は廃止（同構成は MS87 で消滅）。**session の実行設定（provider / model / permission）は workflow 定義が持ち、値域は provider CLI の語彙に従う**。Releash は写像・検証をせず、session 起動時に CLI への起動設定として注入する（workflow の再現性のため、実行構成の正は定義に置く）。 |
| `docs/workflow-engine-evolution-plan.md` | 「NodeDefinition 種別は command / session / fanout の3つ」「完了判定は session の gate」（gate → completion 改名・意味論維持）が改訂対象。改訂は実装マイルストーンの文法正本化 wave で行う。 |
| `docs/workflow-yaml-syntax.md` | 改訂対象。改訂内容の確定分は [syntax.md](syntax.md) が正本（トップレベル = nodes カタログ + main 規約、sequence = entry + output + children、Interface とデータ配線の分離、completion、worktree ほか）。改訂は同上。 |
| `docs/architecture/GLOSSARY.md` | §語彙 の反映。同上。 |
| [`../workflow-lifecycle/workflow-ideal-lifecycle.md`](../workflow-lifecycle/workflow-ideal-lifecycle.md) | 改訂対象。同文書は6値 ExecutionStatus 前提で書かれており、MS87 後の実装（木全体 = Running / Completed / Aborted の3値、WaitingApproval / Paused / Failed は Node 所有）に対して stale。本モデルの文法正本化 wave（#1468）で、3値 + Node 所有状態の形へ不変条件を書き直す。 |

## 語彙

GLOSSARY への反映は実装マイルストーンで行う。

- **実行木（execution tree）**: Worktree に所属する、Node 実行インスタンスの再帰木。単独の Session は1ノードの実行木。
- **Sequence**: 子 Node 列の時系列実行を定義する合成子。WorkflowDefinition のトップレベルは root の Sequence を定義する。「Workflow」の語は木全体の単位（WorkflowDefinition / WorkflowExecution）にのみ使う（GLOSSARY で裸の Workflow は既に使用禁止語）。
- **辺（edge）**: Sequence 内の継ぎ目。rules（条件分岐）または隣接（リストの次へ）で定義される。承認は持たない。
- **completion**: Node 自身が持つ完了の定義。全 Node 種別で宣言可・省略可（既定は種別ごとの完了条件、approval = 既定 + human 承認）。現行の `gate` から改名（gate は使用禁止語へ）。
- **Node**: 実行の構成単位（4種）。GLOSSARY の NodeExecution との対応（実行木のノードへ一般化するか、新語を立てるか）は実装マイルストーンの語彙設計で確定する。
- Workspace 構造: 「Workspace 配下に WorkflowExecution / Session / Command が並列」を「Worktree 配下の実行木」へ改める。
- Worktree の状態所有: ①は従来通り外部実体（Releash は所有しない）。②隔離 worktree は生んだ Node が所有・ライフサイクル管理する。
