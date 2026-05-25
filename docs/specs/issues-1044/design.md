# Design

## Behavior Coverage

- AgentChat exposes exactly three canonical permission modes
  - AgentChat の外部契約で扱う正規の権限モードを `Ask` / `Edit` / `Full` の 3 種類に限定する。
  - 表示、保存、復元、外部入力、セッション開始、実行中の切り替えは、同じ正規化済みモードを参照する。
  - 新規に保存または送信される権限モード値は、常に `Ask` / `Edit` / `Full` のいずれかにする。

- Each canonical permission mode has a distinct user-visible meaning
  - `Ask` は、安全側の権限で開始し、追加の操作が必要な場合にユーザー確認へ進めるモードとして扱う。
  - `Edit` は、ワークスペース内編集を許可し、それを超える操作や確認対象の操作ではユーザー確認へ進めるモードとして扱う。
  - `Full` は、広い操作権限を承認なしで許可するモードとして扱う。

- Claude sessions receive the intended permission meaning
  - AgentChat の正規モードを Claude 側の権限意味へ変換する境界を 1 つに集約する。
  - `Ask` は確認可能な既定権限、`Edit` は編集許可、`Full` は広い権限として Claude セッションに渡す。

- Codex sessions preserve confirmation behavior for safer modes
  - AgentChat の正規モードを Codex 側の実行権限へ変換する境界を 1 つに集約する。
  - `Ask` と `Edit` は、必要な操作でユーザー承認を要求できる実行権限として Codex セッションに渡す。
  - `Full` は、承認なしで広い操作を許可する実行権限として Codex セッションに渡す。

- Runtime permission changes take effect according to the same model
  - 実行中の権限切り替えも、セッション開始時と同じ正規化とバックエンド別変換を通る。
  - 切り替え後の後続操作だけが新しい権限モードに従い、古い表示値や保存値の意味を引き継がない。

- Desktop, remote, workflow, and restored sessions share one permission meaning
  - デスクトップ UI、リモート UI、ワークフロー、保存済みセッションは、同じ AgentChat 権限モード契約を共有する。
  - 各呼び出し元は独自の権限意味を持たず、正規モードを指定または表示するだけに留める。

- Legacy readonly is not a new canonical permission mode
  - `readonly` は正規モードではなく、既存データ互換のためのレガシー入力としてのみ扱う。
  - 既存データや既存ワークフローから `readonly` を読んだ場合は `Ask` として扱い、結果として `Ask` より広い権限を与えない。
  - `readonly` は新規 UI 表示、新規保存値、新規送信値には使わない。

- Invalid permission values do not silently widen permission
  - `Ask` / `Edit` / `Full` / 対応するレガシー値以外は、通常の権限モードとして受理しない。
  - 不正値によって暗黙に `Edit` や `Full` 相当へ広がらないようにし、呼び出し元が不受理を観測できる契約にする。

## Key Decisions

- 正規モード名は `Ask` / `Edit` / `Full` に統一する。
  - 理由: UI、保存値、外部入力、エージェント実行権限の間で同じ語彙を使い、ユーザーが選んだ意味と実際の権限を一致させるため。
  - 採らなかった代替案: `readonly` / `edit` / `full` を維持する案は、Claude の既定権限や Codex の承認挙動と表示上の意味がずれるため採用しない。

- `readonly` は `Ask` へ互換的に読み替える。
  - 理由: 既存データや既存ワークフローを壊さず、かつ `Ask` より広い権限を与えないため。
  - 採らなかった代替案: `readonly` を移行エラーにする案は、安全ではあるが既存ユーザーの保存済みセッションやワークフローを不必要に停止させる可能性が高いため採用しない。

- Codex の `Ask` と `Edit` はどちらも承認要求可能なモードにする。
  - 理由: UI で安全側に見える権限が、実行時に承認なし相当として動く不一致を解消するため。
  - 採らなかった代替案: `Edit` を承認なしのワークスペース編集モードとして扱う案は、確認が必要な操作でユーザー確認が出ない問題を残すため採用しない。

- 正規化とバックエンド別変換の責務はフロントエンドではなく AgentChat のバックエンド境界に置く。
  - 理由: デスクトップ UI、リモート UI、ワークフロー、保存復元が別々に意味を持つと、同じ値でも実行権限が分岐するため。
  - 採らなかった代替案: 呼び出し元ごとに変換を持たせる案は、デスクトップとリモート、保存値と外部入力の不一致を生みやすいため採用しない。

## Responsibility Boundaries

- AgentChat バックエンド
  - 正規モードの受理、レガシー値の互換処理、不正値の不受理を担当する。
  - Claude / Codex それぞれの実行権限への変換を担当する。
  - セッション開始時と実行中切り替え時に、同じ権限モデルを適用する。

- デスクトップ UI
  - `Ask` / `Edit` / `Full` の選択肢を表示し、ユーザー入力を AgentChat へ渡す。
  - 独自に Claude / Codex の実行権限へ変換しない。
  - `readonly` を新規選択肢として表示しない。

