# Requirements

## Type

新機能 / 改善

## Goal

dev 起動の Releash でも、workflow facet 経由で agent に渡される CLI コマンドが dev 側 (`releash-dev` / dev data dir) を確実に指すようにし、かつ facet 本文から workflow 定義由来の変数 (`{{vars.<name>}}`) を参照できるようにする。

完了時には、dev 起動した Releash 内の agent が facet から受け取る CLI コマンドが `releash-dev workflow ...` となり、本番起動した Releash 内の agent が受け取る CLI コマンドが `releash workflow ...` のままで、いずれの環境でも `workflow output submit` 等の動作テストが意図したデータディレクトリ側で完結する。あわせて、workflow 定義で宣言した変数を facet 本文に埋め込んで agent に渡せる。

## Background

現状の Releash には dev / 本番起動の CLI 取り扱いに以下の課題がある。

- macOS 起動時の CLI install が `current_exe()` を `/usr/local/bin/releash` に張るため、dev 起動でも本番 CLI 名 `releash` が debug binary を指してしまい、本番 CLI を壊す。
- Releash CLI の default data dir は `com.releash.app` 固定であり、dev app が `com.releash.app.dev` を使っていても、facet が agent に渡す `releash workflow output submit ...` 系コマンドが本番側データを参照しうる。
- 結果として、dev 環境で workflow の `output submit` / `approve` / `reject` などを agent 経由で実動作テストしづらく、検証のたびに本番状態を汚染する懸念がある。

また facet 本文には現状 `{{project_name}}` / `{{task}}` といった限られたプレースホルダしか展開されず、workflow 定義側で宣言した値を facet から再利用する手段がない。テンプレートに埋め込みたい固有値 (CLI 名や workflow 固有の用語など) を workflow 側で集中管理できない。

これらを解消するため、CLI 名を環境ごとに切り替え可能な抽象（PathAlias）として扱い、facet 本文側からその alias と workflow 変数を共通の置換規則で参照できるようにする。

## Users / Actors

- Releash を dev ビルドで起動して機能検証する開発者
- Releash を本番ビルドで利用するエンドユーザー
- Releash 内で workflow を進行する AgentChat / agent backend
- facet を起点に CLI コマンドを実行する agent
- workflow を定義・編集する作成者（YAML 等で `variables` を宣言する側）

## Scope

- dev 起動 / 本番起動で CLI alias を切り替える仕組み（`releash` / `releash-dev`）を導入する。
- dev alias は単なる debug binary への symlink ではなく、dev データディレクトリを内包した wrapper として振る舞う。
- 本番起動時の CLI install (`/usr/local/bin/releash`) が dev 起動によって debug binary へ張り替わらないようにする。
- agent や facet が起動する子プロセス（PTY / oneshot / agent bridge）に対し、実行環境に応じた alias が解決可能な `PATH` と、実行環境に応じた `RELEASH_DATA_DIR` を伝搬する。
- facet 本文・built-in prompt 内で固定文字列 `releash` を使っている箇所を、環境に応じて展開される `{{path_alias.releash}}` 経由に置き換える。
- workflow 定義に facet 展開用の `variables`（名前→静的文字列値）を workflow 全体共通のスコープで持てるようにする。
- facet 本文から `{{vars.<name>}}` で workflow 定義側の変数を参照できるようにする。
- 未定義の `{{vars.<name>}}` 参照を明示的なエラーとして検出する。
- 既存の `{{project_name}}` / `{{task}}` などのプレースホルダは互換維持する。

## Non-goals

- 既存 workflow の意味そのものを変えること。
- AgentChat の権限モデルや承認 UI の変更。
- 本番 / dev 以外の任意の追加 build profile（staging 等）の新設。
- CLI コマンド体系そのものの再設計（`workflow output submit` 等のサブコマンドの追加・改名）。
- `releash-dev` の Windows / Linux 向けインストール導線の整備（今回は macOS の起動時挙動の修正範囲に留める）。
- `path_alias.releash` 以外の任意の alias 種別を一般化して公開すること。
- facet 内テンプレート言語の本格的な拡張（条件分岐・ループ等の導入）。
- 旧プレースホルダ (`{{project_name}}` / `{{task}}`) の廃止や改名。
- workflow 定義側の `variables` を step / facet 単位で上書きできるようにすること（今回は workflow 全体共通のみ）。
- `{{vars.<name>}}` の値として動的解決値（環境変数・実行時情報等）を扱うこと（今回は静的文字列のみ）。

