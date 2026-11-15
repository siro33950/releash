# Context

- 要求の正本は、ISSUE #2144「Swift.app: Session History を、押す前に中身を見分けられ、誤操作で復元・削除しない形にする」の本文である。
- 見た目の参考は、ISSUE #2144 の本文に貼られた mock 2 枚（「History（行を選択）」「History（削除の確認）」）である。mock の名前、時刻、ブランチ名は例である。
- この文書の Requirements が mock と違う箇所は、Requirements を正とする。違う箇所は、Archive 済みの Workflow の行の時刻（R-009）と、状態の語（R-009）である。
- 調査の時点は main `9e9cc211` である。
- この文書で「シート」とは、Node 面の「…」から「History…」を選ぶと開く、Session History のシートを指す。
- この文書で「Session の名前」とは、Node 面がその Session に出す名前を指す。
- この文書で「Provider の履歴」とは、対象の worktree のディレクトリで provider（Claude、Codex）が自分の記録に残した Session のうち、Releash が Session として持っていないものを指す。
- この文書で「復元」とは、Archive 済みの Session または Workflow を、Node 面に戻すことを指す。
- この文書で「再開」とは、Provider の履歴の Session を、Releash の Session として開くことを指す。

# Outcome

対象者は、Swift.app で Session と Workflow を Archive し、後からシートで復元、再開、削除する開発者である。

今のシートは、Archive 済みの Session を provider の Session ID で出し、名前も時刻も出さない。
Archive 済みの Workflow はタイトルだけを出し、状態も時刻も出さない。
開発者は、どの行が目当ての Session や Workflow かを、押す前に見分けられない。
行は一度押すだけで復元または再開が走り、削除も確認なしに行われる。
開発者は、押し間違えただけで、意図しない復元、再開、削除を起こす。

変更後は、各行に、名前と、種類、provider、状態、時刻が出る。
開発者は、検索と区画の切り替えで行を絞り込める。
行を一度押しても選ばれるだけで、復元と再開は、選んだ行のボタンかダブルクリックで行う。
削除は、確認のアラートで「削除」を押したときだけ行われる。

# Current Behavior

調査は main `9e9cc211` のコードを読み、CI の実行結果を確かめて行った。
アプリの画面を見ることはしていない。

## 入口とシート

- Node 面の「…」から「History…」を選ぶと、シートが開く（`clients/macos/Sources/RootListView.swift:87-89`、`:163`）。
- シートの大きさは 520×420 である（`clients/macos/Sources/SessionHistoryView.swift:60`）。
- 見出しは「History」だけで、対象の worktree のブランチ名は出ない（`SessionHistoryView.swift:22`）。
- 下端に「Close」ボタンが出る（`SessionHistoryView.swift:58`）。

## 一覧

- シートには、「Archived Sessions」「Archived Workflows」「Provider History」の 3 区画が縦に並ぶ（`SessionHistoryView.swift:25`、`:38`、`:43`）。区画の見出しに件数は出ない。
- Archive 済みの Session の行は、`Claude · <provider の Session ID>` の形で出る。名前も時刻も出ない（`SessionHistoryView.swift:28`）。行は Session の識別子の順に並ぶ（`server/src/domain/workspace_tree/visible.rs:74-88`）。
- Archive 済みの Workflow の行は、タイトルだけが出る。状態も時刻も出ない（`SessionHistoryView.swift:40`）。行は Archive の新しい順に並ぶ（`visible.rs:90-113`）。
- Provider の履歴の行は、`Claude · <ラベル>` の形で出る（`SessionHistoryView.swift:45`）。時刻は出ない。
- ラベルは、Session の題、最初の指示、provider の名前と ID の先頭 8 文字（`Claude 1a2b3c4d…` の形）の順に、最初に取れたものである。最初の指示は、空白と改行を 1 つの空白にまとめ、80 文字を超える分を「…」に替える（`server/src/domain/agent_session/provider_history_label.rs:5-42`）。
- Provider の履歴の行は、最終更新の新しい順に並ぶ（`agent_session_history_query_service.rs:148-157`）。
- Provider の履歴は 20 件ずつ出る。続きがあるとき、末尾に「Load more」が出て、押すと 20 件増える（`SessionHistoryView.swift:8`、`:47-51`）。
- 一覧に出る Provider の履歴は、「Load more」を押し続けても、provider ごとに、最終更新の新しい順に 201 件までである（`agent_session_history_query_service.rs:15`、`:41-45`）。
- 検索も絞り込みもない。

