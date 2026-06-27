# Behavior

対象 Issue: #1248「Agent context epoch と instruction resolution policy を導入する」

本書は `requirements.md` の R1〜R7 / AC1〜AC5 を、実装詳細を含まない**外部から観測可能な振る舞い**として定義する。型名・関数名・モジュール経路といった実装整合の詳細は design / 実装の責務とし、本書では「どの条件で、何が context に入る / 残る / 破棄されるか」という結果だけを規定する。

## 仮定

- **A1**: 本書での「観測」は、Agent へ投入される system context（repo / diff / open editor / mentions / terminal log / workflow state / instructions）に**何が含まれ、どの版（epoch / revision）に属するか**を指す。Agent への送信内容そのものは外部から直接見えないが、context source の保持状態・破棄状態を介して観測可能とみなす。
- **A2**: instruction の対象ファイルは少なくとも `AGENTS.md` と `CLAUDE.md` を含む（requirements A4 を継承）。`CONTEXT.md` 等の追加対象は design で確定する。
- **A3**: 「会話メッセージ履歴の復帰」は #1190 の責務であり、本書は system context の鮮度管理のみを対象とする（requirements A3 を継承）。Scenario 中の「会話履歴」は #1190 側の振る舞いであり、本書では責務境界の確認にのみ用いる。
- **A4**: context source の各取得処理（repo summary 生成・diff 取得・terminal log 要約等）の中身は本書の対象外とし、「取得済みの結果が、どの保持単位・どの replacement ルールに載るか」のみを規定する（requirements R7 を継承）。
- **A5**: epoch は「context 全体の鮮度世代」を表す単位、revision は「個々の context source の版」を表す単位とする。stale 判別は両者の不一致として観測する（design で具体化）。

---

## Feature: Agent context の鮮度管理（epoch / revision）

repo / diff / open editor / mentions / terminal log / workflow state / instructions の各 context source を epoch / revision を持つ保持単位として管理し、stale な context を Agent に投入しない。

### Background

```gherkin
Given Agent session が 1 つ開かれている
And その session に有効な context epoch が 1 つ存在する
And 各 context source（repo summary / diff / open editor / mentions / terminal log / workflow state / instructions）は最新版として現在の epoch に属している
```

### Rule: context source は列挙された 7 種を最小集合として保持される（R1 / AC1）

```gherkin
Scenario: context source の列挙
  Given Agent session の context epoch が初期化されている
  Then context source として少なくとも次が保持の対象である
    | source              |
    | repo summary        |
    | diff / review snapshot |
    | open editor / selection |
    | mentions            |
    | terminal log summary |
    | workflow run / step state |
    | project instructions（AGENTS.md 相当） |
  And 各 context source はそれが属する epoch / revision で識別できる
```

### Rule: stale な context は Agent に投入されない（R2 / AC5 failure mode 1）

```gherkin
Scenario: 復帰後に repo 状態が変わっている場合は最新版で投入する
  Given session を中断し後で復帰する
  And 復帰時点で repo の状態が中断時点と異なる
  When 復帰後に Agent へ次のメッセージを送る
  Then repo summary / diff は中断時点の古い版ではなく現在の epoch の版で投入される
  And 中断時点の古い repo summary / diff は stale として投入対象から除外される

Scenario: revision が現在 epoch と一致しない context source は stale 扱いになる
  Given ある context source の revision が現在の epoch の版より古い
  When 次のメッセージ送信のための context を構成する
  Then その古い版の context source は投入されない
  And 同一 source の最新版があればそれが投入される
```

### Rule: epoch / revision は会話履歴復帰（#1190）とは独立して判定される（R5 / AC3）

```gherkin
Scenario: 会話履歴は復帰しつつ system context は最新版に差し替える
  Given #1190 の native resume により会話メッセージ履歴が復帰される
  When 復帰後に Agent へメッセージを送る
  Then 会話メッセージ履歴は #1190 の方針どおり復帰される
  And system context（repo / diff / instructions 等）は復帰時点の最新 epoch の版で投入される
  And 会話履歴復帰の成否は system context の epoch 判定を変更しない
```

---

## Feature: context replacement（backend / model / worktree / instruction file 変更）

backend / model / worktree / instruction file の変更を契機に、破棄すべき context を破棄し、再構築すべき context を再構築し、据え置くべき context を据え置く。

### Rule: backend / model 切替で前 backend / 前 model 向け instruction・system prompt が残留しない（R3 / AC5 failure mode 2）

```gherkin
Scenario: backend を切り替えると前 backend 向け instruction が破棄される
  Given session の backend が backend A であり backend A 向けの instruction / system prompt が context に含まれている
  When backend を backend A から backend B に切り替える
  Then 前 backend A 向けの instruction / system prompt は context から破棄される
  And 切替後に投入される instruction / system prompt は backend B 向けのものである

Scenario: model を切り替えると前 model 向け instruction が残留しない
  Given session の model が model X であり model X 向けの instruction が context に含まれている
  When model を model X から model Y に切り替える
  Then 前 model X 向けの instruction は残留しない
```

