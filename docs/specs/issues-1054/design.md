# Design

## Behavior Coverage

| Rule | 設計方針 |
| --- | --- |
| 起動環境に応じた CLI alias の解決 | 起動環境（dev / 本番）から一意に決まる `PathAliases` を構築し、facet 展開・子プロセス起動の双方に同じ alias を供給する。 |
| dev 起動による本番 CLI の不変性 | CLI install は本番 binary のときのみ `/usr/local/bin/releash` を更新する。dev binary は本番 CLI 名を所有せず、別 alias の install 経路に閉じる。 |
| CLI alias と実行対象の一意な対応 | `PathAliases` は alias 名・実行 binary・データディレクトリの三者を組として保持し、alias を解決した時点で実行対象とデータ領域が確定する。 |
| agent 子プロセスへの実行環境の伝搬 | PTY / oneshot / agent bridge のいずれも、共通の子プロセス環境ビルダーを通して alias 解決可能な `PATH` と環境別 `RELEASH_DATA_DIR` を載せる。 |
| facet テンプレートにおける CLI alias の展開 | facet 展開エンジンに `path_alias` namespace を追加し、`{{path_alias.releash}}` を起動環境の CLI alias に展開する。 |
| workflow 定義変数の facet 展開 | workflow 定義に変数宣言領域を追加し、facet 展開エンジンが `{{vars.<name>}}` を namespace 経由で解決する。 |
| 未定義 workflow 変数の拒否 | workflow 読み込み時を一次境界として未定義 `vars.*` を明示的なエラーにする。保存時・展開時での重複検出は許容する。 |
| 既存プレースホルダの互換性 | 既存プレースホルダの展開は変更前と同一の名前空間・同一の解決経路に残し、新規 namespace と分離する。 |

## Key Decisions

- **CLI alias を「名前空間」として扱う**: 単なる文字列定数ではなく、`PathAlias` という解決単位（alias 名 + 実行 binary + 内包データ領域）として定義する。これによりテンプレート展開と子プロセス起動が同じソースから値を引ける。
- **dev alias は wrapper として扱う**: dev binary を本番 CLI 名にぶら下げる方式（symlink 上書き）ではなく、dev alias 自身が dev データ領域を内包する独立した実行対象として扱う。本番 CLI install 経路と dev alias install 経路を完全に分離する。
  - 代替案: dev 起動時に `/usr/local/bin/releash` を dev binary に張り替え、`RELEASH_DATA_DIR` だけで切り分ける。→ 本番 CLI 破壊リスクと利用者の明示指定との競合があるため不採用。
- **テンプレート展開は namespace 制**: `{{path_alias.<name>}}` と `{{vars.<name>}}` を namespace 付きで導入する。既存の `{{project_name}}` / `{{task}}` はトップレベル namespace として残し、衝突しない。
  - 代替案: workflow 変数を既存のフラット namespace に同居させる。→ システム定義名との衝突や上書き事故を防げないため不採用。
- **未定義変数はエラー**: 黙って空文字に展開せず、検出した境界でエラーとする。`{{path_alias.<name>}}` は起動環境が確定すれば必ず解決可能なので、検出対象は主に `{{vars.<name>}}` および typo した namespace。
- **`RELEASH_DATA_DIR` 解決順序**: 明示指定 > alias 内包値 > プロセス既定。利用者の明示指定を奪わない。
- **path_alias は当面 `releash` キーに限定**: 公開する alias は `releash` のみとし、将来拡張は名前空間構造で吸収できる形にとどめる（一般化は今回スコープ外）。

## Responsibility Boundaries

- **PathAliases（バックエンド）**
  - 起動環境から CLI alias 名・実行 binary・データディレクトリを決定する単一のソース。
  - facet 展開・子プロセス環境ビルダーに同一の値を提供する。
- **CLI install 経路（バックエンド）**
  - 本番 binary 起動時のみ本番 alias を所有する。dev binary は本番 alias install 経路に関与しない。
- **子プロセス環境ビルダー（バックエンド）**
  - PTY / oneshot / agent bridge いずれの起動経路にも、PathAliases から導いた `PATH` と `RELEASH_DATA_DIR` を載せる責務。
- **Workflow 定義（schema / storage）**
  - 変数宣言を保持・配布する。値の意味は workflow 作成者に委ねる。
- **Facet 展開エンジン**
  - namespace ごとの解決元（path_alias / vars / 既存プレースホルダ）を統合して展開する。
  - 未定義参照を検出する。
- **Agent / フロントエンド**
  - alias 名や `RELEASH_DATA_DIR` を自力で組み立てない。展開済み facet と環境変数の入った子プロセスを受け取るだけ。

## Contracts

