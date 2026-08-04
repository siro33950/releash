# Issue #1596 review remediation specification

## Status

Issue #1596 の実装レビューで確定した全指摘を修正するための実装仕様。

本仕様に記載された修正、Red-Green-Refactor、受け入れ検証、規約準拠、品質ゲートがすべて完了するまで、Issue #1596 の修正は完了ではない。

## Sources of truth

- GitHub Issue #1596 本文と全コメント
- GitHub Issues #1594 から #1599 の責任境界
- `specs/milestone-87-agent-tui-cutover/acceptance-contract.md`
- `specs/milestone-87-agent-tui-cutover/issue-1596-provider-lifecycle.md`
- repository `AGENTS.md`
- `src-tauri/AGENTS.md`
- `.claude/rules/rust-first-logic.md`
- `docs/architecture/README.md`
- `docs/architecture/DOMAIN.md`
- `docs/architecture/USECASE.md`
- `docs/architecture/CONTROLLER.md`
- `docs/architecture/GATEWAY.md`
- `docs/architecture/INFRASTRUCTURE.md`
- `docs/architecture/TEST.md`
- supported installed Claude Code / Codex CLI と各公式 Hook 仕様

## Root cause

Provider lifecycle の個別部品はテストされていたが、product acceptance が次の実境界を通っていなかった。

- production Local API router composition
- `ProviderLaunchSpec` が生成した実際の Hook command
- build profile ごとの Releash CLI alias 解決
- supported installed Provider CLI による Hook configuration の読み込みと実行

そのため、テストが Green のまま次の未達が残った。

1. Hook 欠落または Codex Hook trust 未確認時の診断事実が存在しない。
2. debug build の正規 alias が `releash-dev` であるのに、Hook command が `releash` に固定されている。
3. product acceptance が acceptance 専用 router を使い、production router wiring の欠落を検出できない。
4. installed Claude Code / Codex CLI に対する再現可能な characterization がない。
5. Other-AgentSession acceptance が、誤った stream への書き込みを検出できない。
6. 新規テストの配置と命名が `docs/architecture/TEST.md` に準拠していない。

## Scope boundary

### Included

- Provider lifecycle を提供できない状態の Domain 表現、durable fact、usecase、codec、Local API 境界。
- Claude / Codex の build profile 対応 Hook CLI command 生成。
- production Local API router を通る product acceptance。
- supported installed Claude Code / Codex CLI の characterization。
- cross-scope rejection acceptance の誤通過防止。
- Issue #1596 で追加・変更したテストの規約準拠。
- 上記修正に必要な最小限の composition と test fixture の整理。

### Excluded

- 実際の Provider TUI process と durable Terminal Surface の接続。Issue #1597 の責任。
- validated Stop / Submit による Completed / WaitingApproval / Stalled 遷移。Issue #1598 の責任。
- lifecycle unavailable に伴う AgentSession、PTY、Terminal Surface の自動終了。
- Provider transcript body の読み込み、複製、所有。
- Releash application process またはコンピュータ再起動後の lifecycle continuity。
- 旧永続化データの migration。
- Issue #1599 が所有する残存 GUI / Message runtime の削除。

## Required development method

すべての observable behavior change は Red-Green-Refactor で実施する。

1. RED
   - 受け入れ済みの外部仕様を表す最小のテストを先に追加する。
   - 対象テストを実行し、意図した未実装または誤実装によって失敗することを確認する。
   - runnable な black-box test を作れる場合、未完成テストだけによる compile failure を RED の証拠にしない。
2. GREEN
   - RED を満たす最小の production implementation を追加する。
   - focused test と直接関連する test module を実行する。
3. REFACTOR
   - 重複、acceptance 専用 production seam、不要な分岐、無効な依存を削除する。
   - focused test と関連 test を再実行し、Green を維持する。

acceptance test 自体の検出力を修正する箇所では、対象不変条件を一時的に破る controlled mutation を用いてテストが RED になることを確認し、その mutation を残さない。

## Remediation 1: unavailable lifecycle diagnosis

### Required external behavior

- Releash が起動した Provider で必要な SessionStart を観測できない場合、暗黙の Stop や workflow progress を生成しない。
- unavailable の理由を対象 launch binding、AgentSession、workflow execution、NodeExecution、attempt と関連付ける。
- unavailable は durable local event store に診断可能な事実として保存する。
- 同一 unavailable observation の再送は冪等で、二重の診断事実を作らない。
- unavailable 確定後の stale signal は current attempt を進めない。
- unavailable によって AgentSession、PTY、Terminal Surfaceを終了しない。
- unavailable 自体は workflow Node state を変更しない。
- Provider Hook CLI が呼ばれなかった場合にも診断経路が成立する。

### Domain ownership

Domain は次を所有する。