## 操作

- どの行も行全体がボタンである。一度押すと、確認なしに復元または再開が行われる（`SessionHistoryView.swift:28-30`、`:40`、`:45`）。
- Archive 済みの Session の行には、その Session が削除できるとき「Delete」が出る。押すと、確認なしに削除が行われる（`SessionHistoryView.swift:32-34`、`:110-119`）。
- 復元または再開が成功すると、シートは閉じる。削除が成功しても、シートは開いたままである。操作が失敗すると、シートに赤い文字で理由が出て、シートは開いたままである（`SessionHistoryView.swift:23`、`:70-82`、`:92`、`:106`）。

## Archive 済みの Workflow の状態

- Workflow の状態は、実行中、完了、中止の 3 つである（`server/src/domain/workflow/value_objects/execution.rs:5-9`）。
- 実行中の Workflow を Archive すると、その Workflow の状態は中止になる（`execution_archive.rs:92-118`）。Archive 済みの Workflow の状態は、完了か中止である。

## 再現手順

アプリの画面での再現はしていない。
代わりに、main への push の CI（run `43695137276`、commit `c1737fa8`）の `macos-checks / test-integration` で、シートを確かめるテスト（`clients/macos/Tests/Integration/SessionHistoryViewTests.swift`）が実行され、成功したことを確かめた。
commit `c1737fa8` から調査の時点までに変わったファイルは、版番号を持つ 3 つ（`Cargo.toml`、`Cargo.lock`、`clients/macos/project.yml`）だけである。

このテストは、次の手順でシートを開き、出力を確かめる。

1. Archive 済みの Session（provider は Claude、provider の Session ID は `archived-provider`、削除できる）を 1 件、Provider の履歴（provider は Codex、ラベルは `History session`、続きがある）を 1 件用意して、シートを開く。
2. 「Load more」を押す。
3. 行 `Claude · archived-provider` を一度押す。
4. 手順 1、2 の後に、「Delete」を一度押す。
5. 手順 1、2 の後に、行 `Codex · History session` を一度押す。
6. Archive 済みの Workflow（タイトルは `Archived workflow`）を 1 件用意してシートを開き、行 `Archived workflow` を一度押す。

実際の出力は次のとおりである。

| 手順 | 出力 |
| --- | --- |
| 1 | 行 `Claude · archived-provider` と「Delete」、行 `Codex · History session` と「Load more」が出る |
| 2 | Provider の履歴が 40 件分で読み直される。行 `Codex · History session` は出たままである |
| 3 | 確認なしに、その Session の復元がサーバへ送られる |
| 4 | 確認なしに、その Session の削除がサーバへ送られる |
| 5 | 確認なしに、その Session の再開がサーバへ送られる |
| 6 | 行 `Archived workflow` が出る。押すと、確認なしに、その Workflow の復元がサーバへ送られる |

# Scope / Non-goals

変更するもの:

- シートの大きさ、見出し、下端。
- 検索欄と、区画の切り替え。新しく置く。
- 一覧の区画、行の並び、各行に出す情報。
- 行の操作。行の選択、選んだ行のボタン、ダブルクリック、削除の確認。

Non-goals:

- History を開く入口。今の Node 面の「…」のままにする。
- Archive 済みの Session を、最初の指示で検索すること。
- 「PROVIDER の履歴」の区画の見出しに、件数を出すこと。

# Requirements

