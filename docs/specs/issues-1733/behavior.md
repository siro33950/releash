## B-001: `worktree` field の受理

GIVEN Session、Command、Fanout、Sequence のいずれかの Node に、YAML で `worktree: isolated` または `worktree: shared` を書いた定義、および Lua で `worktree = r.worktree.isolated` または `worktree = r.worktree.shared` を書いた定義がある
WHEN それぞれの定義を load する
THEN どちらの表面でも Error Diagnostic なく load できる
AND `worktree` を省略した Node は `shared` として扱われる

## B-002: `worktree` の値域外の値

GIVEN YAML の Node に `shared` / `isolated` 以外の値の `worktree` を書いた定義、および Lua の Node に `r.worktree.*` の handle 以外の値（`"isolated"` などの文字列を含む）の `worktree` を書いた定義がある
WHEN それぞれの定義を load する
THEN どちらの表面でも Error Diagnostic になり、その定義は実行できない

## B-003: Session / Command の隔離実行

GIVEN `worktree: isolated` を宣言した Session または Command が、実行木の root worktree の直下で実行される
WHEN その Node の attempt が開始する
THEN root worktree の HEAD から新しい branch が作られ、その branch を checkout した新しい worktree が生成される
AND その Node の process は生成された worktree を cwd として実行される
AND Command では `RELEASH_WORKTREE_PATH` が生成された worktree の path であり、Session では生成された worktree を起動 worktree として provider が起動される
AND branch 名と worktree の path は #1467 で定義済みの命名規則に従う
AND worktree は repository root の内側には作られない

## B-004: Sequence の隔離実行

GIVEN `worktree: isolated` を宣言した Sequence があり、その children は `worktree` を宣言しない
WHEN その Sequence の実行が開始し、children が順に実行される
THEN その Sequence の実行に対して隔離 worktree が1つ生成される
AND すべての children はその同じ隔離 worktree を cwd として実行される

## B-005: Fanout の隔離実行

GIVEN `worktree: isolated` を宣言した Fanout があり、その children エントリは `worktree` を宣言しない
WHEN その Fanout の実行が開始し、slot が並走する
THEN その Fanout の実行に対して隔離 worktree が1つ生成される
AND すべての slot はその同じ隔離 worktree を cwd として実行される

## B-006: Fanout の children の隔離実行

GIVEN Fanout の children エントリが参照する Node が `worktree: isolated` を宣言し、`items` によって複数の slot に展開される
WHEN 各 slot が並走し、それぞれが同じ path のファイルを編集する
THEN slot ごとに独立した隔離 worktree が生成され、各 slot は自分の隔離 worktree を cwd として実行される
AND ある slot の編集は他の slot の worktree にも親 worktree にも現れない

## B-007: `shared` の継承

GIVEN `worktree: isolated` を宣言した Sequence の child が `worktree: shared` を宣言する、または `worktree` を省略する
WHEN その child が実行される
THEN その child は Sequence の隔離 worktree を cwd として実行され、新しい worktree は生成されない

## B-008: 隔離実行の入れ子

GIVEN `worktree: isolated` を宣言した Sequence の child が `worktree: isolated` を宣言する
WHEN その child の attempt が開始する
THEN Sequence の隔離 worktree の HEAD から branch した新しい隔離 worktree が生成され、その child はそこを cwd として実行される

## B-009: attempt ごとの新しい worktree

GIVEN `worktree: isolated` を宣言した Node の attempt が、隔離 worktree 内に未コミットの変更を残して失敗した
WHEN その Node が再実行され attempt が進む
THEN 新しい branch と worktree が生成され、新しい attempt はそこを cwd として実行される
AND 新しい worktree には前 attempt の未コミットの変更が含まれない

## B-010: 隔離 worktree の一覧非表示

GIVEN `worktree: isolated` を宣言した Node の attempt が隔離 worktree を生成して実行中である
WHEN Worktree 管理の一覧を参照する、または Releash を再起動してから一覧を参照する
THEN その隔離 worktree は作業の場の一覧にも、それ以外のどの節にも現れない
AND その Node の実行が終了した後も同じく現れない

## B-011: 隔離 worktree の実体喪失

