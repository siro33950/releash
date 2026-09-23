# Workflow の概念

この文書は、Releash の workflow を書く人に向けて、定義に書く要素がそれぞれ何を意味し、実行時にどう振る舞うかを説明します。書き方は [YAML リファレンス](./yaml.md) と [Lua リファレンス](./lua.md)、両方に共通する規則と一覧は [共通リファレンス](./common.md) を参照してください。

## workflow

workflow は、AI agent の作業と shell command を組み合わせた手順の定義です。

- 定義は YAML か Lua で書き、ファイルとして置きます。YAML と Lua は同じ定義になり、どちらで書いても実行時の振る舞いは変わりません。
- 定義は、手順の部品である Node の集まりです。実行は `main` という名前の Node から始まります。
- 定義は、worktree を選んで起動したときに実行されます。起動時には依頼文（request）を入力でき、定義の中から参照できます。
- 定義に誤りがあると Diagnostic が出ます。`error` の Diagnostic が一つでもある定義は起動できません。

## Node

Node には4つの種類があります。

| 種類 | 何をするか | いつ完了するか | Artifact |
| --- | --- | --- | --- |
| Session | provider CLI（claude / codex）を起動し、agent に作業させる | agent が成果を提出し、応答を終えたとき | 宣言した Contract の object |
| Command | shell command を一度実行する | command が終了したとき | 標準の結果 field と、宣言した Contract の object |
| Sequence | 子の Node を順番に実行する | 最後まで進んだとき | 通った子の Artifact を集めた object |
| Fanout | 子の Node を並列に実行する | すべての子が終わったとき | 子の Artifact を集めた object |

- Sequence と Fanout の子には、4種類どれでも置けます。Sequence の中に Fanout を置くように、入れ子にできます。
- 一つの Node は、一つの Sequence / Fanout の子にしかなれません。

## Artifact

Artifact は、Node が完了したときに残す JSON object です。後続の Node はこれを input として受け取り、分岐の条件にも使います。

### Session の Artifact

Session に Contract を宣言すると、agent はその Contract に合う JSON を提出しなければ完了できません。

```json
{ "title": "ログイン画面を直す", "steps": ["原因を調べる", "修正する"], "risk": "low" }
```

Contract を宣言しない Session は Artifact を持ちません。ただし、別の worktree に隔離した場合は `worktree` だけを持つ Artifact になります。

### Command の Artifact

Command の Artifact には、宣言しなくても次の field が入ります。

| field | 型 | 内容 |
| --- | --- | --- |
| `ok` | boolean | 終了コードが 0 で、Contract を宣言した場合は stdout がそれに合ったときに true |
| `exit_code` | integer | 終了コード |
| `stdout` | string | 標準出力。100KB を超えると切り詰めて `... (truncated)` を付ける |
| `stderr` | string | 標準エラー出力。切り詰めは `stdout` と同じ |
| `duration` | integer | 実行時間（ミリ秒） |

Command に Contract を宣言すると、stdout 全体を JSON として読み、Contract に合えばその field を上の field と同じ object に加えます。

```json
{ "count": 3, "ok": true, "exit_code": 0, "stdout": "{\"count\": 3}", "stderr": "", "duration": 12 }
```

- 終了コードが 0 以外、または stdout が Contract に合わない場合、Command は `ok: false` で完了します。Node の失敗にはなりません。
- command を起動できなかった場合は、新しい attempt で最大 4 回、自動で起動し直します。使い切った Node は実行中のまま、プロセス無しで利用者の Retry を待ちます。

### Sequence の Artifact

Sequence の Artifact は、通った子の Artifact を、子の名前をキーにして集めた object です。

```json
{ "make_plan": { "title": "...", "steps": ["..."], "risk": "low" }, "count_steps": { "count": 3, "ok": true, "exit_code": 0, "stdout": "...", "stderr": "", "duration": 12 } }
```

- 通らなかった子と、Artifact を持たない子はキーに現れません。
- ループで同じ子を何度も通った場合は、最後の Artifact が残ります。

### Fanout の Artifact

Fanout の Artifact は、子の Artifact を集めた object です。キーの付け方は、要素ごとに展開するかどうかで変わります。

- 要素ごとに展開しない場合、キーは子の名前です。

