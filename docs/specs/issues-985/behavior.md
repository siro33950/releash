# Behavior — issues-985

Issue: #985 「`git_host`（Git hosting integration）を clean architecture 配置へ移行する」

本書は `requirements.md` の要求 R1〜R9 を、実装詳細を含まない**外部から観測可能な振る舞い**として Gherkin で定義する。本 ISSUE は内部アーキテクチャのレイヤー移行であり、移行前後で external observable behavior は不変である（R8）。したがって本書が規定するのは「移行後も観測できなければならない振る舞い」であり、レイヤー分割・型/関数の配置先・module 命名・依存方向の遵守といった内部経路（R1〜R6 の構造面）は対象外であり design / 実装で扱う。

ここで「観測可能」とは、5 本の Tauri command（`check_pr_provider_status` / `fetch_pr_status` / `get_cached_pr_status` / `fetch_issues` / `get_cached_issues`）を frontend が `invoke` したときの、command 名・引数・戻り値 DTO・provider 判定結果・cache の TTL 挙動を指す。

## 仮定

requirements の仮定 A1〜A4 を所与とし、加えて本書作成時に置いた仮定を示す。design で確定すべき内部詳細は振る舞いに持ち込まない。

- **A1（requirements A1）**: 本 ISSUE はレイヤー移行であり、provider 判定ロジック・`gh` 引数・cache TTL（30 秒）・status の語彙は変更しない。型・関数の配置先のみが変わる。
- **A2（requirements A2）**: cache（`PrCache` / `IssueCache`）は引き続き Tauri `manage()` の state として保持し、in-memory + Mutex の保持方式自体は維持する。
- **A3**: 本書では cache TTL の境界を「fetch からの経過時間が 30 秒**未満**なら hit、30 秒**以上**なら stale」として扱う（現実装 `elapsed() < TTL` に準拠）。
- **A4**: `gh` 実行が失敗・タイムアウト・非ゼロ終了・出力 parse 不能のとき、PR / issue の fetch は空の結果（空 map / 空 vec）を返し、command 自体は `Ok` を返す（現実装の fallback に準拠）。エラーを呼び出し側へ伝播しない。
- **A5**: `check_pr_provider_status` が `gh` CLI を起動する経路（`--version` / `auth status`）は環境依存のため、本書では status 値の語彙と判定順序のみを規定し、実 CLI の存在は前提にしない。
- **A6（requirements A4 / R9）**: 削除対象の dead-code（review comment 関連型・`get_pr_review_comments`・`parse_pr_review_comments`・時刻変換ヘルパ `parse_rfc3339_to_millis` / `days_from_civil`）は production 経路から呼ばれていないため、その削除は 5 本の command の結果に影響しない。

---

