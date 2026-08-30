# Milestone 85「Session delegate と Node worktree 隔離」設計

## 1. 目的

この milestone は workflow engine に2つの機能を入れる。

- **Session delegate**: Session Node が、自身の文脈を保ったまま子 Node を起動し、その結果を受けて続行する機構。
- **Node worktree 隔離**: Node の実行を、親とは別の git worktree（および branch）で行う宣言。

両者は独立しており、delegate が worktree 隔離に依存する（delegate の child も隔離できる）。

## 2. Session delegate

### 2.1 定義

delegate は **同一 session 継続機構**である。親 session が文脈を保ったまま、child の結果を受けて自分で続行する。

現行モデルでも「実装 → 検証 → 通るまで直す」は辺のループで書けるが、後方辺で戻ると `start_node_instance` が新しい `node_execution_id` と attempt を作り、前の Artifact を破棄するため、戻り先は別 session になる。delegate はこの往復と構造が同じで、**戻り先が同じ session である**点だけが違う。

並列は child に置いた Fanout、直列は child に置いた Sequence が担う。delegate 自身は並列も直列も持たない。

### 2.2 Session が所有する理由

辺は Sequence が所有する（Node は遷移を持たない）。Sequence の辺として表そうとすると、戻り先の Node は完了済みであり、完了した Node へ戻る唯一の手段が新 attempt の生成＝session を捨てる操作になるため、「完了した Node を同じ session で再開する」という矛盾が生じる。

Session が持てば、親は**提出しても完了していない**状態で child を待つだけなので、完了の意味も NodeExecution の同一性も壊れない。したがって delegate は completion の一種であり、`approval` と同型（本来の完了条件に条件を足して完了を保留する）である。

### 2.3 構文

```yaml
implement:
  session:
    provider: codex
    facets:
      instruction: implement
  artifact: implement_result
  completion:
    require: approval          # 任意。全 Node 種別で宣言できる
    delegate:                  # 任意。Session だけが宣言できる
      child: verify            # 必須。任意の Node を名前で参照する
      inputs:                  # child が input を宣言しているとき
        task: implement_result
        spec: spec
      when: child.judge.clean  # 必須。述語
      max_iterations: 3        # 必須
```

`completion` は map であり、`require` と `delegate` を並べる。両方あるときの意味は and（述語が成立し、かつ human が承認して完了）で、or は持たない。

`require` の値域は `approval`。要求しないなら `require` を書かない。現行の `completion: auto` / `completion: approval` という文字列形式は廃止し、`auto` という値も持たない（要求を書かないことが自動完了を意味する）。

`inputs` は合成子の children エントリと同形で、`<パラメータ名>: <供給元>` を書く。「配線は、その Node を子として扱う側が書く」という現行の原則に従い、child を子として扱う親 session が配線を持つ。供給元は親 session のスコープで参照できるものすべてである（親の `input` パラメータ、親の Artifact、`request`）。親の Artifact には前ラウンドの結果である `child` キーも含まれる。

### 2.4 発火

**発火は Artifact 提出である。** 提出のたびに child が起動する。

発火を専用の typed command にしない理由は、agent が呼ばない選択をできてしまい、engine が制御フローの唯一の権威であるという不変条件が崩れるためである。提出を起点にすれば、agent は発火を回避できない。

「Node の完了」を発火点にしない理由は、完了は Node の終わりであり、完了してから session を維持して作業を続けるのは完了の意味を壊すためである。

### 2.5 述語と評価

`when` は真偽値を返す述語で、`and` / `or` で合成できる。原子は Artifact の required boolean field への参照である。

```yaml
when: done                       # 親の Artifact の field
when: child.judge.clean          # child の Artifact
when:
  and:
    - done
    - child.judge.clean
when:
  or:
    - child.judge.clean
    - child.scan.skipped
```

**評価は「何かが完了したとき」に行う。** 具体的には親が提出したときと child が完了したときで、参照先が何であるかによる場合分けはしない。参照先が未確定なら false になる（現行の辺の評価が `value.get(on).and_then(as_bool).unwrap_or(false)` であるのと同じ）。

```
親が提出したとき   真 → 完了（child は起動しない）／偽 → child を起動
child が完了したとき 真 → 完了／偽 → child の結果を注入して親が続行
```

この規則により、次の2つがどちらも同じ構文で書ける。

- 親の Artifact を見る述語: 親が「もう仕事がない」と判断した提出で、child を起動せずに完了する
- child の Artifact を見る述語: child の検証が通るまで、親が結果を受けて直し続ける

### 2.6 上限

`max_iterations` は必須で、**上限に達したら `when` の評価をスキップして完了させる**能力である。

```
max_iterations: 3 のとき

提出1 → 評価 偽 → child 起動(1) → 完了 → 評価 偽 → 注入 → 続行
提出2 → 評価 偽 → child 起動(2) → 完了 → 評価 偽 → 注入 → 続行
提出3 → 評価 偽 → child 起動(3) → 完了 → 評価 偽 → 注入 → 続行
提出4 → child は上限まで起動済み → 起こさず完了
```

