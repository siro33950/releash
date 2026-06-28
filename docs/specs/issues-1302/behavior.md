# Behavior

本書は #1302（branch / worktree path / shell escaping 導出の Rust-owned 化）の観測可能な振る舞いを Gherkin で定義する。本 ISSUE はリファクタリング / アーキテクチャ移行であり、導出規則の所有を frontend から Rust へ移す。利用者から観測できる結果（生成される branch 名、作成される worktree path、terminal へ入力される文字列、path 突き合わせの結果）は従来と不変に保つことが中心的な振る舞いである。実装経路（command 形・read model の shape）は behavior の対象外とし design.md で確定する。

```gherkin
Feature: branch / worktree path / shell escaping 導出の Rust 所有化

  リファクタリングとして、branch 名・worktree path・shell escaping・path 正規化の
  導出規則を Rust が所有する。利用者から観測できる振る舞いは従来と等価に保つ。

  Background:
    Given Releash の worktree 作成 UI と terminal が利用できる
    And worktree / branch の一覧は backend の read model から供給される

  Rule: issue から作成する branch 名は従来規則と等価である

    Scenario: issue を選択すると default branch 候補が表示される
      Given issue number が "1302" の issue がある
      When 利用者が worktree 作成 UI でその issue を選択する
      Then default branch 候補として "feat/issues/1302" が表示される

    Scenario: 既存 worktree の branch は選択候補から除外される
      Given issue number "1302" に対応する branch "feat/issues/1302" の worktree が既に存在する
      When 利用者がその issue を選択する
      Then 既存 branch に一致する候補は作成対象として除外される

  Rule: worktree directory / path は従来規則と等価に導出される

    Scenario: branch から worktree path が導出される
      Given repository が "/path/to/myrepo" にある
      And 作成対象 branch が "feat/issues/1302" である
      When 利用者が worktree 作成を実行する
      Then worktree は repo の親ディレクトリ配下 "myrepo-worktrees" 内に作成される
      And branch 名の "/" を "-" に置換した directory 名 "feat-issues-1302" が用いられる

    Scenario: branch 名のスラッシュが directory 名へ正しく変換される
      Given 作成対象 branch が "feat/issues/1302" である
      When worktree path が導出される
      Then directory 名は "feat-issues-1302" になる

  Rule: terminal への file drop は従来と等価な文字列を入力する

    Scenario: 通常の path を drop する
      Given terminal が開いている
      When 利用者が path "/path/to/file.txt" を terminal へ drop する
      Then terminal へ入力される文字列の内容（quote の有無・形）は従来と等価である

    Scenario: shell metacharacter を含む path を drop する
      Given terminal が開いている
      When 利用者が空白や記号を含む path を terminal へ drop する
      Then その path は単一引用符で escaping された文字列として terminal へ入力される

    Scenario: 複数 path を同時に drop する
      Given terminal が開いている
      When 利用者が複数の path を同時に terminal へ drop する
      Then 各 path は escaping され、空白で結合された文字列として terminal へ入力される

  Rule: backend が返す path は正規化済みで、突き合わせ結果は不変である

    Scenario: ファイルを開く際の path 突き合わせ
      Given backend が worktree_id / worktree_path を含む event / read model を返す
      When frontend がファイルを開く・worktree を切り替える・worktree 一覧を表示する
      Then path 比較（worktree_id / worktree_path のマッチング）の結果は従来と不変である
      And frontend 側で追加の path 正規化を行わない

  Rule: Notion property からの branch 名規則は frontend に存在しない

    Scenario: Notion branch 導出は frontend UI に露出しない
      Given Notion property から branch 名を導出する規則がある
      When 利用者が worktree 作成 UI を操作する
      Then frontend に Notion branch 導出の UI は存在しない
      And 当該規則（sanitize・pageId fallback・notion-task fallback・prefix）は backend が所有する
```

## 仮定

- 移行先の Rust command / read model の具体的 shape（既存 `create_worktree` 拡張か新規 query command 新設か等）は design.md で確定する。本書は「導出を Rust が所有し frontend は表示・利用のみ」という観測性質のみを扱う。
- `generateNotionBranchName` は現状 production 未配線（テストのみ）であり、frontend に新規 Notion branch UI は作らない。規則は #986 が確定する Notion branch derivation 境界（Rust）に属する domain rule として Rust test 付きで担保する。
- branch 名規則 `feat/issues/<number>`、worktree directory `<repoName>-worktrees`、branch の `/`→`-` 置換、`${dir}/${name}` 結合、既存 branch 重複回避の観測結果は現行と等価に保つ。
- file drop 時に terminal へ入力される文字列の内容（quote の有無・形）は現行 `quotePathForShell` と等価に保つ。shell escaping は POSIX shell（single-quote）前提とし、対象シェル差異（Windows 等）は現行と同等に保ち拡張しない。
- ⑥ の canonical 化は forward-slash 正規化（現行 `normalizePath` と等価）を backend の path 出力に保証することを指し、path の指す対象は変えない。どの emit / read model で正規化を保証するかは design.md で確定する。
- 本 ISSUE はリファクタリングであり、`CreateWorktreeModal` の機能・レイアウト、`create_worktree` の作成手順、terminal / PTY 入出力経路そのものは変更しない。

## Open Questions

なし。
