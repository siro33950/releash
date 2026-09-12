# Context

- 要求の正本: https://github.com/siro33950/releash/issues/1767
- 背景資料
  - `docs/specs/milestone-85/design.md` §2.3（delegate の構文、`inputs` は合成子の children エントリと同形、供給元は親 session のスコープで参照できるもの）
  - `docs/glossary/WORKFLOW.md`（`completion`、「Session の delegate」、「予約語」、「Lua」、「Lua API」、「Diagnostic」）
  - `docs/specs/issues-1734/requirements.md`（YAML 表面の delegate の確定要求。R-019 は「Lua の `completion` table は `require` だけを受理し `delegate` を受理しない」を WORKFLOW.md に記述することを求める）
  - `src-tauri/src/adaptor/gateway/workflow/lua/mod.rs`、`src-tauri/src/adaptor/gateway/workflow/completion_wire.rs`、`src-tauri/src/adaptor/gateway/workflow/lua/stubs.rs`
- 確定済みの背景と制約
  - workflow 定義は YAML と Lua の2表面を持ち、どちらも同じ `WorkflowDefinition` を構築する。同じ定義上の誤りには同じ `code` / `stage` / `message` の domain Diagnostic を使う（WORKFLOW.md「Diagnostic」）。
  - `delegate` は `artifact` を宣言した Session だけが受理する。`child` / `when` / `max_iterations` は必須、`inputs` は任意である（WORKFLOW.md「Session の delegate」）。
  - YAML の delegate `inputs` の供給元は親の `input` パラメータ、親 Session の Node 名による親 Artifact、`request` に限る。`when` は親の提出 field と `child` 以下の field を使う（同上）。
  - YAML の delegate `inputs` は Node 名だけを書いて親 Artifact 全体を供給できる（WORKFLOW.md の delegate 例の `result: implement`）。Lua の既存の配線も Node 値そのものを Source として受理する（`src-tauri/src/adaptor/gateway/workflow/lua/mod.rs:1452`）。
  - `child` は delegate を宣言した Session の Artifact 直下だけの予約キーであり、child が Session / Command なら直接、Sequence / Fanout なら統合 map を辿る（WORKFLOW.md「Session の delegate」「予約語」）。
  - delegate だけから参照される child も到達可能である（同上）。
  - 依存する #1734 は commit 238abe4f で取り込み済みである。

# Outcome

- 対象者: Lua で workflow 定義を書く開発者。
- 現在の問題: delegate は YAML でだけ宣言できる。Lua では `completion` table の `delegate` が Error Diagnostic になり、delegate を使う定義を Lua 表面で書けない。Lua の値参照には親 Session 自身の Artifact を指す手段がなく、#1734 は Lua 表面の受理形を決めずに Non-goal へ置いた。
- 変更後に実現する状態: delegate の所有者である親 Session の handle のメソッドとして delegate を宣言でき、`when` / `inputs` から親自身の Artifact と child の Artifact を既存の値参照で指せる。同じ delegate を書いた YAML 定義と同一の `WorkflowDefinition` になり、定義上の誤りには YAML と同じ Diagnostic が出る。

# Current Behavior

最初の周の開始時点（commit 8f6a107a）で、コードと文書の読解により確認した。build / test / lint は実行していない。

- `completion = { require = r.completion.approval, delegate = {...} }` を含む `.lua` を load すると、`completion` map が `require` 以外のキーを持つため `WFS002` / `parse_shape` / `completion map only accepts the key 'require'` になる（`src-tauri/src/adaptor/gateway/workflow/lua/mod.rs:2467-2474`）。
- Session handle に `delegate` メソッドは無い。`work.delegate` は Node handle の index として Artifact field への Source を作り（`src-tauri/src/adaptor/gateway/workflow/lua/mod.rs:709-747`）、その値の呼び出しは host の UserData が `Index` メタメソッドしか持たないため（`src-tauri/src/infrastructure/lua/evaluator.rs:206-221`）Lua の評価失敗になり、`WFS010` で load が止まる（`src-tauri/src/adaptor/gateway/workflow/lua/mod.rs:162-166`）。
- YAML 側は `completion.delegate` の `child` / `inputs` / `when` / `max_iterations` を受理し、未知キー・`child` 欠落・`when` 欠落・`max_iterations` の型を parse/shape で拒否する（`src-tauri/src/adaptor/gateway/workflow/completion_wire.rs:35-135`）。Session 限定、`artifact` 必須、`max_iterations >= 1`、child が Artifact を持つこと、child の共有と親自身・root の指定は domain の validation が検査する（`src-tauri/src/domain/workflow/services/validation.rs:885-1000`）。
- 生成 stub は `ReleashCompletion` に `require` だけを持ち、`ReleashNode` は `ReleashSource` を継承するだけで `delegate` も `child` も持たない（`src-tauri/src/adaptor/gateway/workflow/lua/stubs.rs:65`、`:77-79`）。
- `docs/glossary/WORKFLOW.md:448` は「Lua の `completion` table は `require` だけを受理し、`delegate` は受理しない」、`:497` は「`completion` table は `require` のみ受理。`delegate` は未対応」と記述する。

# Scope / Non-goals

変更する。

- Lua 表面での Session の delegate の受理形（Session handle のメソッド）。
- Lua での親自身の Artifact と child の Artifact への参照（`work.<field>`、`work.child.<field>...`）。
- Lua 表面の delegate に対する Diagnostic。
- 生成 stub `.releash/releash.lua` の注釈。
- `docs/glossary/WORKFLOW.md` の「Lua」節と「Lua API」節。