## Requirements

- 本番起動時、agent から見える CLI alias は `releash` として解決できること。
- dev 起動時、agent から見える CLI alias は `releash-dev` として解決できること。
- dev 起動時、`/usr/local/bin/releash` が debug binary に張り替えられないこと。
- `releash-dev` は dev データディレクトリ (`com.releash.app.dev` 相当) を内包しており、呼び出し側が `RELEASH_DATA_DIR` を明示しなくても dev 側データを参照すること。
- agent が起動する PTY・oneshot・agent bridge では、実行環境に応じた alias を解決できる `PATH` と、実行環境に応じた `RELEASH_DATA_DIR` が設定されること。
- 本番環境では `{{path_alias.releash}}` が `releash` に展開されること。
- dev 環境では `{{path_alias.releash}}` が `releash-dev` に展開されること。
- facet 本文・built-in prompt 内で従来固定で `releash` を出していたコマンド表記は、`{{path_alias.releash}}` 経由で展開されること。
- workflow 定義は facet 展開用の変数群（名前→静的文字列値）を workflow 全体共通スコープで宣言できること。
- facet 本文から `{{vars.<name>}}` で workflow 定義側の変数（静的文字列値）を参照できること。
- 未定義の `{{vars.<name>}}` 参照は workflow 読み込み時に一次検出され、明示的なエラーとして提示されること（保存時・展開時での重複検出は許容する）。
- 既存プレースホルダ (`{{project_name}}` / `{{task}}` 等) の展開結果が従来と変わらないこと。
- 本番アプリ内 agent が facet から受け取る `workflow output submit` 系コマンドは `releash workflow output submit ...` 形式のままであること。
- dev アプリ内 agent が facet から受け取る `workflow output submit` 系コマンドは `releash-dev workflow output submit ...` 形式となること。
- `releash-dev workflow runs` 等の dev alias 経由の CLI 操作が dev データディレクトリを参照すること。

## Constraints

- dev 起動による副作用で本番 CLI (`/usr/local/bin/releash` が指す実体) が破壊されないこと。
- dev / 本番のいずれの環境でも、agent から見える CLI alias と実際の実行対象（バイナリ・データディレクトリ）の対応が一意であること。
- facet テンプレートの未定義参照を黙って空文字に展開しないこと（権限や宛先がずれる事故を防ぐため）。
- workflow 定義側の変数名と既存プレースホルダ（`project_name` / `task` 等）が衝突して既存挙動を変えないこと。
- dev / 本番で同じ facet 文面が異なる CLI コマンドへ安全に展開できること。
- `RELEASH_DATA_DIR` の解決順序が、明示指定 > alias 内包値 > プロセス既定 となり、利用者の明示指定を奪わないこと。
- `{{vars.<name>}}` の値は静的文字列のみとし、環境変数・実行時情報などの動的解決値を含まないこと。

## Success Criteria

- dev 起動を行っても `/usr/local/bin/releash` が debug binary に張り替わらない。
- dev app 内の agent が facet から受け取る CLI コマンドが `releash-dev workflow output submit ...` 形式になっている。
- `releash-dev workflow runs` 等の dev alias 経由 CLI が dev データディレクトリを参照する。
- 本番 app 内の agent が facet から受け取る CLI コマンドが `releash workflow output submit ...` 形式のままである。
- workflow YAML（または同等の定義）に宣言した `variables` を facet 本文から `{{vars.<name>}}` で参照でき、展開結果に反映される。
- 未定義の `{{vars.<name>}}` を含む workflow を読み込んだ時点で明示的なエラーとして通知される。
- 既存の `{{project_name}}` / `{{task}}` を使った facet の展開結果が、本変更前と等価である。

