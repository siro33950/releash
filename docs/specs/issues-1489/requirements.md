# Requirements

関連: #1489（loop_guard に実行回数のリセット境界を指定できるようにする）

## Type

Workflow routing 機能拡張。

`loop_guard` が参照する対象 Node の実行回数を、Workflow 実行全体の累計だけでなく、同じ Workflow 内の指定 Node が直近に正常完了した時点を境界として数えられるようにする。

## 背景と目的

現行の `loop_guard` は、対象 Node が同一 Workflow 実行内で開始された累計回数を `max_iterations` と比較する。外側の処理ラウンドを表す Node が再度正常完了しても累計回数はリセットされない。

このため、外側の処理を複数回行い、その各回で内側の確認・修正ループを一定回数だけ許可する制約を表現できない。たとえば、Full Review を最大 3 回行い、各 Full Review で整合性確認・修正を最大 2 回許可したい場合、現行構文では「全 Full Review を通して最大 2 回」または「1 回の Full Review で最大 6 回」のいずれかになり、「各 Full Review で最大 2 回」を表現できない。

本変更の目的は、`loop_guard` に任意の `reset_on` を追加し、指定 Node の正常完了ごとに新しいカウント範囲を開始できるようにすることである。

```yaml
- name: check_fix_policy_consistency
  rules:
  - loop_guard:
      max_iterations: 2
      on_exhausted: create_fix_plan
      reset_on: full_review_fanout
```

この例では、`full_review_fanout` が正常完了するたびに `check_fix_policy_consistency` の新しいカウント範囲を開始する。

## 用語

- **guard 対象 Node**: `loop_guard` を持ち、遷移可否を実行回数で制限される Node。
- **リセット境界 Node**: `loop_guard.reset_on` が参照する Node。
- **正常完了**: Node の実行が成功として完了したこと。失敗、中断、abort、実行開始だけでは正常完了に含めない。
- **カウント範囲**: Workflow 開始、またはリセット境界 Node の直近の正常完了より後から、現在の routing 判定までの区間。
- **範囲内実行回数**: カウント範囲内で guard 対象 Node の実行が開始された回数。retry または中断後の resume により新しい attempt が開始された場合も、既存 `loop_guard` の回数単位に従って含める。

## スコープ

1. `loop_guard` に、同じ Workflow 内の Node 名を指定する任意 field `reset_on` を追加する。
2. `reset_on` を指定した guard では、リセット境界 Node の直近の正常完了より後に開始された guard 対象 Node の実行回数だけを `max_iterations` と比較する。
3. リセット境界 Node がまだ正常完了していない場合は、Workflow 開始以降の実行回数を使用する。
4. リセット境界 Node が複数回正常完了した場合は、その都度新しいカウント範囲を開始する。
5. fanout Node を `reset_on` に指定した場合は、全 child の完了を含む既存の fanout 正常完了条件が成立した時点を境界とする。
6. Workflow の中断・再開および永続イベント履歴からの再生後も、同じカウント範囲と範囲内実行回数を復元する。
7. Workflow の load、保存、実行開始、永続化、再生、既存 DTO 取得経路の全てで `reset_on` を欠落させず保持する。
8. `docs/workflow-yaml-syntax.md` に構文と意味論を記載する。

## 非スコープ

- CLI または UI から、実行中 Workflow の loop guard 回数を手動で変更・リセットする機能。
- 実行中の Workflow 定義または `reset_on` を変更する機能。
- `max_iterations` および `on_exhausted` の既存の意味や上限判定を変更すること。
- Workflow 実行をまたいでカウント範囲または実行回数を共有すること。
- `reset_on` 専用の Workflow 編集 UI、入力補完 UI、または実行回数表示 UI の追加。
- 既存 builtin Workflow へ `reset_on` を一括適用すること。

## 要求事項

- **R1: 構文**
  `loop_guard` は任意の `reset_on: <node-name>` を受け付けること。`reset_on` には同じ Workflow 定義内の Node 名を指定できること。

- **R2: リセット境界の成立条件**
  `reset_on` に指定された Node が正常完了するたびに、その正常完了より後を新しいカウント範囲とすること。指定 Node の開始、失敗、中断または abort では新しいカウント範囲を開始しないこと。

- **R3: 範囲内回数による routing**
  `reset_on` を持つ `loop_guard` への遷移判定では、現在のカウント範囲内で開始された guard 対象 Node の実行回数を `max_iterations` と比較すること。`max_iterations: 2` の場合、各カウント範囲で guard 対象 Node へ最大 2 回遷移できること。

