# Design

## The actual design

### Architecture

#### Command の環境変数宣言と検証を workflow domain が所有する

`src-tauri/src/domain/workflow/value_objects/definition.rs` の `CommandSpec` に、環境変数名を identity として input パラメータ参照へ対応づける `env` map を追加する。環境変数名と `<parameter>` / `<parameter>.<field>` は workflow domain の値オブジェクトとして表現し、名前の形式、`RELEASH_` 予約、1段までの参照形式を domain の構築・検証規則にする。`env` の参照は children の供給元を表す既存 `InputSourceRef` とは別の概念であり、Command 自身に束縛済みの input を参照する。

`src-tauri/src/domain/workflow/services/reference.rs` と `validation.rs` は、`env` の各参照を対象 Node の `input` と Contract に照合する。`{{ }}` と `env` で参照構文の判定を二重化せず、既存の input パラメータ参照 parser と Contract field 解決を共用する。一方、`env` の値は template renderer へ渡さず、template 展開と shell command 文字列への連結を構造的に経由しない。

YAML loader は `env` を Node body の field として読み、Command の `CommandSpec` へ正規化する。Session / Fanout / Sequence での宣言は Command へ移し替えず shape error にする。`src-tauri/src/adaptor/gateway/workflow/lua/mod.rs` は `r.command{ env = { DOC = doc, SPEC_DIR = context.spec_dir } }` の string-keyed table を同じ `CommandSpec` へ変換し、値にはその Command の `input` に含まれる `ReleashInput` またはその1段 field handle だけを受理する。`src-tauri/src/adaptor/gateway/workflow/lua/stubs.rs` も同じ interface を補完対象として公開する。

`src-tauri/src/adaptor/gateway/workflow/diagnostics.rs` は YAML の span と Lua の source location の違いだけを adaptor に閉じ、同じ domain error を次の Diagnostic に写像する。

| 誤り | Diagnostic code / stage |
| --- | --- |
| `env` の map shape、値型、Command 以外での宣言 | `WFS002` / `parse_shape` |
| `[A-Za-z_][A-Za-z0-9_]*` に一致しない環境変数名 | `WFS006` / `parse_shape` |
| `RELEASH_` 予約名、Node 名 `env` | `WFR004` / `resolve` |
| 未宣言パラメータ、未知 Contract field、2段以上の参照 | `WFR003` / `resolve` |

これにより YAML と Lua は入力形式固有の parser を通っても、同じ誤りを同じ code で拒否し、Error Diagnostic がある定義を既存どおり実行対象にしない。

#### NodeExecution の binding から起動時だけ環境変数を実体化する

`src-tauri/src/adaptor/gateway/workflow/workflow_host.rs` は、Command leaf の開始時に既存の `LeafStart.bindings` と `CommandSpec.env` を domain の解決処理へ渡す。解決結果は、参照先の値が string なら内容そのもの、それ以外なら compact JSON テキストである。参照先の値が実行時に存在しない参照が一つでもあれば解決は失敗し、環境を部分的に組み立てて起動することはしない。各値を NodeExecution ごとの `CommandExecutionInput` に一時保持し、既存の `command_env` が生成する engine 環境変数と合わせて `src-tauri/src/infrastructure/process/command_runner.rs` の既存 `spawn_shell_command` へ渡す。

`spawn_shell_command` は既に `/bin/sh -c <command>` と `Command::envs` を別の引数として process 起動境界へ渡しているため、command runner の公開境界や shell 起動方式は変更しない。`raw_command` の既存 `{{ }}` 展開もそのまま維持し、`env` 由来の値だけを command 文字列へ入れない。起動後は親側の実体化済み環境変数を直ちに破棄し、durable fact、runtime projection、`display_command`、log に値や materialized command を追加しない。

`env` の参照が解決できない場合は、engine 環境変数だけを持つ子プロセスを起動する代わりに、既存の Command 起動不能と同じ Node failure として扱う。環境全体が platform 上限を超える場合、または値の NUL により OS process API が受理しない場合も、事前検査や独自上限を追加せず、`CommandRunnerError::Spawn` から同じ経路へ合流させる。process 起動に成功する前には `CommandSpawned` fact を記録しない既存の commit 境界を維持する。

#### Lua の env 値参照は Contract の有無に依らず field 参照を作る

Lua の `input` handle の1段 indexing は、現在 Contract を宣言しない input に対して load 時に `WFR003` を返す。これは children の配線が Contract 済みの source を要求するための制約であり、`env` の参照可能範囲を決めるものではない。`env` の参照可能範囲は YAML と同じく domain の `env` 参照検証が所有し、型なしパラメータの1段 field は既存の `{{ }}` 参照検証と同様に load 時に検証しない。

そのため Lua の `input` indexing は Contract の有無に依らず `<parameter>.<field>` の参照を作り、Contract を必須とする検証は、その参照が children の配線として使われた地点へ移す。参照は指す input、field、および indexing の source location を保持し、配線として使われた場合は現在と同じ `WFR003` を同じ location へ出す。これにより型なしパラメータの1段 field を値とする `env` は YAML と Lua の双方で受理され、`env` 以外の既存の Lua 受理範囲と Diagnostic を変えずに R-008 の同値性を保つ。

#### 再試行・再開と定義 snapshot