- R-001: シートの大きさは、新しい Worktree のダイアログ、設定の画面と同じ 1024×800 である。
- R-002: シートの見出し「History」の横に、対象の worktree のブランチ名が出る。
- R-003: シートの下端に、「閉じる」ボタンと、案内の文「行を選ぶと操作が出ます。ダブルクリックで復元・再開します。」が出る。開発者が「閉じる」を押すと、シートは閉じる。
- R-004: シートの上部に、検索欄と、「すべて」「Archive 済み」「Provider の履歴」の切り替えが出る。検索欄は「名前・最初の指示で検索」と案内する。シートを開いた時点で、検索欄は空であり、切り替えは「すべて」である。
- R-005: 一覧は、「ARCHIVE 済み」と「PROVIDER の履歴」の 2 区画である。「ARCHIVE 済み」の見出しには、その区画に出ている行の数が出る。「PROVIDER の履歴」の見出しには、件数は出ず、「このディレクトリで provider が残した Session」と出る。
- R-006: 「ARCHIVE 済み」の区画には、Archive 済みの Session と Archive 済みの Workflow が、一つの並びで、時刻の新しい順に出る。並べる時刻は、Session では Archive した時刻、Workflow では更新時刻である。
- R-007: 「PROVIDER の履歴」の区画には、Provider の履歴が、最終更新の時刻の新しい順に出る。続きがあるとき、区画の末尾に「さらに読み込む」が出る。開発者が「さらに読み込む」を押すと、続きの行が加わる。
- R-008: Archive 済みの Session の行は 2 段で出る。1 段目は Session の名前である。2 段目は、「Session」、provider の名前、Archive した時刻であり、「Session · Claude · 2 時間前に Archive」の形である。
- R-009: Archive 済みの Workflow の行は 2 段で出る。1 段目は Workflow のタイトルである。2 段目は、「Workflow」、状態、更新時刻であり、「Workflow · ● 完了 · 昨日 18:42」の形である。状態は色の点と文字で出る。完了は緑の点と「完了」、中止は赤の点と「中止」である。
- R-010: Provider の履歴の行は 2 段で出る。1 段目はラベルである。ラベルは、Session の題、最初の指示、provider の名前と ID の先頭 8 文字の順に、最初に取れたものである。2 段目は、provider の名前、最終更新の時刻であり、「Claude · 3 時間前」の形である。
- R-011: 行に出る時刻の表記は、開発者がシートを見ている時点の暦の日で切り替わる。今日の時刻は、1 分未満なら「たった今」、1 時間未満なら「25 分前」の形、それ以上なら「2 時間前」の形である。昨日の時刻は「昨日 18:42」の形である。それより前で今年の時刻は「11月12日」の形である。去年以前の時刻は「2025年11月12日」の形である。
- R-012: 開発者が検索欄に語を入れると、一覧は、その語を含む行だけになる。検索の対象は、Archive 済みの Session では Session の名前、Archive 済みの Workflow ではタイトル、Provider の履歴では Session の題と最初の指示である。検索に使う題と最初の指示は、行のラベルを決めるときに取れたものである。大文字と小文字は区別しない。語の前後の空白は無視する。「さらに読み込む」でまだ出していない Provider の履歴も、検索の対象である。検索の対象になる Provider の履歴は、検索欄が空のときに「さらに読み込む」を押し続ければ出る履歴までである。開発者が検索欄を空にすると、一覧は絞り込む前の行に戻る。
- R-013: 開発者が切り替えで「Archive 済み」を選ぶと、「ARCHIVE 済み」の区画だけが出る。「Provider の履歴」を選ぶと、「PROVIDER の履歴」の区画だけが出る。「すべて」を選ぶと、2 区画が出る。検索欄の語は、切り替えを変えても効いたままである。
- R-014: 開発者が行を一度押すと、その行が選ばれる。復元、再開、削除は行われない。操作のボタンは、選ばれた行にだけ出る。Archive 済みの Session の行には「復元」と削除のボタン、Archive 済みの Workflow の行には「復元」、Provider の履歴の行には「再開」が出る。
- R-015: 開発者が、選んだ行の「復元」を押すか、Archive 済みの Session または Workflow の行をダブルクリックすると、その Session または Workflow が復元される。復元されたものは Node 面に出て、「ARCHIVE 済み」の区画から消える。
- R-016: 開発者が、選んだ行の「再開」を押すか、Provider の履歴の行をダブルクリックすると、その Session が再開される。再開された Session は Session タブで開く。
- R-017: 開発者が削除のボタンを押すと、削除は行われず、確認のアラートが出る。アラートには、「「<Session の名前>」を削除しますか？」、「削除した Session は History から消え、復元できません。」、「削除」ボタン、「キャンセル」ボタンが出る。
- R-018: 開発者がアラートの「削除」を押すと、その Session が削除され、「ARCHIVE 済み」の区画から消える。「キャンセル」を押すと、削除は行われず、アラートが閉じ、その Session の行は残る。

# Assumptions

なし

# Invariants

なし

# Risks

- Provider の履歴の検索は、「さらに読み込む」でまだ出していない Session の題と最初の指示も対象にする。Provider の履歴が多い worktree で、検索の結果が開発者の入力に遅れずに出るかは、作って測るまで確かめられない。想定が外れると、R-012 の「まだ出していない Provider の履歴も、検索の対象である」を、対象の件数を狭めずに満たせない。