child は上限回数だけ起動し、すべて注入されて親が判定を受ける。上限に達した後の提出で完了する。

上限に達して完了した場合、述語は false のままなので、その後の分岐は辺の `when` でそのまま区別できる。delegate に遷移先（`on_exhausted` 相当）は持たせない。辺は Sequence が所有するため、Session が遷移先を知ることはできない。

必須にする理由は、辺の cycle に `loop_guard` を必須としている（`WFC005`）のと同じである。

### 2.7 親 Artifact の構造

親が提出した Artifact に、engine が `child` キーを足す。

```
親が提出:      { done: false, tasks: [...] }
engine が追加: { done: false, tasks: [...], child: null }
child 完了後:  { done: false, tasks: [...], child: <child Node の Artifact> }
```

`child` の下は child Node の Artifact そのものであり、child 名は挟まない。したがって child の kind によって参照の形が変わる。

| child の kind | 参照 |
| --- | --- |
| Session / Command | `child.passed` |
| Sequence | `child.<子Node名>.passed` |
| Fanout | `child.<添字またはchild名>.passed` |

`child` は予約キーであり、親の Artifact Contract に `child` field があれば load 時 Diagnostic になる。型検査は、親 Contract の `properties` に `child` → child Node の Artifact schema を足した合成 schema で静的に解決する。

複数ラウンド回った場合、`child` は最後の結果で上書きされる。

### 2.8 実行木での表現

child の NodeExecution は親 session Node を親に持つ部分木として実行木に載り、発火ごとに attempt が増える。

```
main (sequence)
  └ implement (session)  attempt 1        ← 同じ session が続く
      ├ verify  attempt 1  ← 1回目の発火
      ├ verify  attempt 2  ← 2回目の発火
      └ verify  attempt 3  ← 3回目の発火
```

現行の実行木は retry とループ再訪を区別しておらず、`same_retry_target` は node_name と parent の一致だけを見る。delegate の発火も同じ扱いで、既存の `attempt` / `past_attempt_ids` / `is_retry_history` にそのまま乗る。UI 側に追加は要らない。

内部の親参照型 `ExecutionParentRef` は現在 sequence child と fanout child の2種しか持たず、doc も「親は合成子（sequence / fanout）の実行インスタンスを指す」となっている。delegate child は葉である Session を親とするため、3種目として追加する。

### 2.9 resume

child の再開は Fanout の子と同じ扱いで、完了済みの child は Artifact を再利用し、未確定のものだけ再実行する。

delegate 固有の扱いは、**child の結果の親 session への注入を事実として記録する**ことである。child が完了した後・注入が済む前に中断した場合、resume 時に未注入であれば注入する。これにより二重注入も取りこぼしも起きない。

親 session の provider session が復元できない場合は resume が成立せず、既存の失敗経路（`on_failure` / 手動 Retry）に委ねる。これは delegate 固有の扱いではない。

## 3. Node worktree 隔離

### 3.1 宣言と継承

`worktree: shared | isolated` は Node 共通 field である。省略時は `shared`（親から継承）。

宣言した Node がその実行で隔離される、という一つの規則で全種別を説明する。

| 宣言場所 | 動作 |
| --- | --- |
| Fanout | Fanout の実行が1つの隔離 worktree を持ち、children は全員そこで動く |
| Fanout の children | children ごとに隔離 worktree ができる |
| Sequence | Sequence の実行が隔離 worktree を持ち、children は全員そこで動く |
| Session / Command | その Node の実行が隔離 worktree を持つ |
| delegate の child | child の実行が隔離 worktree を持つ |

並列に動く children が同じ隔離 worktree で書き込めば衝突するが、読み取りだけの children（レビュー、チェック）なら衝突しないため、engine は禁止しない。定義の書き方の問題として扱う。

### 3.2 生成と記録

`isolated` の NodeExecution は attempt ごとに、親 worktree の HEAD から branch と worktree を生成し、そこを cwd として実行する。

隔離 worktree の台帳と reconciliation は milestone 86 W7（#1467）で実装済みである。

- 事実: `NodeFact::IsolatedWorktreeCreated` / `IsolatedWorktreeReleased` / `IsolatedWorktreeLost`
- 台帳: 事実ログから導出する `IsolatedWorktreeLedgerSnapshot`
- 起動時の突合: `worktree_reconciliation`（実体喪失 / 所有者終了済み / 台帳外）
- 命名: `isolated_worktree_branch` / `isolated_worktree_path`

本 milestone で実装するのは、`worktree` field の解禁（現行 loader は `WFU002` Error で拒否する）、実行時の branch + worktree 生成と cwd 適用、および観測経路への露出である。

### 3.3 統合

engine は統合（merge）を一切行わない。隔離 worktree の成果は branch に残り、親 worktree には現れない。統合は判断主体（親 session の agent、または human）が diff を確認したうえで、通常の Git 操作として行う。

逐次 Node で `isolated` を使った場合、その diff は branch に残り後続 Node からは見えない。これは仕様である。

## 4. Artifact 構造