```json
{ "lint": { "ok": true, "exit_code": 0, "stdout": "", "stderr": "", "duration": 900 }, "test": { "ok": false, "exit_code": 1, "stdout": "", "stderr": "...", "duration": 4200 } }
```

- 要素ごとに展開する場合、キーは 0 から始まる展開順の番号です。

```json
{ "0": { "approved": true }, "1": { "approved": false } }
```

- Artifact を持たない子のキーは残り、値が `null` になります。

## Contract

Contract は Artifact と input の型です。JSON の object・array・string・boolean・integer・number を、必須の field と string の列挙値付きで表します。

- Session と Command の Artifact の Contract は object にします。
- 分岐の条件に使う field は、Contract の必須 field にします。
- `worktree` と、Command の標準の結果 field（`ok` など）は Releash が加える field なので、Contract に宣言できません。

## input と配線

Node は、受け取るパラメータを input として宣言します。値は、その Node を子に持つ Sequence / Fanout の側で、どこから渡すかを配線します。

- Node の本文（Session の facet、Command の command）は、パラメータ名だけを参照します。値の出どころは配線にだけ書きます。同じ Node を別の場所で使っても、本文を変える必要はありません。
- input には型を付けられます。型を付けると、渡す値と参照する field が、定義を読み込むときに検査されます。

配線で値の出どころにできるもの（供給元）は次のとおりです。

| 供給元 | 値 |
| --- | --- |
| request | 起動時に入力した依頼文（string） |
| 兄弟の Node | 同じ Sequence の中で先に実行された Node の Artifact |
| 兄弟の Node の field | その Artifact の field。`.` で区切って何段でも辿れる |
| 親のパラメータ | 親の Sequence / Fanout 自身が受け取った input |
| items | Fanout が展開している要素（Fanout の子だけ） |

- Fanout の子は並列に動くので、兄弟の Artifact を受け取れません。外の値は親の input を経由して渡します。
- Sequence や Fanout の Artifact の中も、`<Sequence>.<子>.<field>` や `<Fanout>.<キー>.<field>` のように辿れます。

## 進行と分岐

Sequence は、子を並べた順に進みます。

- 分岐の指定が無い子の後は、次の子へ進みます。最後の子の後で Sequence は終わります。
- 開始する子を指定できます。指定しなければ先頭から始まります。
- 子ごとに、次にどこへ進むかを指定できます。

| 指定 | 振る舞い |
| --- | --- |
| 無条件 | 常に指定した Node へ進む |
| 真偽 | Artifact の boolean field が true なら一方、false ならもう一方へ進む。複数の field を and / or で組み合わせられる |
| 列挙 | Artifact の string field の値ごとに進む先を決める |
| 回数制限 | この子へ進もうとしたとき、すでに上限の回数だけ実行していたら、代わりに指定した Node へ進む |
| 終了 | どこにも進まず、Sequence を終える |

- 条件に使う field が Artifact に無い場合、真偽の条件は false として扱います。列挙の条件は、どの値にも当たりません。
- 前の子へ戻ってループを作る場合は、ループのどこかに回数制限が必要です。
- 比較や計算の式は書けません。分岐に使う boolean や列挙値は、Session か Command の Artifact として作ります。

## 並列

Fanout は、子を並列に実行します。

- 子を並べるだけの場合、並べた Node がそれぞれ一度ずつ動きます。
- 配列を要素ごとに展開する場合、要素の数だけ子を動かし、各要素を子の input に渡します。配列には、Artifact の配列 field か、配列の値を使います。
- 要素ごとの展開と複数の子を組み合わせると、要素 × 子の数だけ動きます。

## 完了