- retry attemptを跨ぐ同一実行系列を表すopaqueな`ProviderLifecycleSlotId`。
- Slotごとにcurrent bindingを高々一つだけ保持する`ProviderLifecycleSlot`集約。
- 同一Slotへの新binding armで旧bindingを失効し、新bindingへ置換する遷移。
- active binding を unavailable として確定できるかの受理判定。
- unavailable reason の閉じた分類。
- 同一 unavailable observation の冪等性。
- unavailable 後の signal 受理可否。
- unavailable または失効を表す dedicated Provider lifecycle event。

live stateは過去binding一覧を保持しない。旧bindingの履歴はdurable `BindingExpired` factを正本とし、Slotのcurrent bindingと一致しないsignalはaccepted lifecycle factを追加せず拒否する。

reason は少なくとも次の外部事象を区別できなければならない。

- SessionStart が起動期限までに観測されなかった。
- Codex Hook trust が確認できず、SessionStart が観測されなかった。
- Provider が Hook configuration を受理しなかった。
- lifecycle delivery に必要な Local API を利用できなかった。

Codex trust 不足を直接観測できない場合、理由は trust 不足を断定せず、「trust が必要な Codex Hook の delivery を確認できなかった」という観測事実を表す。

### Application boundary

- Issue #1597 の launch supervisor が lifecycle unavailable を報告できる command 境界を Issue #1596 で提供する。
- command は Slot identity、binding identity、scope、reason を Domain へ渡し、durable fact を保存する。
- controller、CLI、gateway は unavailable の受理判断を持たない。
- #1596 の command は workflow transition または process kill を呼ばない。
- UsecaseはSlot単位のlock、Domain candidate生成、scoped factのatomic append、成功後のcandidate確定をこの順で所有する。
- persistence失敗時はcandidateを破棄し、直前のlive Slotを維持する。
- registry全体のclone、durable I/O中のregistry-wide lock、liveな過去binding保持を禁止する。
- Provider StopまたはNode完了ではSlotをreleaseしない。明示的なAgentSessionまたはProvider終了だけがSlot identityとexpected binding identityを照合し、durable expiry成功後にcurrent bindingをreleaseできる。遅延した旧bindingの終了通知でreplacementをreleaseしない。

### Red-Green-Refactor

1. RED: SessionStart 未観測または Codex trust 未確認を報告しても durable diagnosis が存在しない。
2. GREEN: Domain state/event、usecase、versioned codec、Local API command を追加する。
3. RED: 同一 unavailable の重複、scope mismatch、expired binding、遅延 signal を入力する。
4. GREEN: Domain の冪等性と拒否規則を満たす。
5. REFACTOR: StopFailure と unavailable の意味を分離し、共通化は識別・永続化mechanicsだけに限定する。

## Remediation 2: build-profile-aware Hook CLI command

### Required external behavior

- release build の Hook command は `releash hook receive --provider <provider>` を使用する。
- debug build の Hook command は `releash-dev hook receive --provider <provider>` を使用する。
- Claude plugin と Codex per-process config は同一の解決済み Hook CLI alias を使用する。
- Hook command は Releash が起動した Provider processだけに適用する。
- Providerのglobal user settingsへCLI path、port、token、capabilityを保存しない。
- `--dangerously-bypass-hook-trust` を使用しない。
- command文字列は同一build profile内で安定し、trust判定を不必要に変化させない。

### Layer boundary

- build profile と alias の正本は既存 `infrastructure/platform/path_aliases.rs` とする。
- adaptor gateway の `ProviderLaunchSpec` は infrastructure module をimportしない。
- composition boundary が解決済み Hook CLI alias を `ProviderLaunchSpec` へ入力する。
- `ProviderLaunchSpec` 内の `releash` hard-code をすべて削除する。
- alias は Releash が管理する既知の値だけを受け取り、user inputをshell commandへ連結しない。

### Acceptance correction

- product acceptance は `CARGO_BIN_EXE_releash` をHook executableとして直接指定しない。
- test data directory 配下にbuild profile対応alias wrapperを用意し、Provider launchと同じPATH解決でactual Releash CLI subprocessへ到達する。
- Claude plugin / Codex configに埋め込まれたcommandと、acceptanceが実行するcommandを同じlaunch resultから取得する。

### Red-Green-Refactor

1. RED: development profileで生成commandが`releash-dev`にならない。
2. GREEN: compositionでaliasを解決しlaunch specへ渡す。
3. RED: generated Hook commandをPATH経由で実行すると別binaryまたはcommand-not-foundになる。
4. GREEN: profile対応wrapperからactual CLI subprocessへ到達させる。
5. REFACTOR: Claude/CodexのHook command生成をProvider非依存の最小mechanicsへ集約する。

## Remediation 3: production Local API router acceptance

### Required external behavior