変更しない。

- YAML 表面の受理形と Diagnostic。
- `completion` table 内に `delegate` を書く形（`completion = { delegate = {...} }`）の受理。自己参照を書けないため採らない。
- delegate の実行時の挙動（発火、評価の時点、注入、`max_iterations` の意味、resume）。
- #1734 で確定した delegate の規則そのもの（child の条件、`artifact` 必須、`require` との and、`child` 予約キーの範囲）。

# Requirements

- R-001: Lua の Session handle は `delegate{ child, inputs?, when, max_iterations }` を受理する。`child` / `when` / `max_iterations` は必須、`inputs` は任意である。
- R-002: `child` は Node 値、`inputs` は `<パラメータ名> = <Source>` の table、`when` は Source または `r.all` / `r.any` が返す Predicate、`max_iterations` は1以上の整数である。
- R-003: delegate の `inputs` の供給元は、親 Session の Input、親 Session 自身の Artifact（`work` 全体、`work.<field>...`、`work.child.<field>...`）、`r.request` である。
- R-004: delegate の `when` の Source は、親 Session 自身の Artifact の field（`work.<field>...`）と child の Artifact の field（`work.child.<field>...`）である。
- R-005: `work.child.<field>...` は delegate を宣言した Session の handle でだけ辿れる。child が Session / Command なら child の Artifact を直接、Sequence / Fanout なら統合 map を辿る。
- R-006: `inputs` / `when` で `work.child.<field>...` を参照した定義は、child の Artifact Contract に沿って load 時に型検査される。
- R-007: Lua の `completion = { require = r.completion.approval }` は従来どおり受理され、`require` と handle メソッドの delegate の併記は and として扱われる。
- R-008: delegate だけから参照される child Node は、合成子の children に置かれていなくても定義に含まれ、到達可能として扱われる。
- R-009: Lua で宣言した delegate は、同じ delegate を YAML で書いた定義と同一の `WorkflowDefinition` を構築する。delegate の `inputs` は `<パラメータ名>` から `<Source>` への対応であり、両表面で同じ対応を書いていれば、`inputs` を書いた順序は同一性の条件に含まない。
- R-010: YAML と同じ定義上の誤りには、YAML と同じ `code` / `stage` / `message` の Diagnostic が出る。対象は、`artifact` を宣言していない Session への宣言、必須 field の欠落、未知キー、1未満の `max_iterations`、親 Session 自身や root を child にする宣言、他の合成子や別の delegate と child を共有する宣言である。
- R-011: 同じ Session handle への2回目以降の delegate 宣言は、`code` が `WFS002`、`stage` が `parse_shape` の Error Diagnostic になる。`message` は同じ Session handle への二重宣言であることを示す。
- R-012: `completion` table 内の `delegate` キーは引き続き受理せず、現行と同じ `WFS002` の Error Diagnostic になる。
- R-013: 生成 stub `.releash/releash.lua` は、Session handle の `delegate` メソッドと `child` 段の注釈を含む。
- R-014: `docs/glossary/WORKFLOW.md` の「Lua」節と「Lua API」節は、Lua 表面での delegate の受理形を記述し、「Lua は `delegate` を受理しない」「`delegate` は未対応」の記述を残さない。
- R-015: Lua の Session handle の `delegate` キーは delegate 宣言のメソッドに解決される。Artifact Contract の直下に `delegate` field を宣言した Session の handle でも同じであり、その field を Lua の値参照の供給元にはできない。この場合に Error Diagnostic は増えず、YAML からの同じ field への参照と Diagnostic は変わらない。
- R-016: YAML で宣言した delegate の `inputs` は記述順のまま扱われる。記述順にもとづく既存の観測結果（child へ渡る入力の順序、保存と復元をまたいだ順序、read model が返す `inputs` の順序）は変わらない。

# Assumptions / Open Questions

- 自動判断: 同じ Session handle への2回目以降の delegate 宣言（R-011）は、Lua 表面の shape の誤りとして扱い、`parse_completion` が Lua の `completion` の shape の誤りに既に使う `WFS002` / `parse_shape` を再利用する。YAML の `completion` map は `delegate` を単一キーとしてしか持てず、対応する誤りが存在しないため「YAML と同じ Diagnostic」を満たす参照先がない。新しい `code` は追加せず、`message` の文言は固定しない。
- 自動判断: Artifact Contract の直下に `delegate` field を宣言した Session でも、Lua の `work.delegate` は handle のメソッドへ解決する（R-015）。`delegate` を予約キーにして新しい Error Diagnostic を足す案と、YAML を含む全表面で予約する案は、いずれも現在 load できる定義を拒否するか YAML 表面を変更するため採らない。この選択の帰結として、`delegate` field を Lua の値参照から指せないことを受け入れる。
- 自動判断: R-009 の「同一」は、delegate の `inputs` を `<パラメータ名>` から `<Source>` への対応として比較した一致を指し、書いた順序を含まない。Lua の table は `pairs` の走査順が言語仕様で規定されず記述順を復元できないため、YAML の記述順へ一致させる読み方は成立しない。YAML 側を並べ替えて両表面の格納順を揃える読み方は、`inputs` の記述順を保持する既存の観測結果を変えるため採らず、その維持を R-016 として記す。