Node の完了条件は種類ごとに決まっています（[Node](#node) の表）。Session の完了は次のとおりです。

- agent は、作業を終えたら提出コマンドを実行して成果を提出し、応答を終えます。この2つが揃うと Session は完了します。順番は問いません。
- 提出コマンドの使い方は、Releash が agent への指示の末尾に書き加えます。定義や facet に書く必要はありません。
- Contract を宣言した Session では、Contract に合う成果を含む提出だけが受け付けられます。

完了には、次の2つを加えられます。

### 承認

承認を要求した Node は、完了条件を満たした後に承認待ちになり、人が承認すると完了します。4種類どの Node にも指定できます。

### delegate

Session に delegate を指定すると、agent が成果を提出するたびに別の Node（child）を実行し、その結果を同じ会話に返します。

1. agent が成果を提出すると、条件を評価します。true なら child を実行せずに完了へ進み、false なら child を実行します。
2. child が終わると、もう一度条件を評価します。true なら完了へ進み、false なら child の結果を agent に渡し、agent は作業を続けて再提出します。
3. child を実行した回数が上限に達した後の提出では、条件を評価せずに完了へ進みます。

- 条件には、提出した成果の field と、直近の child の Artifact の field を使えます。
- 親の Artifact には、直近の child の Artifact が `child` として入ります。child を実行する前は `null` です。
- 承認も要求している場合は、delegate の条件が揃った後に承認待ちになります。

## 起動の失敗と利用者の操作

Session / Command の起動に失敗すると、1、2、4、8 秒の間隔で最大 4 回、新しい attempt を作って起動し直します。使い切った Node は実行中のまま利用者の操作を待ちます。プロセスが居るかは Node の状態とは別に表示され、プロセスが居ない Node は介入待ちとして分類されます。Session の異常終了も Node の失敗状態にはしません。

Command が 0 以外で終了した場合は、`ok: false` の Artifact を出して完了します。次の操作は workflow 定義の分岐で決めます。

| 操作 | 対象と振る舞い |
| --- | --- |
| Abort | 実行木を中止する。worktree のフォルダが無くても実行でき、中止した実行は再開できない |
| Resume | プロセスが居ない Session Node の会話を再開する。Node の状態は問わず、完了・中止した Node でも再開でき、Node の状態は変わらない。会話または作業場所が無ければ、実行中の実行木では新しい attempt で起動して最初の指示を送り、終わった実行木では失敗する |
| Retry | プロセスが居ない、未完了の Command Node を新しい attempt で起動する。実行 worktree のフォルダが必要 |
| Approve | 承認待ちの Node を完了させる |

既存の会話を Resume するとき、会話の続きを促す指示は自動送信しません。まだ親 Session に渡していない child の結果があれば、その Resume 時に渡します。Submit と provider Stop の片方だけ届いていることは Retry の理由になりません。

## worktree

Node は、起動時に選んだ worktree で動きます。Node ごとに、別の worktree で動かすこともできます。

| 指定 | 振る舞い |
| --- | --- |
| 共有（既定） | 親の Node と同じ worktree で動く |
| 隔離 | 実行のたびに、親の worktree の HEAD から新しい branch と worktree を作って動く |

- 隔離した Node の Artifact には、`worktree` として `branch` と `path` が入ります。後続の Node はこれを参照できます。
- Sequence / Fanout を隔離すると、子はすべてその一つの worktree で動きます。Fanout の子を隔離すると、子ごとに別の worktree になります。
- 隔離した worktree の成果を元の worktree へ統合する処理は、Releash は行いません。worktree と branch も削除しません。統合は人か後続の Node が git で行います。

## facet と指示

facet は、Session の agent に渡す指示を組み立てる Markdown の部品です。役割ごとに3種類あります。

| 種類 | 役割 | 一つの Session で参照できる数 |
| --- | --- | --- |
| policy | 振る舞いの方針 | 1つ |
| knowledge | 作業に必要な知識 | 複数 |
| instruction | この Node でする作業 | 1つ |

Session を起動すると、次の順に空行で区切って連結した一つの文章が、最初の指示として agent に渡ります。

1. policy
2. knowledge（参照した順）
3. instruction
4. input の値（パラメータごとの見出しと JSON）
5. delegate の案内（delegate を指定した場合）
6. 提出する成果の形の説明（Contract を宣言した場合）
7. 提出コマンドの使い方

- 4 以降は Releash が書き加えます。
- policy も、system prompt としてではなく、この文章の先頭として渡ります。
- facet の本文では、Session の input の値を `{{ パラメータ名 }}` で埋め込めます。埋め込まなくても、input の値は 4 として渡ります。

## 次に読むもの

- [YAML リファレンス](./yaml.md)
- [Lua リファレンス](./lua.md)
- [共通リファレンス](./common.md): 置き場所、名前の規則、Diagnostic のコード、Lua 補完ファイル