- **`{{path_alias.releash}}`**
  - 本番起動時は literal `releash` に展開される。
  - dev 起動時は literal `releash-dev` に展開される。
  - facet 本文・built-in prompt の双方で同じ意味を持つ。
- **`{{vars.<name>}}`**
  - workflow 定義側で宣言された名前→値の組から解決される。
  - 値は静的文字列のみとし、動的解決値を含まない。
  - 未定義の `<name>` を参照する facet は workflow 読み込み時に一次検出され、明示的なエラーとなる。保存時・展開時での重複検出は許容する。
- **既存プレースホルダ（`{{project_name}}` / `{{task}}` 等）**
  - 展開結果は本変更前と等価。
- **workflow 定義における変数宣言**
  - workflow 定義ファイルに facet 展開用の変数群（名前→静的文字列値）を宣言できる。
- **CLI alias と実行対象の対応**
  - 本番 alias の CLI 呼び出しは本番データ領域を参照する。
  - dev alias の CLI 呼び出しは、呼び出し側が `RELEASH_DATA_DIR` を明示しない限り dev データ領域を参照する。
- **子プロセスの実行環境**
  - 起動環境に対応する CLI alias が `PATH` 経由で解決可能。
  - 起動環境に対応する `RELEASH_DATA_DIR` が設定される（明示指定がある場合はそちらが優先）。
- **本番 CLI install**
  - 本番 binary 起動時のみ `/usr/local/bin/releash` を更新する。dev 起動はこのパスへ書き込まない。

## Data / Communication Flow

- **起動 → PathAliases 確定**
  - アプリ起動時に、ビルド種別から `PathAliases`（本番 alias セット or dev alias セット）が一意に確定する。
- **PathAliases → 子プロセス起動経路**
  - PTY / oneshot / agent bridge の起動コードは、PathAliases を参照して `PATH` と `RELEASH_DATA_DIR` を子プロセス env に積む。
- **PathAliases + workflow 定義 → facet 展開エンジン**
  - facet 展開時、展開エンジンは PathAliases（`path_alias.*` の解決元）と workflow 定義の変数宣言（`vars.*` の解決元）の両方を受け取り、namespace ごとに解決する。
- **agent ← facet 展開結果**
  - agent には既に展開済みの facet 本文が渡る。alias 名や変数値を agent 側で組み立てない。

## State Ownership

- **CLI alias 名・実行 binary・内包データ領域**: `PathAliases`（バックエンドの起動時確定値）。
- **workflow 変数の宣言値**: workflow 定義（ストレージ上の workflow ファイル）。
- **実行時に積み上がる workflow 実行変数**: 既存の workflow engine（execution 状態）。本タスクで導入する宣言型変数とは別領域として並存する。
- **`/usr/local/bin/releash` の指す実体**: 本番 binary の install 経路のみが所有。dev binary は所有しない。
- **`RELEASH_DATA_DIR` の最終値**: 子プロセスの環境変数として確定。決定順序は契約に記載のとおり。

## Boundaries

- フロントエンドは alias 文字列・CLI コマンド文字列・データディレクトリパスを自前で組み立てない。
- agent は facet 本文の文字列置換を行わない。展開済み本文だけを受け取る。
- facet 展開エンジンは workflow execution 状態（実行時変数）の意味づけに踏み込まない。`vars.*` namespace は宣言型変数の解決にのみ使う。
- CLI install 経路は dev binary を本番 alias の所有者にしない。dev 起動が本番 alias の実体を書き換える経路は存在させない。
- `RELEASH_DATA_DIR` の利用者明示指定は alias 内包値で上書きしない。
- 既存プレースホルダ namespace と新規 namespace（`path_alias` / `vars`）は同一キーで衝突させない。
- 本タスクの範囲は `path_alias.releash` のみ。任意の alias 種別の公開・一般化はスコープ外。

## Implementation Freedom

- `PathAliases` の具体的な型表現（struct 構成、`HashMap` か固定フィールドか等）。
- facet 展開エンジン内部での namespace 解決の合成方法（事前マージ vs 名前空間別ルックアップ）。
- 未定義変数エラー検出の一次境界は workflow 読み込み時で確定。保存時・展開時で重複検出を行うかは実装自由。
- workflow 定義ファイルにおける変数宣言の表記（フィールド名・位置・YAML 構造）。
- dev alias binary の物理配置（resource bundle 内 / 別パス）と、`PATH` への追加方法。
- 子プロセス環境ビルダーの実装単位（共通 helper として切り出すか、各起動経路に同等のロジックを持たせるか）。
- `RELEASH_DATA_DIR` の明示指定検出方法（env 直読み / 起動コンテキスト経由）。
- facet 展開エンジン内における namespace 対応の API 形（既存 helper の拡張か新規導入か含む）。
