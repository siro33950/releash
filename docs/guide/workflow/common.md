# 共通リファレンス

この文書は、YAML と Lua のどちらで書く場合にも共通する規則と一覧をまとめます。各要素の意味は [Workflow の概念](./concepts.md)、書き方は [YAML リファレンス](./yaml.md) と [Lua リファレンス](./lua.md) を参照してください。

## 置き場所

定義ファイルと facet は、次のディレクトリに置きます。

```text
~/Library/Application Support/releash/workflows/
├── <name>.yml
├── <name>.lua
├── <モジュール>/<name>.lua
├── policies/<key>.md
├── knowledge/<key>.md
└── instructions/<key>.md
```

- workflows ディレクトリ直下の `.yml` と `.lua` が、workflow 定義として読み込まれます。
- ファイル名（拡張子を除く）と定義の `name` を一致させてください。
  - workflow の一覧にはファイル名が表示され、起動するときは `name` で定義を探します。一致しない YAML は、一覧に出ても起動できません。
  - Lua で一致しない場合は `WFS006` です。
- 同じ `name` の定義が複数あると、起動が `WFS006` で拒否されます。`.yml` と `.lua` に同じ名前を置かないでください。
- Lua のモジュールはサブディレクトリに置きます。[Lua リファレンスの require](./lua.md#require) を参照してください。

### builtin と同じ名前

Releash には builtin の workflow と facet が組み込まれています。

- builtin は編集・削除できず、外部エディタでも開けません。
- Settings の「Automation」から、builtin と同じ名前の workflow や facet は保存できません。
- ディレクトリに builtin と同じ名前のファイルを置いた場合は、次のように扱われます。
  - workflow: 一覧と詳細には置いたファイルが表示され、builtin は表示されません。起動すると `WFS006`（`workflow name '<name>' is duplicated: <ファイル>, builtin:<name>`）で拒否されます。
  - facet: 置いたファイルの本文が builtin の代わりに使われます。

## 名前の規則

| 名前 | 規則 |
| --- | --- |
| workflow 名、Node 名、Contract 名、facet の key | 先頭が ASCII 英数字、2文字目以降が ASCII 英数字・`-`・`_` |
| 配線・`env`・`{{ }}`・`items` の field path の各段 | 同上 |
| 分岐の条件の field path の各段 | 文字種の制限なし。ただし `.` は段の区切りなので、`.` を含む field は参照できない |
| 環境変数名 | `[A-Za-z_][A-Za-z0-9_]*`。`RELEASH_` で始まる名前は使えない |

次の語は Node 名に使えません（`WFR004`）。

```text
command session fanout sequence input artifact completion env worktree
inputs rules items entry children
```

- `request` と `items` は input のパラメータ名に使えません（`WFR008`）。`request` は Contract 名にも使えません。
- `#` を含む名前は、名前を付けていない Node に Releash が付ける名前（`<親の名前>#<番号>`）です。配線や field path の中では使えません。

## 上限

| 対象 | 上限 |
| --- | --- |
| 一つの定義の Node の数 | 256 |
| 一つの Fanout の children | 64 |
| Command の `stdout` / `stderr` field | 100KB（超えた分は切り詰める） |

Lua の評価の上限は [Lua リファレンスの評価のしかた](./lua.md#評価のしかた) を参照してください。

## Contract の規則

| 型 | 規則 |
| --- | --- |
| object | `required` の field は `properties` に無ければならない |
| array | 要素の型を一つ持つ |
| string | 列挙値を持つ場合は、空にできない |
| boolean / integer / number | 追加の指定は無い |

- Session と Command の Artifact の Contract は object にします（`WFT004`）。
- 分岐に使う field は、直上の object の `required` に含めます。途中の object は `required` でなくてもかまいません。
- Artifact の Contract の直下に、次の field は宣言できません（`WFT005`）。

| field | 対象 |
| --- | --- |
| `worktree` | すべての Node |
| `ok` / `exit_code` / `stdout` / `stderr` / `duration` | Command |
| `child` | delegate を指定した Session |

## facet ファイル

- `policies/`、`knowledge/`、`instructions/` に `<key>.md` として置きます。
- key が名前の規則に違反すると `FAC001` です。存在しない facet を参照すると、YAML では `FAC002`、Lua では `WFR900` です。
- facet の description は、本文で最初の空でない行です。行頭が `# ` なら、それを除いた見出しを使います。description は facet の一覧と Lua の補完に表示されます。
- 本文の `{{ パラメータ名 }}` / `{{ パラメータ名.field }}` は、その facet を参照する Session の input の値に置き換わります。
  - `{{ }}` の中身が参照の書き方になっていないと `FAC003` です。
  - Session が宣言していないパラメータや、Contract から辿れない field を参照すると `WFR003` です。同じ facet を複数の Session が参照する場合は、Session ごとに検査されます。
- 指示になる順番は [Workflow の概念の facet と指示](./concepts.md#facet-と指示) を参照してください。

## Diagnostic

Diagnostic は定義の検査結果です。`error` の Diagnostic が一つでもある定義は起動できません。

### 見る場所

- Settings の「Automation」の一覧で、各行に error の件数（赤）と info の件数（青）が表示されます。詳細を開くと「Diagnostics」に1件ずつ並びます。
- `releash workflow diagnostics --dir <ディレクトリ>` で、置く前の定義を検査できます。引数と終了コードは [`releash workflow diagnostics`](../cli.md#releash-workflow-diagnostics) を参照してください。

例:

```text
error WFR003 31:5 [workflow=summarize-request, node=print_summary, field=env]: command node 'print_summary' env 'TEXT: result.body' is invalid: unknown field on the input parameter Contract

1 error, 0 info
```

各行は、severity、コード、位置、対象（workflow・Node・facet・field）、メッセージの順です。YAML は誤りをすべて報告し、Lua は最初の誤りだけを報告します。

### コードの分類

| 頭文字 | 分類（stage） | 検査する内容 |
| --- | --- | --- |
| `WFS` | `parse_shape` | 構文と形 |
| `WFR` / `FAC` | `resolve` | 名前と参照 |
| `WFT` | `typecheck` | 型 |
| `WFC` | `control_flow` | 進行・分岐・children の制約 |
| `WFI` | `resolve` | builtin であることの表示（info） |

### コード一覧

severity は `error` と `info` の2つです。

#### parse_shape

| コード | severity | 意味 |
| --- | --- | --- |
| `WFS001` | error | YAML の構文誤り、または定義ファイルを読み込めない |
| `WFS002` | error | 未知の field、値の形の誤り（mapping / list、`completion`、`permission`、`env`、条件、Contract の宣言）、Lua の builder の引数の誤り、対応しない拡張子 |
| `WFS003` | error | Node の種類を決める field がちょうど一つでない、一つの rule に `when` / `switch` / `loop_guard` を複数書いた |
| `WFS006` | error | 名前の誤り（文字種、重複、Lua の `name` とファイル名の不一致、`main` に付けた名前）、`nodes` が空、`command` が空、Node 数や Fanout の children 数の上限超過 |
| `WFS007` | error | Node の直下に `rules` / `inputs` を書いた |
| `WFS008` | error | children のエントリの形の誤り、`children` が空、Sequence に `artifact` を書いた、`inputs` の形の誤り |
| `WFS009` | error | Lua の構文誤り |
| `WFS010` | error | Lua の評価エラー（実行時エラー、上限超過、`r.workflow{}` を返さない、facet の一覧を読めない） |
| `WFS011` | error | Lua の `require` を解決できない |

#### resolve

| コード | severity | 意味 |
| --- | --- | --- |
| `WFR001` | error | 存在しない Node を参照した（children、遷移先、`entry`）。Lua では同じ Sequence の子でない遷移先も含む |
| `WFR002` | error | 存在しない、または不正な Contract を参照した |
| `WFR003` | error | 参照の誤り（`{{ }}`、`env`、`items`、辿れない field、Contract を持たない input の field） |
| `WFR004` | error | 予約語や予約された名前を使った（Node 名、`RELEASH_` で始まる環境変数名） |
| `WFR006` | error | `main` が無い |
| `WFR007` | error | 配線の誤り（供給元が無い・範囲外・Artifact を持たない、配線先が宣言された input でない） |
| `WFR008` | error | 配線の供給元が兄弟の Node と親のパラメータの両方に一致して曖昧、または `request` / `items` をパラメータ名にした |
| `WFR900` | error | Session に facet の参照が無い。Lua では存在しない facet の参照も含む |
| `FAC000` | info | builtin の facet である |
| `FAC001` | error | facet の key が名前の規則に違反している |
| `FAC002` | error | 存在しない facet を参照した |
| `FAC003` | error | facet 本文の `{{ }}` が参照の書き方になっていない |
| `WFI000` | info | builtin の workflow である |

#### typecheck

| コード | severity | 意味 |
| --- | --- | --- |
| `WFT001` | error | 真偽の条件、delegate の完了条件が、`required` の boolean field を参照していない |
| `WFT002` | error | 列挙の条件が `required` の string の列挙 field を参照していない、または `cases` の値が列挙値に無い |
| `WFT003` | error | Fanout で展開する要素の型が、子の input の Contract と一致しない |
| `WFT004` | error | `artifact` の Contract が object でない |
| `WFT005` | error | Artifact の Contract に予約された field を宣言した |
| `WFT006` | error | Artifact を持たない Node の field で分岐した、delegate を指定した Session に `artifact` が無い、delegate の child が Artifact を持たない |

#### control_flow

| コード | severity | 意味 |
| --- | --- | --- |
| `WFC001` | error | `main` から到達できない Node がある |
| `WFC002` | error | 一つの `rules` に `when` / `switch` や `loop_guard` を複数書いた、`when` / `switch` と単独の `next` を併記した |
| `WFC003` | error | `next` の誤り（すべての値を書いた `switch` に `next` がある、一部だけの `switch` に `next` が無い、catch-all の `next` が複数ある） |
| `WFC004` | error | `switch` に `cases` が無い |
| `WFC005` | error | ループに `loop_guard` が無い、`max_iterations` が1未満 |
| `WFC006` | error | 同じ Node を複数の Sequence / Fanout や delegate の子にした、`main` を子にした、別の Sequence の子へ外から遷移した |
| `WFC007` | error | 同じ Sequence / Fanout に同じ Node を複数回置いた、Fanout のエントリに `rules` を書いた、Lua で同じ Node の値を複数の `r.child{}` に置いた |
| `WFC008` | error | Sequence / Fanout が children を通じて自分自身を含む |
| `WFC011` | error | Session 以外の Node に delegate を指定した |

## Lua 補完ファイル

Lua の定義を LuaLS で補完するためのファイルを、Releash が workflows ディレクトリに作ります。

| ファイル | 内容 |
| --- | --- |
| `.releash/releash.lua` | `require("releash")` の builder の型 |
| `.releash/facets.lua` | `require("facets")` の facet の key と description |
| `.releash/facets/<種類>/<key>.md` | builtin facet の本文の写し（補完から本文を開くため） |
| `.luarc.json` | LuaLS の設定。無い場合だけ作る |

- 作るのは、アプリの起動時と、Settings の「Automation」で facet を保存・削除したときです。
- ファイルを直接置いた facet は、次に作り直すまで `.releash/facets.lua` に反映されません。
- これらは補完のためのファイルです。定義の読み込みと実行には使われません。