product acceptance は次の経路を通る。

```text
agent_tui_fixture
    -> generated Hook CLI command
    -> actual Releash CLI subprocess
    -> dynamic Local API discovery
    -> bearer authentication
    -> production build_router
    -> production Provider lifecycle controller
    -> production usecase
    -> Domain
    -> SQLite local event store
```

### Composition correction

- `build_provider_lifecycle_router` を削除する。
- application runtimeとacceptanceが唯一のproduction `build_router` を使用する。
- router dependenciesを明示的なcomposition inputへ整理してもよいが、Provider lifecycle専用acceptance routerは残さない。
- acceptanceに不要なAgentSession / Terminal Surface dependenciesはproduction routerが許容する既存optional boundaryを使う。
- workflow router用collaboratorはテスト用実装を渡してよいが、Provider lifecycle route、auth middleware、controller、usecaseはproductionと同一にする。

### Test sensitivity proof

- production routerからProvider lifecycle route mergeを一時的に除いたcontrolled mutationでproduct acceptanceがREDになることを確認する。
- mutationを戻した後、production compositionでGreenにする。
- acceptance専用routerを復活させる変更を防ぐ構造監査を追加する。

## Remediation 4: installed Provider CLI characterization

### Required characterization targets

supported installed Claude CodeとCodex CLIについて次を実行検証する。

- executableのversionを取得し、supported versionであることを確認する。
- generated per-process Hook configurationを実CLIが受理する。
- SessionStartがactual Provider CLIからReleash Hook CLIへ届く。
- Stopがactual Provider CLIからReleash Hook CLIへ届く。
- user HookとReleash Hookが同一Provider起動で共存する。
- user settings fileが実行前後でbyte-for-byte不変である。
- Releash HookがReleash起動processだけに適用される。
- Codex trust未確認時はHookが実行されず、trust bypassも行われない。
- Hook CLI / Local API failureがProvider processを強制終了しない。

### Test structure

- `src-tauri/tests/provider_lifecycle_characterization_test.rs` に配置する。
- fake Providerだけのテストをcharacterization evidenceとして扱わない。
- installed CLI、認証、TTY等を必要とするため、通常unit testとは分離した明示実行gateにする。
- characterization gate実行時にexecutableが無い、versionがunsupported、必要な公式隔離機構が使えない場合はskipせず、診断可能な失敗を返す。
- temporary configurationはProvider公式の隔離方法だけを使用する。
- 実ユーザー設定は書き換えない。
- provider configuration isolation、non-network startup、Hook trust確認方法が不明な場合は、実装前に公式ドキュメントとinstalled CLIで確定する。
- characterizationの実行commandと結果を再現可能にする。

### Red-Green-Refactor

1. RED: installed CLIを起動するcharacterizationが存在せず、generated configurationの実行を証明できない。
2. RED: development aliasを含むgenerated Hook commandでactual lifecycle deliveryが成立しない。
3. GREEN: supported installed CLIでuser Hook共存、settings不変、lifecycle deliveryを成立させる。
4. RED: Codex trust未確認条件で診断事実が残らない。
5. GREEN: fail-closed diagnosis boundaryへ接続する。
6. REFACTOR: fake Provider acceptanceとinstalled CLI characterizationの責任を明確に分離する。

## Remediation 5: rejection acceptance false-positive removal

### Required assertions

次の各rejection scenarioで、単一AgentSession streamだけでなくledger全体の不変性を確認する。

- previous attempt
- Other-AgentSession
- Other-workflow execution
- Other-NodeExecution
- invalid capability
- stale capability
- expired binding
- provider mismatch
- malformed payload

各scenarioで次を検証する。

- original AgentSession streamにaccepted lifecycle eventが追加されない。
- 改変先AgentSession streamにaccepted lifecycle eventが追加されない。
- signal送信前後でglobal accepted Provider lifecycle event件数が増えない。
- expected rejection reasonがLocal API responseまたはHook CLI diagnosticに残る。
- Provider-required stdoutは有効なままである。
- Hook CLIはProvider processを終了させるexit codeを返さない。
- workflow event、kill operation、transcript body persistenceが発生しない。

### Test sensitivity proof

- Domainのscope checkを一時的に無効化するcontrolled mutationでOther-AgentSession acceptanceがREDになることを確認する。
- mutationを戻し、Domainの拒否とledger全体不変でGreenにする。

## Remediation 6: test convention compliance

### File placement

Issue #1596で追加・変更した同一directoryのRust testを`*_test.rs`へ配置する。

- `usecase/provider_lifecycle/provider_lifecycle_usecase_test.rs`
- `adaptor/gateway/provider_lifecycle/provider_lifecycle_gateway_test.rs`
- `adaptor/controller/api/provider_lifecycle_controller_test.rs`
- `cli/hook_test.rs`
- 必要に応じてcodec用`provider_lifecycle_codec_test.rs`
- `tests/provider_lifecycle_acceptance_test.rs`
- `tests/provider_lifecycle_characterization_test.rs`