GIVEN `worktree: isolated` を宣言した Session の attempt が provider process の終了で失敗して resume 可能であり、その隔離 worktree の実体が外部で削除された
WHEN その attempt を resume する
THEN process は起動できず、その attempt は Node failure になる
AND その Node に隔離環境喪失を示す recovery reason は付かない
AND 手動 Retry は新しい branch と worktree で新しい attempt を始める

## B-012: Artifact への `worktree` キーの付与

GIVEN `worktree: isolated` と `artifact` を宣言した Session または Command がある
WHEN その Node が Contract に従う Artifact を産出して完了する
THEN その Node の Artifact には Contract の field に加えて `worktree` キーがある
AND `worktree.branch` はその attempt の隔離 branch 名、`worktree.path` はその attempt の隔離 worktree の path である

## B-013: `artifact` を宣言しない isolated な Node の Artifact

GIVEN Sequence の child と Fanout の child にそれぞれ、`artifact` を宣言せず `worktree: isolated` を宣言した Session がある
WHEN それらの Session が完了する
THEN 各 Session は `worktree` キーだけを持つ Artifact を産出する
AND Sequence の統合 map にはその child のキーが現れ、値はその Artifact である
AND Fanout の map ではその slot の値が `null` ではなくその Artifact である

## B-014: 合成子の Artifact への `worktree` キーの付与

GIVEN `worktree: isolated` を宣言した Sequence または Fanout がある
WHEN その合成子の実行が完了する
THEN その合成子の Artifact には children の map のキーに加えて `worktree` キーがあり、その実行の隔離 branch 名と worktree の path を持つ

## B-015: `worktree` 予約 field の衝突

GIVEN 直下に `worktree` field を持つ Contract を `artifact` に参照した Node があり、その Node は `worktree` を宣言しない
WHEN その定義を load する
THEN Error Diagnostic になり、その定義は実行できない
AND その Node に `worktree: isolated` または `worktree: shared` を宣言しても結果は同じである

## B-016: 配線からの `worktree` の参照

GIVEN `worktree: isolated` を宣言した Node `work` があり、下流の Command が配線 `inputs` で `branch: work.worktree.branch` を受け、`env` でそのパラメータを環境変数に割り当てる
WHEN その定義を load して実行する
THEN その定義は Error Diagnostic なく load できる
AND 実行時、その環境変数には `work` の attempt の隔離 branch 名が渡る

## B-017: 合成子経由の参照と `artifact` を宣言しない Session の参照

GIVEN Sequence `seq` の child に、`artifact` を宣言せず `worktree: isolated` を宣言した Session `work` がある
WHEN `seq` の外の Node の配線 `inputs` の供給元に `seq.work.worktree.path` を書いた定義を load して実行する
THEN その定義は Error Diagnostic なく load できる
AND 実行時、その input パラメータには `work` の attempt の隔離 worktree の path が渡る

## B-018: Artifact 経由の branch / path の観測

GIVEN `worktree: isolated` を宣言した Node が Artifact を産出して完了している
WHEN local API でその execution または Artifact を取得する、CLI で `releash workflow status --json` または `releash workflow output get` を実行する
THEN いずれの経路でも、その Node の Artifact の `worktree.branch` と `worktree.path` を読める

## B-019: 統合を行わない

GIVEN Sequence の child `work` が `worktree: isolated` を宣言し、後続の child `next` は `worktree` を宣言しない
WHEN `work` が隔離 worktree 内でファイルを変更して完了し、`next` が実行される
THEN `next` は親 worktree を cwd として実行され、`work` の変更はその cwd に現れない
AND `work` の変更は `work` の隔離 branch と worktree に残る

## B-020: 正本サンプルと builtin の load

GIVEN `implement_and_verify` と `fix_and_verify` に `worktree: isolated` を宣言した `workflows/examples/full-cycle-development.yml`、および `workflows/*.yml` の builtin 8本
WHEN それぞれを load する
THEN Diagnostic はゼロである
AND 正本サンプルの `implement_all` / `fix_all` の各 slot は独立した隔離 worktree で並走する

## B-021: 正本ドキュメントでの `worktree` の説明

