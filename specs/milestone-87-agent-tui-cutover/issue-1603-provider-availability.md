# Issue #1603 Provider CLI availability implementation specification

## 1. 目的

Releashが対応するProvider CLIについて、アプリ初期化時の自動検出、Settingsでの状態確認、実行ファイル設定、再判定、Standalone AgentSession選択、Workflow検証、実際のProvider process起動を、一つのRust backend-ownedなProvider registryへ統合する。

利用可能状態をfrontend、AgentSession起動処理、Workflow runtimeが個別に判定しない。GUIアプリと通常のTerminalの`PATH`差異、および非標準パスへ配置されたCLIからユーザーが復旧できることを保証する。

## 2. 正本と優先順位

本Specは次の正本を実装可能な作業単位へ落としたものである。

1. GitHub Issue #1603本文
2. GitHub Issue #1603の「Orca調査を踏まえた外部仕様の補足」コメント
3. `specs/milestone-87-agent-tui-cutover/acceptance-contract.md`のATUI-025
4. GitHub Milestone 87
5. repository rootの`AGENTS.md`
6. `src-tauri/AGENTS.md`
7. `.claude/rules/rust-first-logic.md`
8. `docs/architecture/README.md`
9. `docs/architecture/DOMAIN.md`
10. `docs/architecture/USECASE.md`
11. `docs/architecture/GATEWAY.md`
12. `docs/architecture/INFRASTRUCTURE.md`
13. `docs/architecture/CONTROLLER.md`
14. `docs/architecture/TEST.md`
15. 本Spec

上位の正本と既存実装が矛盾する場合は既存実装を正本へ合わせる。推測によるProvider、設定項目、判定条件の追加は行わない。

## 3. 現在の問題

現行実装には次の不一致がある。

- `LocalProviderAvailabilityGateway`は保持した`PATH`に対して呼び出しごとに実行可能性を調べるだけで、初期化済みProvider registryを持たない。
- SettingsからProviderごとの利用可否、解決済み実行ファイル、利用不可理由を取得できない。
- Settingsから`agents.claude.cli_path`と`agents.codex.cli_path`を変更または初期値へ戻すproduction経路がない。
- availability判定用Gatewayとprocess起動用Gatewayが、Config由来の実行文字列を別々にコピーして保持するため、設定更新後に同じstate authorityを参照できない。
- 利用不可は`bool`へ潰され、未検出、実行権限不足、探索不能を区別できない。
- `list_available_provider_agent_session_providers`は選択候補だけを返し、Settingsが必要とする全Providerの状態を返さない。
- ATUI-025はacceptance contractに存在するが、初期化、設定更新、再判定、Standalone選択、Workflow検証、process起動をproduction境界で一続きに検証するtestがない。
- frontendの旧Agent設定はProvider availabilityとは別のlocalStorage stateを持ち、#1603の正本として使用できない。

## 4. 外部仕様

### 4.1 自動検出

- アプリ初期化時に、Releashが対応する全Provider CLIを自動判定する。
- 初期判定はConfig読込後、Provider選択・Workflow実行・Provider process起動を受け付ける前に完了する。
- 通常のTerminalから実行できるCLIは、GUI起動時の疎な`PATH`だけを理由に未検出としてはならない。
- macOS / Linuxでは既存のlogin-shell `PATH`補正を使用し、補正後のprocess環境をProvider判定とprocess起動で共有する。
- Providerの既定実行コマンドはbackendの対応Provider catalogが所有し、frontendに固定Provider分岐を置かない。

### 4.2 Settings表示

- Settingsは対応する全Providerを、利用可能・利用不可にかかわらず表示する。
- 各Providerについて、Provider ID、表示名、既定実行コマンド、設定中の上書き、判定に使用した実効コマンド、利用可能時の解決済み実行ファイル、利用不可理由を確認できる。
- 利用不可理由は少なくとも、実行ファイル未検出、実行権限なし、探索環境利用不可、判定失敗を区別する。
- Settingsの読込または更新要求が失敗した場合は失敗を表示し、直前に取得済みの状態を利用可能と誤表示しない。

### 4.3 実行コマンド・パス設定

