# ワークフローエンジン発展計画

この文書は Releash workflow engine の戦略、実行モデル、状態所有を定義する一次 Owner である。語彙は [`architecture/GLOSSARY.md`](./architecture/GLOSSARY.md)、定義構文は [`workflow-yaml-syntax.md`](./workflow-yaml-syntax.md)、ライフサイクル不変条件は [`../specs/workflow-lifecycle/workflow-ideal-lifecycle.md`](../specs/workflow-lifecycle/workflow-ideal-lifecycle.md) を正とする。完成形の例は [`../specs/unified-node-model/examples/full-cycle-development.yml`](../specs/unified-node-model/examples/full-cycle-development.yml) の1本だけを正とする。

## 目的

Releash は workflow を決定論的な実行レールとして扱う。開発者は WorkflowDefinition を定義し、WorkflowExecution として実行し、実行木の NodeExecution と Artifact を観測し、人間の判断点で承認・追加指示・中断・再開できる。

workflow aggregate は state transition の唯一の権威である。Agent、UI、CLI、API は action を要求できるが state を直接決めない。状態変更は typed command と durable fact を通る。

## 統一 Node モデル

Node は次の4種である。

| Node | 役割 | 構造 |
| --- | --- | --- |
| Session | provider CLI と継続対話する | 葉 |
| Command | 非対話 command を一度実行する | 葉 |
| Fanout | children を並列実行する | 合成子 |
| Sequence | children を時系列実行する | 合成子 |

合成子は任意の Node を child にできる。Sequence の子の Sequence は部品境界、Fanout の子の Sequence は並列 pipeline になる。定義を跨ぐ参照は持たず、Lua の `require` も load 後は単一の WorkflowDefinition になる。

## 定義と実行木

WorkflowDefinition は実行テンプレートであり、実行木そのものではない。実行木には実際に開始した NodeExecution だけが載る。分岐で選ばれなかった Node、まだ開始していない Node、未展開の Fanout child は実行木へ合成しない。loop と retry で複数回開始した Node は、それぞれ別の NodeExecution になる。

```text
Workspace
  └─ Worktree
       ├─ execution tree
       │    └─ Sequence / Fanout / Session / Command
       └─ execution tree
            └─ Session
```

workflow の木は定義 root `main` の実行インスタンスを公開 root とする。単独 Session も Session Node 1個を root とする実行木であり、別の実行モデルを持たない。

## completion と辺

completion は Node 自身が所有する「何をもって完了とするか」の定義である。辺は Sequence が children エントリで所有する「完了後にどこへ進むか」の定義である。この二つを混ぜない。

| Node | 既定の completion |
| --- | --- |
| Session | 同一 attempt の Submit と provider Stop が揃う |
| Command | process が終了する |
| Fanout | 全 child が決着する |
| Sequence | 終端 child へ到達する |

`completion: approval` は既定条件を満たした後、人間の Approve まで完了を保留する。approval を独立 Node や辺として表現しない。

辺は Sequence の children に置く `when` / `switch` / `next` / `loop_guard` と、rules 省略時の隣接辺である。現行 loader は後方辺の cycle 上に少なくとも一つの `loop_guard` を要求し、合成子の静的な包含 cycle も load 時に拒否する。Agent の追加指示による反復は turn 境界と fact を持ち、人間が観測・中断できる。

## 実行木全体の状態

WorkflowExecution が表す実行木全体の status は3値だけである。

| status | 意味 |
| --- | --- |
| Running | 木に継続または人間の操作を待つ実行がある |
| Completed | 定義された root の completion が成立した |
| Aborted | 人間の abort により木全体が終端した |

WaitingApproval、Paused、Failed、Waiting、Interrupted などの詳細状態は NodeExecution が所有する。木全体は Node 状態と durable fact から capability、現在位置、要約を導出し、詳細状態を追加の WorkflowExecution status として複製しない。

## 実行と復旧

- start は workflow 名、root Worktree、任意 request、起動元を受ける。
- Node の開始・完了・失敗・Submit・provider Stop・承認・retry は durable fact として記録する。
- Session completion の二信号は順不同で、同じ attempt に属するものだけを組み合わせる。
- Retry は新しい NodeExecution を作り、元 attempt との明示的な retry 関係を記録する。同名 Node の loop 再訪や別 Fanout lane は retry とみなさない。
- resume は確定済み Artifact と完了済み child を再利用し、未確定な実行だけを再開する。
- process や storage の喪失は Node の failure / interruption として記録し、結果を推測しない。
- recovery は execution ごとに独立して収束し、一つの破損した execution が他を永久に止めない。

## Worktree 所有

通常の実行木は root を植えた Worktree を参照し、その Worktree の作業状態を判断材料にする。`worktree: isolated` による隔離実行では、Node attempt が隔離 worktree の Releash 側 lifecycle state を所有する。Git working tree 自体は外部実体であり、成果の統合は無条件の engine action ではなく、人間または親 Session が判断して通常の Git 操作として行う。

定義での `worktree` 宣言は現時点では未解禁であり、loader は `WFU002` Error で拒否する。隔離 worktree の recovery state は将来の受理契約を先行して domain が所有している。

## Operation Surface

Tauri、WebSocket、CLI、将来の native client は同じ backend-owned state と usecase を読む。frontend は表示、入力、command 呼び出し、開閉などの UI state のみを持ち、実行開始済み Node の選別、status、capability、retry 分類を再計算しない。

CLI は現在、次の読み取り・Artifact 操作を公開する。

- `releash workflow status <execution-id>`
- `releash workflow output submit --node-execution <id>`
- `releash workflow output get <execution-id>`

## 進化原則

- engine の振る舞いは domain aggregate と決定サービスに置く。
- full-retention より fact、summary、page、ID operation を優先する。
- human checkpoint と観測可能性を後付けにしない。
- 文法に同じ意味の別記法や互換読み替えを追加しない。
- 未解禁 field は Diagnostic で拒否し、実行時に黙って無視しない。