engine が組み立てる Artifact の形を、次のように定める。

### 4.1 Sequence

`output` と `artifact` 宣言を廃止し、**children の Artifact を child 名をキーとする map** として返す。

```json
{ "implement_task": {...}, "verify_task": { "complete": true } }
```

現行は `output` で名指しした1つの child の Artifact を返し、`artifact` で Contract を宣言していた。廃止により、Contract 宣言も不要になる（Fanout が Contract を宣言しないのと同じく、engine が組み立てるため）。

通らなかった child は map に現れない。ループで複数回通った child は最後の結果が残る（現行の `sequence.artifacts` の挙動と同じ）。

### 4.2 Fanout

children の Artifact を **map** として返す。キーは `items` の有無で決まる。

```json
// items なし: キーは child 名
{ "run_lint": {...}, "run_test": {...}, "run_build": {...} }

// items あり: キーは添字
{ "0": {...}, "1": {...}, "2": {...} }
```

`items` があると同じ child が複数展開されて名前で一意に引けないため、添字をキーにする。`items` と複数 children が同時にある場合も、展開は `(item_index, child_index)` のフラットな並びであり、階層は作らない。

配列ではなく map にする理由は2つある。`on_failure: ignore` で除外された slot があっても他のキーがずれないこと、および `isolated` の Node にメタデータを足す場所が必要なことである。

### 4.3 isolated な Node

`isolated` を宣言した Node の Artifact に、engine が `worktree` キーを足す。

```json
{ "passed": true, "worktree": { "branch": "releash/isolated/...", "path": "/repo-worktrees/..." } }
```

全 Node 種別で同じ形になる。`worktree` は予約キーであり、同名の field や child 名があれば load 時 Diagnostic になる。

`artifact` を宣言しない Node でも、`isolated` なら `worktree` キーだけを持つ Artifact が生まれる。したがって Artifact の有無は「`artifact` 宣言があるか、`isolated` であるか」で決まる。

Command の Artifact に engine が `ok` / `exit_code` / `stdout` / `stderr` / `duration` を合成し、Contract に再宣言させないのと同じ仕組みである。

## 5. 参照

**複数段参照を全経路で解禁する。** 現行で1段に制限されているのは次の5箇所で、いずれも「2段以上を拒否する」検査を「各段が解決できるか」の検査に置き換える。

| 箇所 | 現行の制限 |
| --- | --- |
| 配線 `inputs` | `<name>` または `<name>.<field>` の1段 |
| 辺の述語 `when.on` / `switch.on` | 自 Node Artifact の required field 1つ |
| `env` | input パラメータとその1段の field |
| テンプレート `{{ }}` | `{{ parameter }}` / `{{ parameter.field }}` |
| `fanout.items` | Artifact の `<node>.<field>` 配列 |

1段に制限する根拠は正本のどこにも記録されていない。`SchemaDef` は Object の `properties` と array の `items` を保持しているため、多段でも load 時に静的に辿れる。

Sequence が統合 map を返し、Fanout も map になり、delegate の親が `child` キーを持つため、値を取り出す経路は多くが2段以上になる。解禁はこれらの前提である。

## 6. 述語の共通化

辺の `when` と delegate の `when` は同じ述語構造を持つため、**真偽値の合成だけを担う値オブジェクト**としてコードで共通化する。

```rust
pub enum Predicate<R> {
    Ref(R),
    And(Vec<Predicate<R>>),
    Or(Vec<Predicate<R>>),
}
```

共通化するのは論理式の構造、評価（論理演算）、および「各 Ref が boolean を指すか」という型検査の枠である。参照の解決は各所が自分のスコープで行うため、起点を揃える必要はない。

現行の `Rule::When` は `on: String` を直接持ち、述語を表す型が存在しない。評価も `routing.rs` の3行に埋まっている。ここを `Predicate` に置き換える。

辺の `when` にも and / or が使えるようになる。`switch` は string enum の多分岐であり、述語ではないため対象外。

## 7. 現行からの変更一覧

| 対象 | 現行 | 変更後 |
| --- | --- | --- |
| `completion` | `auto` / `approval` の文字列 | map。`require: approval` と `delegate` |
| Session | delegate を持たない | `completion.delegate` を宣言できる |
| Sequence | `output` で1つ返す。`artifact` で Contract 宣言 | 常に children の統合 map。`output` と `artifact` 宣言は廃止 |
| Fanout | children の実行順配列 | children の map（キーは child 名または添字） |
| `worktree` field | `WFU002` Error で拒否 | `shared` / `isolated` を受理し実行する |
| Artifact の有無 | `artifact` 宣言の有無 | `artifact` 宣言、または `isolated` |
| 参照 | 1段のみ | 多段 |
| 辺の `when` | boolean field 1つ | `Predicate`（and / or 合成可） |
| `ExecutionParentRef` | sequence child / fanout child | + delegate child |

正本ドキュメント（`docs/glossary/WORKFLOW.md`、`docs/glossary/DOMAIN.md`）と正本サンプル（`workflows/examples/full-cycle-development.yml`）は、上記に合わせて更新する。