- Providerごとに実行コマンド名または実行ファイルパスを設定できる。
- 設定値は単一のstructured executableであり、shell command、引数、環境変数を含めない。
- 設定値を初期値へ戻せる。初期値へ戻した場合はConfigの上書きを削除し、Provider catalogの既定実行コマンドを使用する。
- 設定更新は永続Configへの保存と対象Providerの再判定を一つの操作として提供する。
- Config保存に失敗した場合は、backend registryとSettings表示を新しい値へ進めない。
- 存在しないパスまたは実行できないパスも設定結果として保持し、理由付きの利用不可状態として返す。
- 設定変更は現在動作中のProvider processを終了・再起動・差し替えしない。変更確定後に開始するProvider processから新しい判定結果を使用する。

### 4.4 再判定

- Settingsから全Providerを手動で再判定できる。
- 手動再判定はlogin-shell `PATH`を再取得可能なplatformでは再取得した後、全Providerを同じbackend処理で判定する。
- CLIの追加、削除、実行権限変更、`PATH`変更、Config上書き変更を、アプリ再起動なしで次のsnapshotへ反映する。
- 同時の再判定または設定更新で、Providerごとに異なる世代の状態を一つのsnapshotとして公開しない。

### 4.5 利用可能の定義

- Provider CLIの「利用可能」は、新しく開始するProvider processへstructured executableとして渡せる実行ファイルが解決されている状態を指す。
- 実行ファイルが解決されていないProviderは、Standalone AgentSession候補へ表示しない。
- Workflowが利用不可Providerを指定した場合は、AgentSession、Provider lifecycle binding、Terminal Surfaceを作成する前に拒否する。
- Providerへのログイン状態、Hook設定、Hook health、Hook trust、Provider TUI起動後の応答、model、permission、sandboxはProvider CLI利用可否へ含めない。
- Hookが未設定または異常でも、実行ファイルが解決されていればProvider CLIは利用可能である。

### 4.6 共通参照とprocess起動

- Settings、`list_available_provider_agent_session_providers`、Standalone AgentSession起動、Workflow Provider検証は同じProvider registry snapshotを参照する。
- Provider processを開始する処理は、利用可能判定に使用した解決済み実行ファイルを同じsnapshotから一度取得し、そのlaunchへstructured executableとして渡す。
- availability確認後に別のConfigコピーから実行ファイルを選び直さない。
- process起動中にSettingsが変更されても、開始済みprocessへ影響させない。
- 暗黙のdefault Providerを設定しない。ユーザーまたはWorkflow定義がProviderを明示する。

## 5. 状態と責任境界

### 5.1 Provider registry

- Provider registryはRust backendが所有するprocess-local state authorityである。
- registryは対応する全Providerの現在snapshotを保持する。
- 各entryはProvider identity、既定実行コマンド、optionalなConfig上書き、実効コマンド、判定結果を一つの整合した状態として持つ。
- 利用可能状態は解決済み実行ファイルを必ず持つ。利用不可状態は解決済み実行ファイルを持たず、理由を必ず持つ。矛盾する中間状態を表現しない。
- entryの生成、判定結果適用、Config上書き変更、resetはdomain modelを通して行う。
- registry snapshotの世代更新は一括して行い、read中に部分更新を公開しない。

### 5.2 Config

- 永続するのはProviderごとのoptionalな実行コマンド・パス上書きだけである。
- 自動検出結果、解決済み絶対パス、利用不可理由は環境から再構築できるためConfigへ保存しない。
- 既存の`agents.claude.cli_path`と`agents.codex.cli_path`をProvider上書きの保存先として使用する。
- `agents.default`を#1603から設定または参照しない。
- 旧Config migration、別設定ファイル、parallel JSON stateを追加しない。

### 5.3 Layer boundary

- domainはProvider registry、entry、availability状態、利用不可理由、設定値の不変条件を所有する。filesystem、process環境、Tauri、serde、Config保存形式を知らない。
- usecaseは初期化、全件再判定、設定保存後の対象再判定、snapshot取得、launch用解決済み実行ファイル取得の手順を所有する。filesystemまたはTauriを直接呼ばない。
- infrastructureはprocess環境の再取得、`PATH`探索、filesystem metadataと実行権限確認を外部世界の形で提供し、Providerやdomain型を知らない。
- gatewayはConfig保存形式およびraw executable probe結果をdomainのProvider設定・判定結果へ変換する。
- controllerはProvider IDと設定入力を閉じたbackend型へ変換し、usecaseを呼び、protocol responseへ変換する。判定規則を持たない。
- frontendは一覧表示、入力draft、保存・reset・refresh要求、処理中表示、error表示だけを担当する。Provider検出、利用可否分類、default command決定、候補filteringを行わない。

## 6. Backend操作契約

Backendは少なくとも次の操作を同じProvider availability usecase上に提供する。