### Test naming

すべてのtest functionを次の形式にする。

```text
test_{業務機能}_{条件と期待結果}
```

例:

```rust
fn test_Providerライフサイクル受信_別AgentSessionの信号ではledgerを変更しない()
```

- 業務機能と条件・期待結果を日本語で表す。
- helper functionはtest function命名規約の対象外。
- test module名は`{implementation_name}_tests`とする。
- integration testもtest function命名規約に従う。
- テスト期待値をproduction implementationへ合わせて変更しない。

本Remediationは全behaviorがGreenになった後のRefactorとして実行する。

## Required implementation order

1. 現行quality baselineを記録する。
2. product acceptanceをgenerated Hook commandとproduction routerへ切り替える。
3. controlled mutationでacceptanceがproduction route欠落を検出することを確認する。
4. unavailable diagnosisのDomain REDを追加する。
5. Domain event、usecase、codec、Local APIをGreenにする。
6. build profile対応Hook CLI commandのREDを追加する。
7. alias compositionとlaunch specをGreenにする。
8. installed Claude Code / Codex CLI characterizationを実行する。
9. unavailable/trust diagnosisをcharacterization結果へ接続する。
10. cross-scope rejection acceptanceをglobal ledger不変検証へ変更する。
11. controlled mutationでOther-AgentSession false-positiveが解消されたことを確認する。
12. test file配置、module名、function名を規約どおりにRefactorする。
13. legacy Hook production pathとdead codeを再監査する。
14. 全品質ゲートを実行する。

各stepでfocused RED、focused GREEN、related tests Greenを記録し、複数stepのproduction implementationを先行してまとめて書かない。

## Product acceptance completion matrix

ClaudeとCodexの両方で次を満たす。

- generated profile-aware Hook commandからactual Releash CLIへ到達する。
- dynamic discoveryとbearer authenticationを通る。
- production Local API routerを通る。
- correct SessionStart、session identity、opaque transcript reference、Stopがdurableである。
- duplicate SessionStart / Stopが冪等である。
- delayed Stopが正しいattemptだけに関連付く。
- missing SessionStartがStopを生成せず、diagnosable unavailable factになる。
- missing StopがStopを推測しない。
- previous attempt、Other-AgentSession、Other-workflow、Other-NodeExecutionを拒否する。
- invalid、stale、expired capabilityを拒否する。
- malformed payloadを拒否する。
- missing discoveryでaccepted factを作らず、diagnosticを保持する。
- Provider process exitとvisible terminal textをStopにしない。
- Provider process、AgentSession、PTY、Terminal Surfaceを自動終了しない。
- workflow stateを変更しない。
- transcript body、raw capability、raw bearer tokenを永続化しない。
- global user settingsを変更しない。
- installed Provider CLIがgenerated configurationを受理する。
- Codex Hook trustをbypassしない。

## Legacy audit

production source、current Config、Tauri command registration、frontend、current documentationについて次が残っていないことを確認する。

- `server.hook_port`
- `/hooks/agent`
- `generate_hooks_config`
- `apply_hooks_config`
- `get_hooks_status`
- global Claude settings mutation
- fixed Local API portまたはglobal Hook token
- legacy Hook Domain / Usecase / Gateway modules

過去Issueのhistorical spec内の記録とcurrent production contractを区別し、current sourceとして参照される文書に旧経路を残さない。

## Quality gates

最低限、次をすべて実行して成功させる。

```bash
cd src-tauri
cargo fmt --check
cargo clippy -- -D warnings
cargo test

cd ..
pnpm lint
pnpm test
pnpm build
pnpm test:integration
```

加えて次を実行する。

- Provider lifecycle focused unit / integration tests。
- ATUI-020 / ATUI-021 product acceptance。
- installed Claude Code / Codex CLI characterization gate。
- production sourceに対するlegacy Hook audit。

## Completion criteria

次のすべてを満たした場合のみ完了とする。

- Remediation 1から6がすべて実装済み。
- 各observable behaviorでREDを意図どおり観測済み。
- 各GREEN後にfocused testとrelated testsが成功済み。
- controlled mutationによってproduction router acceptanceとOther-AgentSession acceptanceの検出力を確認済み。
- acceptance専用production routerが削除済み。
- build profileとHook CLI aliasが一致している。
- unavailable/trust failureがdurableかつ診断可能である。
- installed Provider CLI characterizationが成功している。
- test配置、module名、function名が規約準拠済み。
- Issue #1597 / #1598 / #1599の責任を取り込んでいない。
- legacy auditが成功している。
- 全quality gateが成功している。