### Rule: worktree / instruction file の変更で該当 context が再構築される（R3）

```gherkin
Scenario: worktree を切り替えると repo 由来の context が再構築される
  Given session が worktree W1 を対象にしている
  When 対象を worktree W2 に切り替える
  Then repo summary / diff / project instructions は W2 を基準に再構築される
  And W1 を基準とした古い版は破棄される

Scenario: instruction file の変更で instruction context が再構築される
  Given context に AGENTS.md / CLAUDE.md 由来の instruction が含まれている
  When 対象範囲内の AGENTS.md または CLAUDE.md が変更される
  Then instruction context は変更後の内容で再構築される
  And 変更前の instruction は据え置かれない

Scenario: replacement 契機に該当しない context source は据え置かれる
  Given backend / model / worktree / instruction file のいずれも変更されていない
  When 次のメッセージを送る
  Then 直前と同一版の context source はそのまま据え置かれ再構築されない
```

---

## Feature: instruction 解決と重複回避

`AGENTS.md` 相当の探索範囲を定め、複数経路から同一 instruction が重複投入されないようにし、read した file 近傍の局所 instruction を投入する。

### Rule: instruction の探索範囲（R4 / AC2）

```gherkin
Scenario: リポジトリ階層を辿って instruction を収集する
  Given 対象 worktree のディレクトリ階層に複数の AGENTS.md / CLAUDE.md が存在する
  When instruction を解決する
  Then 定義された探索範囲（リポジトリ階層）に含まれる AGENTS.md / CLAUDE.md が収集される
  And 探索範囲外の instruction は収集されない

Scenario: read した file 近傍の局所 instruction を投入する（AC5 failure mode 4）
  Given Agent があるディレクトリ配下の file を read する
  And そのディレクトリ近傍に project instruction が存在する
  When 当該 file に関する context を構成する
  Then その file 近傍の局所 instruction が投入される
```

### Rule: 同一 instruction は重複投入されない（R4 / AC5 failure mode 3）

```gherkin
Scenario: 複数経路から同一 instruction が来ても 1 回だけ投入される
  Given 同一の instruction が次の複数経路から到達しうる
    | 経路                       |
    | リポジトリ instruction          |
    | workflow facet instruction |
    | read file 近傍 instruction    |
  When instruction を解決する
  Then 同一内容の instruction は重複して投入されず 1 回だけ投入される

Scenario: 重複回避により context が肥大化しない
  Given workflow facet instruction とリポジトリ instruction に重複がある
  When context を構成する
  Then 重複分は除外され context に同一 instruction が複数含まれない
```

---

## Feature: ロジック配置（Rust 集約と frontend の責務）

context 構築・instruction 解決・epoch / replacement 判定は Rust 側で行い、frontend は生入力を渡すだけに留める。

### Rule: frontend は生入力を渡すだけで context を構築しない（R6 / AC4）

```gherkin
Scenario: frontend は編集中ファイル・選択範囲・mentions を生のまま渡す
  Given ユーザーが編集中ファイル・選択範囲・mentions を持つ状態で Agent にメッセージを送る
  When frontend が Rust usecase を呼び出す
  Then frontend は context 構築・instruction 解決・epoch 判定を行わない
  And 編集中ファイル・選択範囲・mentions 等の生入力のみが Rust usecase に渡される
  And context source の構成・重複回避・stale 判定の結果は Rust 側で決定される
```

---

## Feature: Rust 実装の正常系・異常系（R7 / AC1）

### Rule: 型と解決ロジックが存在し、正常系・異常系の双方が扱われる

```gherkin
Scenario: 正常系 — context source が保持され epoch / revision で識別できる
  Given 各 context source の取得結果が得られている
  When それらを保持単位に載せる
  Then 各 context source は epoch / revision を伴って保持される
  And 現在 epoch の版を投入対象として取り出せる

Scenario: 異常系 — instruction file の読み取りに失敗しても他 context は維持される
  Given 探索対象の AGENTS.md / CLAUDE.md の 1 つが読み取れない
  When instruction を解決する
  Then 読み取れた instruction は投入され、読み取れない分はスキップされる
  And instruction 解決の失敗が他の context source の保持・投入を巻き込んで破壊しない

Scenario: 異常系 — context source の取得結果が欠落しても epoch 判定が破綻しない
  Given ある context source（例: terminal log summary）の取得結果が存在しない
  When context を構成する
  Then 当該 source は欠落として扱われ、他 source の epoch / revision 判定はそのまま機能する
```

---

## Open Questions

なし。

requirements の Q1（実装範囲）は「Rust 実装まで含む」で確定済み。本書で残る具体化事項（epoch と revision の厳密な版管理アルゴリズム、探索範囲の階層境界、`CONTEXT.md` 等の追加対象、replacement の再構築単位の粒度）は外部観測可能な振る舞いではなく実装設計の詳細であるため、`design.md` の責務として扱い、本書の Open Questions には残さない。
