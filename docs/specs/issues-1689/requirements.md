# Context

- 入力文書
  - Issue #1689「[workflow engine] Command node に env での param 供給を追加する（shell 構文を経由しない値の受け渡し）」 https://github.com/siro33950/releash/issues/1689
  - `docs/glossary/WORKFLOW.md`（workflow 定義構文の正本。Command の節は 185-195 行、`{{ }}` の quoting 注意書きは 195 行、`echo '{{ reviews }}' | jq` の例は 73 行）
  - `AGENTS.md`（143 行「command テンプレートの `{{ }}` は shell quoting を行わない。信頼できない値を shell syntax へ直接連結しない」）

- 確定済みの背景と制約
  - Command node は worktree を cwd として `/bin/sh -c <command>` を一度実行する。子プロセスは Releash プロセスの環境を継承し、engine が指定した環境変数をその上に重ねる。
  - engine が Command の子プロセスへ注入する環境変数は `RELEASH_WORKFLOW_EXECUTION_ID` / `RELEASH_NODE_EXECUTION_ID` / `RELEASH_WORKTREE_PATH` / `RELEASH_SESSION_ID` である。`RELEASH_BASE_BRANCH` は Session の provider CLI 起動時にだけ注入され、Command の子プロセスへは注入されない。
  - Issue が「engine が注入する既存変数（`RELEASH_BASE_BRANCH` 等）との衝突は load 時に拒否する」と書く予約名の範囲は、本 Spec 作成時に人間が **`RELEASH_` で始まる名前すべてを予約する** と確定した。Command へ実際に注入される 4 変数だけに限定しない。
  - `{{ }}` テンプレートは維持する。Issue は `{{ }}` の置き換えではなく、shell 構文を経由しない別経路の追加を求めている。
  - 採用する方式は env による out-of-band 供給である。engine が宣言 path へ内容を配置する Argo 型の input artifact は Issue で検討のうえ不採用と結論している。
  - YAML と Lua は load 後に同じ `WorkflowDefinition` になる。定義形式の違いは実行・事実ログ・read model・resume に残らない。
  - 定義は Diagnostic に Error が一つでもあれば実行できない。未知 field は受理しない。

# Outcome

workflow 定義を書く開発者が対象である。

現在、Session が生成した任意の値（文書本文、Artifact の JSON など）を Command へ渡す経路は `{{ }}` テンプレートによる shell command 文字列への連結しかない。`{{ }}` は shell quoting を行わないため、この経路には次の問題がある。

- 正当な入力で日常的に壊れる。アポストロフィ・引用符・バッククォートを含む markdown は、コードフェンスがある限り通常ケースである。
- 汚染された入力では command injection になる。
- single quote・heredoc・jq 経由といった囲い方の工夫では防げない。値が shell 構文の中（in-band）を通る限り、値自身が囲いを閉じる文字列を含み得る。
- 正本自身が「信頼できない値を shell syntax へ直接連結しない」と禁じているため、文書本文のファイル化などは禁止に違反する形でしか書けない。

変更後は、Command node が `env` を宣言することで、値を shell 構文へ通さず子プロセスの環境変数として受け取れる。定義の書き手は `printf '%s' "$DOC" > "$SPEC_DIR/requirements.md"` のように shell ネイティブの変数参照で値を扱え、値の内容が command 実行へ昇格しない。

# Current Behavior

## Command node が `env` を受理しない

Command node の body が受理する field は `command` / `session` / `fanout` / `sequence` / `artifact` / `input` / `completion` / `worktree`（および移設案内のための `inputs` / `rules`）だけである。

再現手順: 次の定義を load する。

```yaml
materialize_requirements:
  command: "printf '%s' \"$DOC\" > \"$SPEC_DIR/requirements.md\""
  env:
    DOC: doc
  input:
  - doc
```

実際の出力: parse / shape 段の Error Diagnostic `WFS002` `unknown workflow field 'env' is not allowed here` が出る。Error があるため定義は実行できない。

Lua の `r.command{ ... }` も受理キーが `name` / `command` / `artifact` / `input` / `completion` に固定されており、`env = { ... }` を渡すと `WFS002` `unknown field 'env'` になる。

## `{{ }}` 経由の値が shell 構文として再解釈される

`{{ parameter }}` / `{{ parameter.field }}` は load 時に宣言済み input パラメータと Contract field へ照合されるが、実行時の展開では string 値をそのまま、非 string 値を JSON テキストとして command 文字列へ差し込むだけで、shell quoting は行わない。

再現手順: `command: "echo '{{ doc }}'"` の Command node に、`doc` として `it's fine` が束縛された状態で実行する。展開後の command 文字列は `echo 'it's fine'` になる。これは `/bin/sh -c` へそのまま渡る。

実際の出力:

```
$ /bin/sh -c "echo 'it's fine'"
/bin/sh: -c: line 0: unexpected EOF while looking for matching `''
/bin/sh: -c: line 1: syntax error: unexpected end of file
exit=2
```

非ゼロ exit code なので、Command node はこれを `ok: false` の確定結果として扱う。値の内容によっては同じ経路で任意の command が実行され得る。

## 正本の記述と例が噛み合っていない

`docs/glossary/WORKFLOW.md` は 195 行で「shell quoting は自動で行われないため、信頼できない値を shell syntax へ直接連結しない」と禁じている。一方で 73 行の例は `command: "echo '{{ reviews }}' | jq '{all_lgtm: all(.[].lgtm)}'"` であり、値を single quote で囲って連結している。正本には、禁止に違反しない値の渡し方が示されていない。

## 現行 builtin workflow の回避策

`workflows/` 配下の builtin 定義は、command 内の `{{ }}` 参照を避け、`releash workflow output get "$RELEASH_WORKFLOW_EXECUTION_ID" --node <node> --json | jq ...` のように engine 注入の環境変数と CLI 経由で値を取り直している。`workflows/examples/full-cycle-development.yml` にだけ `rm -f -- '{{ spec }}/behavior.md'` の形が残る。

# Scope / Non-goals

## 変更するもの

- YAML の Command node への `env` field の追加。
- Lua の `r.command{ ... }` への `env` 引数の追加。
- `env` の load 時検証（参照先の解決、環境変数名の形式、予約名、宣言できる Node 種別）。
- `env` で宣言された値の、実行時の子プロセス環境への供給。
- `docs/glossary/WORKFLOW.md` への `env` の明文化。`{{ }}` の quoting 注意書きと対にし、「信頼できない値は env で渡す」を正の作法として記す。

## 変更しないもの

- `{{ }}` テンプレートの受理形と展開。廃止も、自動 shell quoting の追加も行わない。
- 既存 builtin workflow および `workflows/examples/` の定義。`{{ }}` や `releash workflow output get` を使う既存箇所を `env` へ置き換えない。
- Session / Fanout / Sequence node への値供給経路。
- engine が注入する既存の環境変数の名前・値・注入対象。
- Argo 型（engine が宣言 path へ内容を直接配置する input artifact）の導入。
- `env` を前提とする dev-cycle workflow の Spec 作成段の実装。

# Requirements

- R-001: Command node は `env` を宣言できる。`env` は `<環境変数名>: <input パラメータ名>` または `<環境変数名>: <input パラメータ名>.<field>` の map であり、宣言した各エントリは、その Command が実行する子プロセスの環境変数として届く。
- R-002: `env` の値参照は、その Node が `input` で宣言したパラメータに限る。未宣言パラメータの参照、型あり（Contract 付き）パラメータに存在しない field の参照、2 段以上の field path は load 時に Error Diagnostic として拒否され、その定義は実行できない。
- R-003: 参照した値が string の場合はその文字列を、string 以外の場合は JSON テキストを環境変数の値にする。値に対してテンプレート展開も shell 解釈も行わない。
- R-004: `env` の環境変数名は `[A-Za-z_][A-Za-z0-9_]*` に一致しなければならない。一致しない名前は load 時に Error Diagnostic として拒否される。
- R-005: `RELEASH_` で始まる環境変数名は engine の予約であり、`env` で宣言すると load 時に Error Diagnostic として拒否される。これにより、engine が注入する環境変数を定義側から上書きできない。
- R-006: `env` は Command node だけが宣言できる。Session / Fanout / Sequence node が `env` を宣言すると load 時に Error Diagnostic として拒否される。
- R-007: `env` は Node body の field 名であるため、`env` は Node 名として使えない予約語になる。`env` という名前の Node を宣言すると load 時に Error Diagnostic として拒否される。
- R-008: Lua は `r.command{ command = ..., env = { DOC = doc }, input = { doc } }` の形で同じ `env` を表現でき、YAML と Lua の同じ内容の定義は load 後に同じ定義になる。同じ誤りには同じ Diagnostic code が使われる。
- R-009: 互換性要件。`env` を宣言しない既存の定義は、load 結果（Diagnostic の有無と内容）も実行結果も変わらない。`{{ }}` を含む定義も同じ結果になる。
- R-010: 安全性要件。`env` で渡した値は shell 構文として再解釈されない。値が引用符・バッククォート・改行・`$`・`;` などを含んでも、command 実行へ昇格しない。引用符を欠いた `$VAR` 参照でも、影響は word splitting による値の破損までにとどまる。
- R-011: `env` の値の大きさは platform の環境サイズ上限に従い、engine 独自の上限や load 時の値検査を設けない。上限超過、または値に NUL が含まれることで子プロセスを起動できない場合は、既存の command 起動不能と同じ扱いになる。
- R-012: `docs/glossary/WORKFLOW.md` が `env` の受理形、値の変換規則、load 時検証、予約名を記述し、`{{ }}` の shell quoting 注意書きと対にして「信頼できない値は env で渡す」を正の作法として示す。同文書に残る、値を shell 構文へ直接連結する既存の例は、この禁止に違反しない形へ改める。
- R-013: `env` が参照する値が実行時に解決できない場合、その Command は子プロセスを起動せず Node failure になる。解決できない場合とは、宣言済みの input パラメータが実行時に束縛されない場合、および型なしパラメータの参照 field が実行時の値に存在しない場合である。

# Assumptions / Open Questions

Assumption なし。Open Question なし。
