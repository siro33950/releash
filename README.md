# Releash

**Programmable agentic workflow workbench for software development.**

Releash は、AI agent と協働する開発プロセスを workflow として扱うデスクトップアプリです。

実行中の差分、コメント、terminal output、agent session、承認待ちを同じ workbench で確認し、その場で承認・却下・再指示できます。調査、実装、検証、レビュー、修正、PR comment 対応までを、チャットのやり取りではなく workflow として進めます。

## なぜ Releash か

問題は、agent がコードを書けるかどうかだけではありません。agent が書いた後の判断が、チャット、diff、terminal、PR comment の間に散らばることです。

- 実装方針が妥当か確認する
- 差分を読み、意図と違う変更を指摘する
- 検証結果を見て修正方針を決める
- 複数のレビュー観点を繰り返し適用する
- agent の作業をどこで承認し、どこで止めるか判断する
- PR comment を読み、対応方針を決め、修正後に再確認する

これらを毎回チャットで手作業にすると、判断の流れ、参照した出力、承認した理由、やり直した履歴が散らばります。

Releash は、その一連の開発プロセスを workflow として扱います。実装後に diff review で止める、コメントを agent に返す、test output を見て修正を指示する、承認したら次へ進める。こうした判断と操作を、チャットの外側で一つの流れとして扱えるようにします。

## Releash でできること

### 開発プロセスを再利用できる形で回す

開発プロセスを workflow として定義できます。たとえば、調査、実装、テスト、レビュー、承認、修正という流れを、毎回チャットで組み立て直すのではなく、再利用可能な手順として実行できます。

Releash の workflow は、単なるプロンプト集ではありません。実行中に止まり、人間が確認し、承認したら進め、却下したら指摘内容を持って修正へ戻し、その履歴を残すための実行単位です。

### 実行中の状態を観測する

Workflow の現在地、進行中の agent session、承認待ち、失敗したステップ、agent の出力や検証結果を画面上で確認できます。

どの作業が終わり、どこで止まり、何を根拠に次へ進めるべきかを見失わないようにします。

### 差分やコメントを判断材料として扱う

Diff viewer 上でコメントを書き、agent に渡せます。PR comment や review comment も、単なるメモではなく workflow の判断材料として扱い、必要な修正へ戻せます。

レビュー、修正、再確認の流れを、チャットの外側で追えるようにします。

### 人間の確認と承認を組み込む

人間の承認、却下、再指示を workflow の中に置けます。

Agent が勝手に最後まで進むのではなく、必要な地点で止まります。人間は差分、出力、コメント、実行履歴を見て、承認したら進め、却下したら修正へ戻せます。

### UI と CLI から同じ workflow を扱う

人間は UI から workflow を確認し、承認し、却下し、再指示できます。Agent や automation は CLI から workflow の状態を読み、対応する操作を行えます。

画面上の作業と CLI からの操作が、別々の世界にならないことを目指しています。

## 代表的な workflow

### 実装 workflow

タスクを渡し、Agent に調査、実装、テスト、結果報告まで進めさせる。途中で人間が diff や test output を確認し、必要なら再指示する。

### 複数観点レビュー workflow

複数の観点で差分をレビューし、指摘を集約する。人間は結果を見て、修正すべき点を承認または却下する。

### PR comment 対応 workflow

PR comment を取得し、対応方針を検討し、必要な修正を Agent に実行させる。対応結果を diff と comment の文脈で確認する。

### test / lint / fix loop

test や lint の結果をもとに修正し、再実行する。失敗内容と修正結果を workflow の履歴として残す。

## Built-in Workflows

Releash は、自身の開発に使っている workflow を built-in で同梱しています。

Workflow は、アプリ本体に埋め込まれた固定機能ではなく、開発プロセスそのものです。built-in workflow はその出発点であり、個人やチームのレビュー観点、検証手順、承認ルールに合わせて編集し、再利用していく対象です。

## 機能

| 領域 | 内容 |
|---|---|
| Workflow | workflow 定義、実行、履歴、状態表示、承認、分岐、出力チェック |
| Workflow Panel | 実行中 workflow、timeline、作業詳細、agent conversation、承認状態 |
| Workflow CLI | `list` / `start` / `executions` / `status` / `logs` / `approve` / `abort` / `output` |
| Agent Session | Claude / Codex との session、実行状態、許可要求、streaming 表示 |
| Diff / Review | diff viewer、inline comment、comment 送信、画像 / Markdown の差分確認 |
| Terminal | portable-pty による shell session、bash / zsh / fish の command 完了検出 |
| Git | branch / commit / status / diff / stage / worktree / log、`gh` 経由での PR / Issue 取得 |
| Notification | Slack / Discord 互換 Webhook、許可待ちや workflow 状態の通知 |

## Getting Started

**対応プラットフォーム**: macOS

最新のインストーラは [Releases ページ](https://github.com/siro33950/releash/releases/latest) から取得できます。

任意: [GitHub CLI](https://cli.github.com/) (`gh`) は PR / Issue 取得に使います。

## License

MIT OR Apache-2.0