- リモート UI
  - デスクトップ UI と同じ正規モードを表示し、選択値を AgentChat へ渡す。
  - リモート専用の権限意味を持たない。

- ワークフロー / 外部入力
  - AgentChat 権限モードとして `Ask` / `Edit` / `Full` を指定する。
  - 既存互換として `readonly` が入力される場合は、AgentChat 側の互換処理に委ねる。

- 保存 / 復元
  - 新規保存時は正規モードのみを記録する。
  - 復元時は保存値を AgentChat の権限モード契約として評価し、復元後の意味を保存時から変えない。

- Agent バックエンド
  - AgentChat から渡されたバックエンド固有の権限意味に従って動作する。
  - AgentChat の正規モード名やレガシー値互換を独自に解釈しない。

## Contracts

- AgentChat permission mode contract
  - 正規値: `Ask` / `Edit` / `Full`
  - レガシー入力値: `readonly`
  - `readonly` の扱い: 入力または復元時に `Ask` と同等に扱う
  - 新規出力値: `Ask` / `Edit` / `Full` のみ
  - 不正値: 通常の権限モードとして受理せず、呼び出し元が不受理を観測できる

- User-visible permission meaning
  - `Ask`: 安全側の権限で開始し、必要に応じてユーザー確認へ進める
  - `Edit`: ワークスペース内編集を許可し、必要に応じてユーザー確認へ進める
  - `Full`: 広い操作を承認なしで許可する

- Claude session permission contract
  - `Ask`: 確認可能な既定権限として渡す
  - `Edit`: 編集許可として渡す
  - `Full`: 広い権限として渡す

- Codex session permission contract
  - `Ask`: 読み取り中心で、追加操作にユーザー承認を要求できる権限として渡す
  - `Edit`: ワークスペース内編集を許可し、追加操作にユーザー承認を要求できる権限として渡す
  - `Full`: 広い操作を承認なしで許可する権限として渡す

- Runtime permission change contract
  - 入力値は AgentChat permission mode contract に従う。
  - 受理された切り替えは後続の Agent 操作に適用される。
  - 不受理の切り替えは、暗黙に広い権限へ置き換えない。

- Persistence / message contract
  - 新規保存値と新規送信値は `Ask` / `Edit` / `Full` のみ。
  - 復元値または受信値に `readonly` がある場合は `Ask` として扱う。
  - 復元値または受信値が不正な場合は、通常の権限モードとして扱わない。

## Data / Communication Flow

1. デスクトップ UI、リモート UI、ワークフロー、保存復元のいずれかが AgentChat 権限モードを渡す。
2. AgentChat バックエンドが入力値を正規モードへ評価する。
3. `readonly` は `Ask` として扱われ、不正値は通常の権限モードとして受理されない。
4. セッション開始または実行中切り替えの対象 Agent が Claude か Codex かに応じて、正規モードをバックエンド固有の権限意味へ変換する。
5. Agent セッションは変換後の権限意味に従って後続操作を実行する。
6. セッション状態を保存または送信する場合は、正規モードのみを書き出す。

## State Ownership

- AgentChat セッション状態
  - 現在の正規権限モードの owner。
  - セッション開始時、復元時、実行中切り替え時に更新される。

- AgentChat バックエンド
  - 正規モードへの評価結果と、Agent バックエンドへ渡す権限意味の owner。
  - レガシー入力と不正入力の扱いを一元的に決定する。

- UI
  - 表示中の選択状態とユーザー入力の owner。
  - 永続化された権限意味やバックエンド実行権限の owner ではない。

- 保存データ / 外部メッセージ
  - 正規モード値を運ぶ媒体。
  - 権限意味の解釈やバックエンド別変換の owner ではない。

## Boundaries

- フロントエンドは、権限モードをバックエンド固有の実行権限へ変換しない。
- デスクトップ UI とリモート UI は、同じ正規モードに異なる意味を与えない。
- Agent バックエンドは、`readonly` 互換や不正値フォールバックを独自に決めない。
- `readonly` は新しい正規モードとして表示、保存、送信しない。
- 不正な権限モード値を、暗黙に `Edit` や `Full` 相当として扱わない。
- `Ask` または `Edit` の Codex セッションで、確認対象の操作が承認なし相当として実行できる設計にしない。
- `Full` の意味を、確認要求可能な安全側モードとして扱わない。
- この設計では、OS や Git 操作そのものの権限制御モデルを新設しない。

## Implementation Freedom

- UI 上の補足説明文や日本語併記の有無は、正規値と意味が変わらない範囲で実装に委ねる。
- 権限モードを保持する内部表現や変換処理の分割は、外部契約を満たす範囲で実装に委ねる。
- `readonly` を読み替えたことをユーザーまたはログにどう示すかは、既存の通知・エラー表示方針に合わせて実装に委ねる。
- 不正値を呼び出し元へ返す表現は、既存の command、message、workflow のエラー契約に合わせて実装に委ねる。
- 既存セッションへ実行中切り替えを伝える内部通信方法は、後続操作に新しい権限が反映される限り実装に委ねる。
- Claude / Codex の具体的な起動オプションや内部パラメータ名は、ここでは固定しない。
