## B-001: inputパラメータを環境変数として受け取る

GIVEN Command nodeが`input`で宣言したパラメータを`env`の環境変数へ対応づけたworkflow定義がloadされている
WHEN そのパラメータに値が束縛されたCommandを実行する
THEN Commandの子プロセスは、宣言した名前の環境変数から対応する値を受け取れる

## B-002: Contract fieldを環境変数として受け取る

GIVEN Command nodeが`input`で宣言した型ありパラメータの1段のfieldを`env`の環境変数へ対応づけたworkflow定義がloadされている
WHEN そのパラメータに値が束縛されたCommandを実行する
THEN Commandの子プロセスは、宣言した名前の環境変数から対応するfieldの値を受け取れる

## B-003: 未宣言inputパラメータ参照の拒否

GIVEN Command nodeの`env`が、そのNodeの`input`で宣言されていないパラメータを参照するworkflow定義がある
WHEN そのworkflow定義をloadする
THEN 未宣言の参照であることを示すError Diagnosticが返る
AND そのworkflow定義は実行できない

## B-004: 存在しないContract field参照の拒否

GIVEN Command nodeの`env`が、型ありinputパラメータのContractに存在しないfieldを参照するworkflow定義がある
WHEN そのworkflow定義をloadする
THEN 存在しないfieldの参照であることを示すError Diagnosticが返る
AND そのworkflow定義は実行できない

## B-005: 2段以上のfield pathの拒否

GIVEN Command nodeの`env`が、inputパラメータから2段以上のfield pathを参照するworkflow定義がある
WHEN そのworkflow定義をloadする
THEN 受理されない参照形式であることを示すError Diagnosticが返る
AND そのworkflow定義は実行できない

## B-006: string値の無変換での供給

GIVEN Command nodeの`env`が参照するinputの値がstringである
WHEN そのCommandを実行する
THEN 子プロセスの環境変数の値は元のstringと一致する
AND 値に含まれるテンプレート表記は展開されない
AND 値に含まれるshell構文は解釈されない

## B-007: string以外の値のJSON供給

GIVEN Command nodeの`env`が参照するinputまたはfieldの値がstring以外である
WHEN そのCommandを実行する
THEN 子プロセスの環境変数の値は、参照した値を表すJSONテキストと一致する

## B-008: 環境変数名の受理

GIVEN `env`以外が妥当なCommand node定義がある
WHEN `[A-Za-z_][A-Za-z0-9_]*`に一致し、`RELEASH_`で始まらない名前を`env`の環境変数名としてworkflow定義をloadする
THEN その環境変数名を理由とするError Diagnosticは発生しない

## B-009: 不正な環境変数名の拒否

GIVEN Command nodeの`env`に`[A-Za-z_][A-Za-z0-9_]*`と一致しない環境変数名がある
WHEN そのworkflow定義をloadする
THEN 環境変数名が不正であることを示すError Diagnosticが返る
AND そのworkflow定義は実行できない

## B-010: engine予約環境変数名の拒否

GIVEN Command nodeの`env`に`RELEASH_`で始まる環境変数名がある
WHEN そのworkflow定義をloadする
THEN engineの予約名であることを示すError Diagnosticが返る
AND そのworkflow定義は実行できない
AND engineが注入する環境変数は定義側の値で上書きされない

## B-011: Command以外のNodeでのenv拒否

GIVEN Session、Fanout、またはSequence nodeが`env`を宣言するworkflow定義がある
WHEN そのworkflow定義をloadする
THEN そのNode種別では`env`を宣言できないことを示すError Diagnosticが返る
AND そのworkflow定義は実行できない

## B-012: envというNode名の拒否

GIVEN `env`という名前のNodeを宣言するworkflow定義がある
WHEN そのworkflow定義をloadする
THEN Node名が予約されていることを示すError Diagnosticが返る
AND そのworkflow定義は実行できない

## B-013: YAMLとLuaのenv定義の同値性

GIVEN 同じCommand、input、およびenv対応を持つworkflow定義がYAMLとLuaで記述されている
WHEN 両方のworkflow定義をloadする
THEN 両方のload後の定義は同じ内容になる
AND 両方のCommandの子プロセスは同じ名前と値の環境変数を受け取る

## B-014: YAMLとLuaのenv Diagnosticの同値性

GIVEN Requirementsで拒否対象とされた同じ`env`の誤りを持つworkflow定義がYAMLとLuaで記述されている
WHEN 両方のworkflow定義をloadする
THEN 両方に同じDiagnostic codeのError Diagnosticが返る
AND 両方のworkflow定義は実行できない

## B-015: envを宣言しない既存定義の互換性

GIVEN `env`を宣言しない既存のworkflow定義がある
WHEN `env`対応の変更前後でその定義をloadして実行する
THEN load時のDiagnosticの有無と内容は変わらない
AND Commandの実行結果は変わらない

## B-016: 既存テンプレートの互換性

GIVEN `{{ parameter }}`または`{{ parameter.field }}`をCommandに含む既存のworkflow定義がある
WHEN `env`対応の変更後にその定義をloadして実行する
THEN テンプレートの受理形と展開結果は変更前と同じである
AND shell quotingは自動で追加されない

## B-017: env値のshell commandへの非昇格

GIVEN Command nodeが引用符、バッククォート、改行、`$`、または`;`を含む値を`env`で受け取る
WHEN Commandがshellネイティブの引用付き環境変数参照でその値を使用する
THEN 値は元の内容を保ったデータとしてCommandへ渡る
AND 値の内容はshell commandとして実行されない

## B-018: 引用されていない環境変数参照の安全性

GIVEN Command nodeがshell構文を含む値を`env`で受け取る
WHEN Commandが引用符を付けずにその環境変数を参照する
THEN 値の内容はshell commandとして実行されない
AND shellのword splittingによる値の破損を超えてcommand実行へ昇格しない

## B-019: platform環境制約による起動不能

GIVEN Command nodeの`env`に、platformの環境サイズ上限を超える値またはNULを含む値が束縛されている
WHEN そのCommandの子プロセスを起動する
THEN engine独自の上限またはload時の値検査による拒否は行われない
AND 子プロセスを起動できない結果は既存のCommand起動不能と同じ扱いになる

## B-020: workflow定義構文でのenvの明文化

GIVEN workflow定義の書き手が`docs/glossary/WORKFLOW.md`のCommand nodeの説明を参照する
WHEN `env`による値供給の仕様と安全な値の渡し方を確認する
THEN `env`の受理形、stringとstring以外の値の変換規則、load時検証、および`RELEASH_`予約名を確認できる
AND `{{ }}`ではshell quotingされないことと、信頼できない値は`env`で渡すことを対にして確認できる
AND 同文書には値をshell構文へ直接連結する例が残っていない

## B-021: 解決できないenv参照によるNode failure

GIVEN Command nodeの`env`が参照する値が実行時に解決できない
WHEN そのCommandを実行する
THEN Commandの子プロセスは起動されない
AND そのCommandはNode failureになる

## 要件IDとBehavior IDの対応表

| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001, B-002 |
| R-002 | B-003, B-004, B-005 |
| R-003 | B-006, B-007 |
| R-004 | B-008, B-009 |
| R-005 | B-008, B-010 |
| R-006 | B-011 |
| R-007 | B-012 |
| R-008 | B-013, B-014 |
| R-009 | B-015, B-016 |
| R-010 | B-006, B-017, B-018 |
| R-011 | B-019 |
| R-012 | B-020 |
| R-013 | B-021 |