1. 全Providerの現在snapshotを取得する。
2. login-shell `PATH`の再取得を含めて全Providerを再判定し、更新後snapshotを返す。
3. Providerの実行コマンド・パス上書きを保存し、対象Providerを再判定して更新後snapshotを返す。
4. Providerの上書きをresetし、対象Providerを既定実行コマンドで再判定して更新後snapshotを返す。
5. Providerが利用可能かを同じsnapshotから判定する。
6. 新しいProvider process用の解決済み実行ファイルを同じsnapshotから取得する。

既存の`list_available_provider_agent_session_providers`は新しいregistry snapshotから利用可能Provider IDだけを返す選択用queryとして維持する。Settings用queryと候補用queryで別の検出処理を実行しない。

## 7. Settings UI契約

- 既存SettingsのAgent sectionにProvider CLI availabilityを表示する。
- 行はbackendが返したProvider一覧から生成し、Claude / Codexの固定分岐を追加しない。
- 各行に状態、実効コマンド、利用可能時の解決済みパスまたは利用不可理由、上書き入力、reset操作を表示する。
- 全ProviderのRefresh操作を提供する。
- 入力中の文字列はfrontend form stateとして保持してよいが、availabilityの正本にしない。
- 保存成功時はbackendが返した更新後snapshotを表示する。
- 保存失敗時は入力を保存済み扱いにせず、backendの既存snapshotを維持してerrorを表示する。
- Provider上書き以外の旧Agent localStorage設定を#1603のavailabilityまたはProvider選択へ使用しない。

## 8. Acceptanceと自動検証

### 8.1 ATUI-025 product acceptance

ATUI-025として、fixture self-testではなくproductionのTauri command、Provider availability usecase、Standalone AgentSession launch、Workflow Provider validation、Provider process launch境界を通るtestを追加する。

次を一つの製品契約として検証する。

- 起動時に対応する全Providerが初期化され、存在するCLIだけが利用可能になる。
- Settings用snapshotには利用不可Providerと理由も含まれる。
- 候補queryには利用可能Providerだけが含まれ、default選択を返さない。
- 非標準の実行ファイルパスを保存すると、再起動なしで利用可能になり、その実行ファイルで新しいStandalone AgentSessionを起動する。
- 同じ更新後snapshotをWorkflow検証も参照し、利用可能Providerを受理する。
- 実行ファイルの削除後にRefreshすると理由付きで利用不可になり、StandaloneとWorkflowの両方がAgentSession作成前に拒否する。
- 上書きをresetすると既定実行コマンドへ戻り、再判定結果へ反映する。
- path変更前から動作しているProvider processは変更によって停止または差し替えられない。

### 8.2 Unit / integration verification

- domain: available / unavailableの排他的表現、上書き、reset、snapshot一括更新を検証する。
- infrastructure:絶対パス、`PATH`上のcommand、未検出、directory、実行権限なし、探索環境なしを検証する。
- gateway:既存TOML Configのread / update / resetとprobe結果変換を検証する。
- usecase:初期化、refresh、設定保存失敗時のrollback、同時readのsnapshot整合、launch executable取得を検証する。
- controller:未知Provider、空設定、query / update / reset / refreshのprotocol変換を検証する。
- frontend:全Provider表示、利用不可理由、path保存、reset、refresh、loading、error、固定Provider分岐不在を検証する。
- WorkspaceのProvider選択とWorkflow validationが同じregistry結果を使うことを検証する。

## 9. Red-Green-Refactor実施順

すべてのobservable behavior changeを、独立した完全なRed-Green-Refactorで実施する。一つのcycleがRefactor後もGreenになる前に次のproduction behaviorへ進まない。

### Cycle 0: ATUI-025 sensitivity

RED:

- ATUI-025 product acceptanceを追加し、現行production pathではSettings snapshot、path更新、再判定、同一resolved executableによる起動が成立しない理由で失敗することを確認する。

GREEN:

- なし。baseline failureを記録する。

REFACTOR:

- fixture self-testまたは既存のbool availability testがATUI-025を代替していないことを確認する。

### Cycle 1: domain registry

RED:

- Provider entryのavailable / unavailable不変条件、上書き、reset、snapshot一括置換を表すdomain testを先に追加して失敗を確認する。

GREEN:

- 最も単純なProvider registry domain modelを実装する。

REFACTOR:

- Provider catalog、状態判定、default executableの重複をdomainへ集約し、domain testを再実行する。