- **R4: 境界未到達時の初期範囲**
  `reset_on` に指定した Node がまだ正常完了していない場合、Workflow 開始をカウント範囲の始点とすること。

- **R5: 複数回の境界到達**
  リセット境界 Node が複数回正常完了した場合、routing 判定は常に直近の正常完了より後の範囲内実行回数を使用すること。以前のカウント範囲で消費した回数は新しい範囲へ持ち越さないこと。

- **R6: 上限到達時の互換性**
  現在のカウント範囲で実行回数が `max_iterations` に到達している場合、guard 対象 Node を開始せず、既存と同じく `on_exhausted` に遷移すること。`on_exhausted` の chain 処理を含む既存 routing の意味を変更しないこと。

- **R7: `reset_on` 省略時の後方互換性**
  `reset_on` を省略した既存 Workflow は、Workflow 実行全体の累計実行回数を使用する現行挙動を維持すること。既存 YAML および `reset_on` を持たない永続済み Workflow 定義を引き続き読み込めること。

- **R8: 参照検証と Diagnostic**
  `reset_on` が同じ Workflow 内に存在しない Node 名を参照した場合、Workflow load 時に定義を拒否し、参照された Node 名と `loop_guard.reset_on` が原因であることを識別できる Diagnostic を返すこと。`reset_on` は routing edge ではないため、到達可能性または cycle の遷移先として扱わないこと。

- **R9: fanout 境界**
  fanout Node を `reset_on` に指定した場合、個々の child の開始または完了ではリセットせず、全 child の完了を含む fanout Node 自体の正常完了を境界とすること。

- **R10: 中断・再開とイベント再生**
  Workflow の中断・再開後、および永続イベント履歴から同じ Workflow 実行を再生した後も、直近の正常完了境界と範囲内実行回数が中断前と一致し、同じ routing 判定になること。

- **R11: 定義表現の保持**
  schema、domain、runtime routing、永続化・再生、Workflow 定義 DTO の各経路で `reset_on` を保持すること。既存 DTO が `loop_guard` を返す場合、指定済みの `reset_on` も返すこと。

- **R12: Rust 所有**
  カウント範囲、正常完了境界、範囲内実行回数および routing 判定の source of truth は Rust backend が所有すること。frontend に Workflow runtime の回数計算またはリセット判断を追加しないこと。

- **R13: ドキュメント**
  `docs/workflow-yaml-syntax.md` に `reset_on` の構文、正常完了だけが境界になること、境界未到達時の範囲、fanout の成立条件、および省略時の後方互換性を記載すること。

## 受け入れ基準の概要

- `loop_guard.reset_on` に同じ Workflow 内の通常 Node または fanout Node を指定できる。
- 通常 Node が正常完了するたびに新しいカウント範囲が開始され、`max_iterations: 2` なら各範囲で最大 2 回 guard 対象 Node を開始できる。
- 境界 Node が未到達の場合は Workflow 開始以降の回数を使用する。
- 境界 Node が複数回正常完了した場合は、直近の正常完了以降だけを数える。
- 境界 Node の失敗、中断または abort ではカウント範囲を更新しない。
- fanout Node は全 child の完了を含む fanout 自体の正常完了時だけ境界になる。
- 各カウント範囲で上限へ到達すると、既存どおり `on_exhausted` へ遷移する。
- `reset_on` を省略した既存 Workflow の累計回数による挙動が変わらない。
- 存在しない Node 名を `reset_on` に指定すると、Node 名を含む Diagnostic が Workflow load 時に返る。
- 中断・再開およびイベント履歴からの再生後も、カウント範囲、範囲内実行回数、routing 結果が一致する。
- YAML、domain、永続化された Workflow 定義、再生、DTO の往復で `reset_on` が欠落しない。
- 通常 Node 境界、fanout Node 境界、境界未到達、複数回到達、非正常終了、中断・再開、存在しない参照、および `reset_on` 省略時の互換性を検証する Rust テストが追加される。
- `docs/workflow-yaml-syntax.md` に R13 の内容が記載される。
- `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test` が通る。frontend の DTO 型または表示を変更した場合は `pnpm lint` / `pnpm test` / `pnpm build` も通る。

## 仮定

- `max_iterations` が数える実行単位は現行 `loop_guard` の意味を維持し、Node の新しい attempt が開始された回数とする。
- `reset_on` はリセット境界への参照であり、Workflow の control-flow edge を新設しない。
- Workflow 実行中は、実行開始時に確定した Workflow 定義を使用し続ける。
- 既存の Workflow event 履歴は、正常完了境界と実行回数を一意に復元できる順序を保持している。

## Open Questions

なし