`env` の宣言は既存の `WorkflowRootFact.definition` に含まれる `WorkflowDefinition` snapshot の一部として保存する。再試行・再開では保存済み宣言と aggregate が再構築した NodeExecution binding から値を再度解決し、実体化済み環境変数は保存しない。これにより新しい recovery state や環境変数値の別 source of truth を作らない。

主要な変更対象は次のとおり。

| Path | 変更の要旨 |
| --- | --- |
| `src-tauri/src/domain/workflow/value_objects/definition.rs` | `CommandSpec.env`、環境変数名、input パラメータ参照の domain 表現と YAML 正規化 |
| `src-tauri/src/domain/workflow/services/reference.rs` / `src-tauri/src/domain/workflow/services/validation.rs` | `env` 参照の解決、Contract 照合、名前・予約規則の検証 |
| `src-tauri/src/adaptor/gateway/workflow/diagnostics.rs` | YAML の shape 検査、domain error の Diagnostic code・span への変換 |
| `src-tauri/src/adaptor/gateway/workflow/lua/mod.rs` / `src-tauri/src/adaptor/gateway/workflow/lua/stubs.rs` | Lua `env` table の受理、domain 定義への変換、補完 interface |
| `src-tauri/src/adaptor/gateway/workflow/mapper.rs` | `env` を含む `CommandSpec` の既存 schema/domain 変換の追随 |
| `src-tauri/src/adaptor/gateway/workflow/workflow_host.rs` / `src-tauri/src/adaptor/gateway/workflow/workflow_host/command_preparation.rs` | binding からの値実体化、engine 環境変数との合成、process 起動までの一時保持 |
| `docs/glossary/WORKFLOW.md` | Command の `env` 構文・変換・検証・予約名と、安全な値供給例の正本化、値を shell 構文へ連結する既存例の是正 |

#### 検証境界

domain test は環境変数名、予約 prefix、input / Contract field 参照、string と非 string の変換を検証する。workflow gateway test は YAML と Lua の同値な定義が同じ `WorkflowDefinition` になること、上表の各誤りが両形式で同じ code になること、`env` がない定義と `{{ }}` の load 結果が変わらないことを検証する。

B-017 と B-018 は文字列変換の単体テストだけでは shell の非再解釈を確認できないため、実際の `/bin/sh` process 境界で引用符、バッククォート、改行、`$`、`;` を含む値を quoted / unquoted の両方で渡し、副作用となる command が実行されないことを検証する。B-019 は OS process 起動境界で NUL と platform 環境上限による失敗が既存の spawn failure 分類へ入ることを、engine 固有の固定サイズを前提にせず検証する。

### Interface

公開する workflow 定義 interface は、YAML Command node の `env: <環境変数名>: <parameter | parameter.field>` map と、Lua `r.command` の `env = { <環境変数名> = <ReleashInput | ReleashInput.field> }` table である。環境変数名は `[A-Za-z_][A-Za-z0-9_]*` に一致し、`RELEASH_` で始まらない名前だけを受理する。値参照は同じ Command の宣言済み `input` とその1段 field に限定する。

子 process への値は、string を無変換で、string 以外を JSON テキストで渡す。shell からは通常の `$VAR` / `"$VAR"` として参照するが、その展開結果を shell source として再評価しない。既存の `{{ parameter }}` / `{{ parameter.field }}` interface と quoting 特性は変更しない。

Tauri command、local API、CLI、Command の結果型、durable fact の種別は変更しない。`env` を省略した `CommandSpec` は空 map を既定値とし、保存時も空 map を省略するため、既存 workflow 定義と既存 `WorkflowRootFact.definition` を同じ意味で読み出せる。schema version や移行 command は追加しない。

### Data Model

workflow domain が所有する `CommandSpec.env` は、環境変数名を identity とする map と、対応する1段までの input パラメータ参照を保持する。値そのもの、shell 展開結果、process の全環境 snapshot は保持しない。

永続化するのは `WorkflowDefinition` 内の宣言だけである。NodeExecution ごとの実体化済み `(name, value)` は process 起動用の一時データであり、起動後に破棄する。新しい identity、runtime projection、versioning は追加しない。

### Database

該当なし。SQLite schema と access path は変更せず、既存 workflow root fact の定義 snapshot に additive な省略可能 field として保存する。

### UI/UX

該当なし。

### Algorithm

該当なし。参照解決、string / JSON 変換、process 環境への供給方法は Requirements と既存 binding / command runner 境界から一意に決まる。

### Infra

該当なし。

## Alternatives Considered

### shell quoting を生成して command 文字列へ埋め込む

採用しない。値を shell source と同じ in-band 文字列へ戻すため、囲いを閉じる内容に対して安全性を保証できず、R-003 と R-010 を満たさない。

### 実体化済み環境変数を durable fact に保存する

採用しない。WorkflowDefinition の宣言と NodeExecution binding から再構築できる値を別の source of truth として保持し、文書本文や Artifact 値を実行履歴へ複製するためである。再試行・再開に必要なのは参照宣言であり、値 snapshot ではない。

## Cross-cutting concerns

### セキュリティと保持範囲

`env` の値は `Command::envs` だけへ渡し、command source、Diagnostic、fact、read model、log へ複製しない。`RELEASH_` prefix を load 時に予約することで、定義側の値が engine-owned identity を上書きできる状態を作らない。値の大きさには engine 独自の拒否境界を設けず、同時実行中も各 Command の起動準備に必要な値だけを一時保持する。

## Risks

該当なし。