### Cycle 2: executable probeと初期化

RED:

- executable解決と利用不可理由、およびConfigから全Providerを初期化するusecase testを先に追加して失敗を確認する。

GREEN:

- raw executable probe infrastructure、変換gateway、Provider availability usecase初期化を実装する。

REFACTOR:

- 呼び出しごとの旧bool probeと初期化済みregistryの二重authorityを削除し、focused testを再実行する。

### Cycle 3: Config update / reset / refresh

RED:

- path保存、自動再判定、reset、全件Refresh、保存失敗時state維持、CLI追加・削除反映を先にtestで失敗させる。

GREEN:

- 既存TOML Configを使用する更新・resetとlogin-shell `PATH`再取得を含むRefreshを実装する。

REFACTOR:

- Configコピー、Provider固定分岐、重複した再判定手順を除去し、focused testを再実行する。

### Cycle 4: selection / Workflow / launch統合

RED:

- Standalone候補、Workflow検証、process起動が同じsnapshotを使い、path変更後の新規launchだけが新しいresolved executableを使うtestを先に失敗させる。

GREEN:

- 既存のselection、Workflow port、Provider launchへ共有registry snapshotを接続する。

REFACTOR:

- availability用とlaunch用に別々の実行文字列を保持する構造を削除し、focused testと既存AgentSession / Workflow testを再実行する。

### Cycle 5: Tauri protocolとSettings

RED:

- Settings snapshot、update、reset、refresh commandと、Settings UIの表示・操作・error behaviorを先にtestで失敗させる。

GREEN:

- 薄いTauri controller、protocol response、interface-onlyなfrontendを実装する。

REFACTOR:

- frontendのProvider検出、候補filter、Provider名固定分岐が存在しないことを確認し、focused testを再実行する。

### Cycle 6: ATUI-025 Greenと全体整理

RED:

- Cycle 0のATUI-025が未達の境界を列挙し、production path以外のtest doubleだけで通過していないことを確認する。

GREEN:

- ATUI-025をproduction境界でGreenにする。

REFACTOR:

- dead code、旧Gateway、未使用DTO、重複state、暫定compatibility branchを削除し、関連testと品質gateを再実行する。

各cycleで実行したRED、GREEN、REFACTORのcommandと結果を作業報告へ残す。テストの期待値をimplementationへ合わせて変更しない。

## 10. 非対象

- Provider CLIのインストール、更新、ログイン代行
- Provider lifecycle Hookの設定、health判定、trust判定
- Provider TUI起動後の認証状態または正常応答判定
- Provider CLIのバージョン表示またはversion compatibility判定
- Provider CLIのデフォルト引数、環境変数、model、permission、sandbox設定
- default Providerの設定または自動選択
- Workspace別、Worktree別、Session別のProvider実行パス設定
- remote hostまたはWSL hostごとのProvider検出
- 旧AgentSessionデータまたは旧Configのmigration
- 旧Agent GUIと旧Agent localStorage設定の最終削除。これは#1599が所有する

## 11. 品質gate

実装完了前に少なくとも次を実行し、すべて成功させる。

```bash
pnpm lint
pnpm test
pnpm build

cd src-tauri
cargo fmt --check
cargo clippy -- -D warnings
cargo test

cd ..
pnpm test:integration
```

加えて、ATUI-025を含む対象acceptance testを単独実行し、失敗時に他testの偶然の状態へ依存していないことを確認する。

## 12. 完了条件

次のすべてを満たした場合だけ#1603の実装完了とする。

- 本Specの外部仕様、状態不変条件、責任境界を満たす。
- ATUI-025がproduction境界で成功する。
- Provider availabilityのstate authorityがRust backendの一つのregistryだけである。
- 初期化、Settings query、update、reset、refresh、Standalone選択、Workflow検証、process起動が同じsnapshotを参照する。
- 通常のTerminalから実行可能なCLIと、Settingsで指定した非標準パスを利用できる。
- CLI追加・削除・path変更をアプリ再起動なしで反映できる。
- 利用不可理由をSettingsで確認できる。
- 暗黙のdefault Providerを導入していない。
- Hook、auth、model、permission、sandboxをavailabilityへ混入させていない。
- frontendにProvider検出ロジック、候補filter、Provider名の固定分岐がない。
- availability判定用とprocess起動用に別の実行文字列stateを保持していない。
- 新規・変更したbehaviorがRed-Green-Refactorの各cycleで検証されている。
- 関連する既存testと品質gateがすべて成功する。
