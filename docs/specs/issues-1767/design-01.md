# Design 01

## 開始状態

初回。直前の Design は無く、実装の状態は `docs/specs/issues-1767/requirements.md` の Current Behavior を参照する。

- 差分の基準: base ブランチ `main`、派生点 commit 8f6a107a（`release: v0.4.12 (#1756)`）。未コミットの変更は `docs/specs/issues-1767/` の `requirements.md` / `behavior.md` だけで、コードは派生点のままである。
- 依存する #1734（YAML 表面の delegate）は commit 238abe4f で取り込み済みである。
- この周までに解消・見送りとなった Thread は無い。

## 変える部分

- Session handle への delegate メソッド: Session の handle が `delegate{ child, inputs?, when, max_iterations }` を受理する。`child` / `when` / `max_iterations` は必須、`inputs` は任意で、`child` は Node 値、`inputs` は `<パラメータ名> = <Source>` の table、`when` は Source または Predicate、`max_iterations` は1以上の整数である。根拠: R-001、R-002、B-001、B-002、B-003。ルート: 宣言箇所を Lua の `completion` table ではなく Session handle のメソッドにする（「固定するルート」）。
- delegate の `inputs` の供給元: 親 Session の Input、親 Session 自身の Artifact（handle そのもの、`work.<field>...`、`work.child.<field>...`）、`r.request` を受理し、YAML で同じ供給元を書いた場合と同じ配線に解決する。根拠: R-003、B-004。ルート: 委任。
- delegate の `when` の供給元: 親 Session 自身の Artifact の field と `work.child.<field>...` を、単独または `r.all` / `r.any` で合成して受理する。根拠: R-004、B-005。ルート: 委任。
- `child` 段の走査: `work.child.<field>...` を delegate を宣言した Session の handle でだけ辿れるようにし、child が Session / Command なら child の Artifact を直接、Sequence / Fanout なら統合 map を辿る。delegate を宣言していない Session の `child` 段の参照は、YAML で同じ参照を書いた場合と同じ Diagnostic にする。根拠: R-005、B-006、B-007。ルート: 委任。
- child の Artifact Contract による型検査: `inputs` / `when` の `work.child.<field>...` を load 時に child の Artifact Contract へ照合する。根拠: R-006、B-008。ルート: 委任。
- `require` との and 合成: `completion = { require = r.completion.approval }` と handle メソッドの delegate を併記した Session が、両方を持つ定義になる。根拠: R-007、B-009。ルート: 委任。
- delegate だけから参照される child の到達可能性: 合成子の children に置かれていない Node が delegate の `child` としてだけ参照された場合も、定義に含まれ到達可能として扱われる。開始状態の Lua の定義構築は main から合成子の children だけを辿る（`src-tauri/src/adaptor/gateway/workflow/lua/mod.rs:1596-1602`）。根拠: R-008、B-010。ルート: 委任。
- YAML 定義との同一性: 同じ delegate を書いた Lua と YAML の定義が同一の `WorkflowDefinition` を構築する。根拠: R-009、B-011。ルート: 委任。
- YAML と同じ Diagnostic: `artifact` を宣言していない Session への宣言、必須 field の欠落、未知キー、1未満の `max_iterations`、親 Session 自身や root を `child` にする宣言、他の合成子や別の delegate と child を共有する宣言が、YAML と同じ `code` / `stage` / `message` になる。根拠: R-010、B-012。ルート: 委任。
- 同じ handle への二重宣言: 2回目以降の delegate 宣言が `WFS002` / `parse_shape` の Error Diagnostic になる。根拠: R-011、B-013。ルート: 委任。
- Artifact Contract 直下の `delegate` field との関係: `delegate` field を宣言した Session の handle でも `delegate` は宣言メソッドへ解決し、Error Diagnostic は増えない。根拠: R-015、B-017。ルート: 委任。
- 生成 stub の注釈: `.releash/releash.lua` が Session handle の `delegate` メソッドと `child` 段の注釈を含む。開始状態の stub は `ReleashCompletion` に `require` だけを持ち、`ReleashNode` に `delegate` も `child` も無い（`src-tauri/src/adaptor/gateway/workflow/lua/stubs.rs:65,77-79`）。根拠: R-013、B-015。ルート: 生成 stub は `.releash/releash.lua`（「固定するルート」）。
- 用語集の更新: `docs/glossary/WORKFLOW.md` の「Lua」節と「Lua API」節に Lua 表面での delegate の受理形を記述し、「Lua の `completion` table は `require` だけを受理し、`delegate` は受理しない」（`:448`）と「`delegate` は未対応」（`:497`）の記述を残さない。根拠: R-014、B-016。ルート: 更新先は同ファイルの「Lua」節と「Lua API」節（「固定するルート」）。

## 固定するルート

- delegate の宣言箇所は Lua の `completion` table ではなく Session handle のメソッドにする。#1732 の Design が記す「Lua の `completion` を table にしたのは #1734 で delegate を足すため」という方向性は維持し、足す場所だけを変える。
- 生成 stub は `.releash/releash.lua` に、Session handle の `delegate` メソッドと `child` 段の注釈を足す。
- 文書の更新先は `docs/glossary/WORKFLOW.md` の「Lua」節と「Lua API」節。
- テストは `src-tauri/src/adaptor/gateway/workflow/lua/` の既存 completion テストに、delegate の受理、Diagnostic の一致、YAML との定義同一性を追加する。
- 上記以外の実装上のルートは委任。

## 変えないもの

- YAML 表面の受理形と Diagnostic。理由: Issue が変更対象から明示的に外している。
- `completion` table 内の `delegate` キーの拒否（`WFS002`）。開始状態で `completion` map は `require` 以外のキーを拒否しており（`src-tauri/src/adaptor/gateway/workflow/lua/mod.rs:2467-2474`）、R-012 はこの周では変更を伴わない。理由: 自己参照を書けないため、この形を受理しない判断を維持する。
- Artifact Contract 直下に `delegate` field を持つ定義の、YAML からの参照の解決と Diagnostic。理由: Q-002 の決定により、現在 load できる定義を新しい Error Diagnostic で拒否せず、対象範囲も YAML 表面へ広げない。

## 未確定・リスク

- 自動判断（Q-001、R-011）: 同じ Session handle への2回目以降の delegate 宣言は、YAML に対応する誤りが存在しないため、Lua 表面の shape の誤りとして既存の `WFS002` / `parse_shape` を再利用する。新しい `code` は追加せず、`message` の文言は固定していない。
- 自動判断（Q-002、R-015）: Artifact Contract 直下に `delegate` field を宣言した Session でも、Lua の `work.delegate` は handle のメソッドへ解決する。帰結として、その field を Lua の値参照の供給元にはできない。
- 未決のまま残した要求は無い。
- `[DEFERRED]` で人間へ渡した件は無い。