GIVEN workflow 定義の書き手が `docs/glossary/WORKFLOW.md` で `worktree` を確認する
WHEN Node 共通 field、予約 field、Artifact の有無、参照の各記述を読む
THEN `worktree` は `shared` / `isolated` を値域とする解禁済みの Node 共通 field として、省略時の意味と種別ごとの隔離の範囲とともに説明されている
AND 「Lua API」の表に `r.worktree.shared` / `r.worktree.isolated` の handle と各 Node builder の `worktree?` field が記載されている
AND Artifact の `worktree` キー、予約キーの検査、`isolated` な Node が `artifact` 宣言なしでも Artifact を持つことが説明されている
AND 「未解禁」「`WFU002`」「成功する定義には `worktree` を書かない」の記述は残っていない

## B-022: 正本ドキュメントでの隔離 worktree の説明

GIVEN 読み手が `docs/glossary/DOMAIN.md` の隔離 worktree 節を確認する
WHEN 生成と統合の記述を読む
THEN 隔離 worktree が attempt ごとに親 worktree の HEAD から生成されること、命名が識別の根拠であること、engine が統合を行わないこと、逐次 Node で `isolated` を使った成果が後続 Node から見えないことが説明されている
AND 「定義上の `worktree` field は現時点では未解禁である」の記述は残っていない
AND 台帳、lifecycle fact、recovery fence、掃除候補の記述は残っていない

## B-023: 実行中の branch / path の観測

GIVEN `worktree: isolated` を宣言した Node の attempt が隔離 worktree を生成して実行中であり、Artifact はまだ産出されていない
WHEN UI でその Node を参照する、local API でその execution を取得する、CLI で `releash workflow status --json` を実行する
THEN いずれの経路でも、その attempt の隔離 branch 名と worktree の path を読める
AND その attempt が Artifact を産出せずに失敗または abort で終わった後も、同じ経路で同じ値を読める
AND その attempt が Artifact を産出して完了した後も、同じ経路で同じ値を読め、UI に表示される branch / path はその Artifact の `worktree.branch` / `worktree.path` と同じ値である

## B-024: 隔離 worktree からの Thread の参照

GIVEN 実行木が属する Workspace に open な Thread があり、`worktree: isolated` を宣言した Command と Session がその実行木で実行中である
WHEN その Command が `releash review list --state open` を、その Session の agent が `releash review list --state open --session-id <自分の session id>` を実行する
THEN どちらもその Workspace の open な Thread を返す
AND 隔離 worktree の path を Workspace とする Thread 集合は作られない

## B-025: 隔離 worktree の生成失敗

GIVEN `worktree: isolated` を宣言した Session または Command を参照する children エントリが `on_failure: { retry: 1 }` を宣言し、その attempt の branch / worktree の生成が失敗する
WHEN その attempt が開始する
THEN その attempt は Node failure として失敗し、process は起動されない
AND 新しい attempt が自動で始まり、その attempt のために新しい branch と worktree が生成される
AND `on_failure` を宣言しないエントリで同じ失敗が起きた場合は、その Node は失敗のまま中断し、手動 Retry を待つ
AND 失敗のまま中断した attempt は、Releash を再起動した後も失敗として復元され、`worktree` を宣言しない Command の process 起動不能で失敗した attempt も同じく失敗として復元される

## B-026: milestone design での台帳記述の削除

GIVEN 読み手が `docs/specs/milestone-85/design.md` §3.2 を確認する
WHEN 生成と記録の記述を読む
THEN 台帳・reconciliation・事実3種を実装済みの前提として挙げる記述は残っておらず、命名規則と本 Issue の生成・観測の規則が記述されている

## B-027: 合成子の branch / path の UI での観測

GIVEN `worktree: isolated` を宣言した Sequence または Fanout の attempt が隔離 worktree を生成して実行中である
WHEN UI でその合成子を参照する
THEN その合成子自身の attempt の隔離 branch 名と worktree の path を読める
AND その Fanout の `items` が空に解決され children を一つも持たない場合も、同じ値を読める
AND その attempt が完了、失敗、または abort で終わった後も、同じ値を読める

## 要件IDとBehavior IDの対応表

| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001, B-002 |
| R-002 | B-003, B-007, B-008, B-025 |
| R-003 | B-003, B-004, B-005, B-006 |
| R-004 | B-009 |
| R-005 | B-010, B-011 |
| R-006 | B-003 |
| R-007 | B-012, B-013, B-014 |
| R-008 | B-015 |
| R-009 | B-016, B-017 |
| R-010 | B-018, B-023, B-027 |
| R-011 | B-019 |
| R-012 | B-020 |
| R-013 | B-021, B-022, B-026 |
| R-014 | B-024 |