```gherkin
Feature: git_host のレイヤー移行後も Git hosting integration の振る舞いを不変に保つ

  Releash の Git hosting integration（provider 判定 / PR status 取得 / issue 取得 / cache）を
  clean architecture の 4 レイヤーへ移行する。移行は内部構造の再配置であり、
  frontend から invoke する 5 本の Tauri command の観測可能な振る舞い
  （command 名・引数・戻り値 DTO・provider 判定結果・cache TTL 挙動）は移行前後で変わらない。

  Background:
    Given デスクトップアプリのバックエンドが起動している
    And PR cache と issue cache が Tauri state として保持されている
    And 各 command は repo_path を引数に取り frontend から invoke できる

  # --- R2 / R8: provider 判定（check_pr_provider_status） ---

  Rule: provider status は remote URL とホスト種別から判定する

    Scenario: remote(origin) が無いリポジトリは NoRemote を返す
      Given repo_path のリポジトリに origin remote が設定されていない
      When check_pr_provider_status を invoke する
      Then 戻り値の status は "no_remote" である

    Scenario: GitHub 以外のホストは UnsupportedPlatform を返す
      Given origin remote の URL が github.com を含まない（例: gitlab.com）
      When check_pr_provider_status を invoke する
      Then 戻り値の status は "unsupported_platform" である

    Scenario: GitHub で gh CLI が見つからない場合は CliNotFound を返す
      Given origin remote の URL が github.com を含む
      And gh CLI が利用できない
      When check_pr_provider_status を invoke する
      Then 戻り値の status は "cli_not_found" であり cli は "gh" である

    Scenario: GitHub で gh が未認証の場合は NotAuthenticated を返す
      Given origin remote の URL が github.com を含む
      And gh CLI は存在するが認証されていない
      When check_pr_provider_status を invoke する
      Then 戻り値の status は "not_authenticated" である

    Scenario: GitHub で gh が認証済みの場合は Available を返す
      Given origin remote の URL が github.com を含む
      And gh CLI が存在し認証済みである
      When check_pr_provider_status を invoke する
      Then 戻り値の status は "available" である

  # --- R2 / R8: provider 不在時の fetch ---

  Rule: provider を解決できない場合 PR / issue は空を返す

    Scenario: GitHub 以外のリポジトリの PR status は空である
      Given origin remote が GitHub ではない、または remote が無い
      When fetch_pr_status を invoke する
      Then 戻り値の open_prs は空であり merged_branches は空である
      And command は Ok を返す

    Scenario: GitHub 以外のリポジトリの issue 一覧は空である
      Given origin remote が GitHub ではない、または remote が無い
      When fetch_issues を invoke する
      Then 戻り値の issue 一覧は空である
      And command は Ok を返す

  # --- R8: GitHub provider 経由の PR status 取得（fetch_pr_status） ---

  Rule: GitHub リポジトリでは gh 出力から open PR と merged branch を構成する

    Scenario: open PR を branch 名をキーとして返す
      Given GitHub provider が利用可能である
      And gh が open PR の一覧（headRefName / number / url）を返す
      When fetch_pr_status を invoke する
      Then open_prs は branch 名をキーに number と url を持つ
      And merged_branches は merged PR の headRefName の一覧である

    Scenario: gh 実行が失敗・タイムアウトしても空で返り Ok になる
      Given GitHub provider が利用可能である
      And gh 実行が失敗・タイムアウト・非ゼロ終了する
      When fetch_pr_status を invoke する
      Then open_prs は空であり merged_branches は空である
      And command はエラーを伝播せず Ok を返す

  # --- R8: GitHub provider 経由の issue 取得（fetch_issues） ---

  Rule: GitHub リポジトリでは gh 出力から open issue 一覧を構成する

    Scenario: open issue を number / title / state / url / author 等とともに返す
      Given GitHub provider が利用可能である
      And gh が open issue の JSON 一覧を返す
      When fetch_issues を invoke する
      Then 各 issue は number・title・state・url・author・created_at・updated_at を持つ
      And labels・assignees・body・milestone は欠落時に既定値（空 / 空文字 / null）になる

    Scenario: gh 出力が parse できない場合は空一覧で Ok になる
      Given GitHub provider が利用可能である
      And gh 出力が不正な JSON である、または gh 実行が失敗する
      When fetch_issues を invoke する
      Then 戻り値の issue 一覧は空である
      And command はエラーを伝播せず Ok を返す

  # --- R3 / R8: cache の TTL 挙動（get_cached_pr_status / get_cached_issues） ---

  Rule: cached 系 command は TTL 30 秒で hit / miss / stale を判定する

    Scenario: 初回呼び出しは cache miss として fetch し結果を保存する
      Given 対象 repo_path の cache entry が存在しない
      When get_cached_pr_status を invoke する
      Then provider から取得した PR status が返る
      And その結果が repo_path をキーに cache に保存される

    Scenario: 30 秒未満の再呼び出しは cache hit で同じ結果を返す
      Given get_cached_pr_status を呼び済みで cache entry が存在する
      And 前回 fetch からの経過時間が 30 秒未満である
      When 同じ repo_path で get_cached_pr_status を再度 invoke する
      Then provider を再 fetch せず cache 済みの値が返る

    Scenario: 30 秒以上経過した entry は stale として再 fetch する
      Given get_cached_pr_status を呼び済みで cache entry が存在する
      And 前回 fetch からの経過時間が 30 秒以上である
      When 同じ repo_path で get_cached_pr_status を再度 invoke する
      Then provider から再 fetch され新しい値が返る
      And cache entry が更新される

    Scenario: issue cache も同一の TTL 30 秒で hit / miss / stale を判定する
      Given get_cached_issues を対象 repo_path で呼び出す
      Then 初回は miss として fetch し保存する
      And 30 秒未満の再呼び出しは cache 済みの一覧を返す
      And 30 秒以上経過後の再呼び出しは再 fetch する

  # --- R4 / R8: command の入出力契約の不変性 ---

  Rule: 5 本の command 名・引数・戻り値 DTO は移行前後で不変である

    Scenario: frontend は同じ command 名と引数で invoke できる
      When frontend が check_pr_provider_status / fetch_pr_status / get_cached_pr_status / fetch_issues / get_cached_issues を invoke する
      Then いずれも repo_path を引数に取り、移行前と同じ DTO 形状で結果を返す
      And frontend 側の invoke 先 command 名と DTO 形状の変更は不要である

  # --- R9: dead-code 削除が観測可能な振る舞いに影響しない ---

  Rule: review comment 関連と時刻変換ヘルパの削除は command の結果を変えない

    Scenario: dead-code 削除後も 5 本の command の結果が変わらない
      Given review comment 関連型・get_pr_review_comments・parse_pr_review_comments・時刻変換ヘルパが削除されている
      When 5 本の command をそれぞれ invoke する
      Then provider 判定・PR status・issue 一覧・cache TTL 挙動は削除前と同じである
      And review comment を取得・公開する command や戻り値は新たに追加されない
```

---

## 主要観点の対応

- **正常系**: provider Available 判定、open PR / merged branch の構成、open issue 一覧の構成、cache miss → fetch → 保存、cache hit。
- **異常系**: remote 不在（NoRemote）、非 GitHub（UnsupportedPlatform）、gh 不在（CliNotFound）、未認証（NotAuthenticated）、gh 実行失敗 / タイムアウト / parse 不能時の空結果 + Ok（エラー非伝播）。
- **境界条件**: cache TTL 30 秒の境界（30 秒未満は hit、30 秒以上は stale → 再 fetch）、欠落フィールドの既定値（labels / assignees / body / milestone）。
- **不変性**: command 名・引数・DTO 形状の不変（R4）、dead-code 削除が command 結果に無影響（R9）。

## Open Questions

なし。requirements の仮定 A1〜A4 と Open Questions「なし」により、振る舞いレベルの未確定事項はない。レイヤー内の module 構成・命名・ファイル分割・cache value object 化の有無といった内部配置は design で確定する事項であり、観測可能な振る舞いには影響しない。
