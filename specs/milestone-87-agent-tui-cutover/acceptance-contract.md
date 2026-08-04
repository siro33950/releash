# Agent TUI cutover acceptance contract

## 目的

本書は、Agent GUIをprovider CLI TUIへ置き換える際に、利用者とworkflowから観測できる保証とrelease gateを定める。Rustの型、保存形式、providerごとのHook設定、terminal snapshot形式は後続Issueで決定する。

## 状態の正本

| 状態 | 正本 | Releashの責務 |
|---|---|---|
| conversation、thinking、tool表示、provider permission | provider CLIとprovider transcript | CLI TUIを改変せず表示し、独自Message modelへ複製しない |
| live PTY、terminal checkpoint、bounded scrollback | Releash backend | frontendのmount状態から独立して保持し、同じ状態を各surfaceへ公開する |
| NodeExecution、attempt、Submit、Artifact、Approval | Releash workflow | durableに記録し、providerの表示文言から推定しない |
| provider session identity、Stop signal | provider lifecycle integrationを経由したReleashのledger | session、NodeExecution、attemptとの対応を検証する |

frontendはbackend-owned stateのprojectionであり、workflow遷移またはterminal継続性の正本にならない。

## Terminal Surface identity

Terminal SurfaceのownerはRust backendが型として解決し、frontendが生成する一時的なPTY IDやsession keyをdurable identityにしない。

| Owner | Workspace内のcardinality | Surface identity |
|---|---:|---|
| Workspace terminal | 1 | `WorkspaceIdentity` |
| AgentSession | 1:N | `WorkspaceIdentity + SessionId` |

Workflow、Fanout、Command executionはTerminal Surfaceを所有しない。Command executionは現行どおり非PTYで実行し、分離したstdout、stderr、exit code、durationをworkflow stateの正本として維持する。

## 外部保証

1. ClaudeとCodexのCLI TUIで対話、permission応答、再指示を行える。
2. xtermのunmount / remount、tab・workspace切替、renderer reload後も同一live PTYへ欠落・重複なく再接続できる。
3. App process restart後は最終画面とbounded scrollbackを復元できる。同一PTYまたはprocessの継続は保証しない。
4. Submitとprovider Stopを別signalとして扱い、同じattemptの両方が揃った場合だけAutoまたはApprovalの次状態へ進む。
5. ArtifactはSubmitへ任意で添付でき、Artifactなしでも完了意思を表明できる。
6. signal欠落、重複、遅延、別attemptからの到着を、providerのterminal表示文言に依存せず判定する。
7. Node完了後もAgentSessionとlive PTYを終了せず、Releashは自動入力または自動resumeを行わない。
8. 旧AgentSessionデータを読み取り、変換、backfillしない。

## Acceptance scenario

| ID | 観測する境界 | 合格条件 | Owner |
|---|---|---|---|
| ATUI-000 | 実PTYとout-of-band lifecycle fixture | 実PTYのANSI / Unicode入出力と、Submit / Stopの欠落・重複を独立して再現できる | #1594 |
| ATUI-010 | live terminal attach | remount、tab・workspace切替、renderer reload中の出力にgap、duplicate、順序逆転がない | #1595 |
| ATUI-011 | terminal checkpoint | alternate screen、cursor、style、wide character、resize、bounded scrollback、終了画面を復元できる | #1595 |
| ATUI-012 | App process restart | 最終画面とbounded scrollbackだけを復元し、同一process継続を成功扱いしない | #1595 |
| ATUI-020 | provider lifecycle | Claude / Codexのsession、transcript参照、Stopを正しいAgentSessionとattemptへ関連付ける | #1596 |
| ATUI-021 | lifecycle fault | signalの重複、遅延、欠落、別sessionからの混入をfail-closedで扱う | #1596 |
| ATUI-030 | AgentSession vertical slice | 開始、対話、permission、reload、明示終了が旧Message projectionなしで成立する | #1597 |
| ATUI-040 | Auto workflow | 同じattemptのSubmitとStopが揃った場合だけCompletedとなる | #1598 |
| ATUI-041 | Approval workflow | 同じattemptのSubmitとStopが揃った場合だけWaitingApprovalとなり、Approve / Rejectできる | #1598 |
| ATUI-042 | workflow fault | 片側signal、遅延signal、Retry、完了後の追加質問が定義済み状態へ収束する | #1598 |
| ATUI-050 | atomic cutover | 旧runtime経路が削除され、Config、Hook、canonical docs、release buildが新契約だけを参照する | #1599 |

ATUI-000は後続Issueが利用するfixture自体のself-testであり、ATUI-010以降のproduct保証を代替しない。各Owner Issueは該当scenarioをproduct境界へ接続してから完了する。

## Harness self-test

実行入口:

```bash
cd src-tauri
cargo test --locked --test agent_tui_harness
```

fixtureは実PTY上でANSI / Unicodeを出力し、PTY入力を受け取る。Submit / Stopはterminal outputへ埋め込まず、別transportで構造化signalとして送る。表示文言を変更してもlifecycle判定が変わらないことをcontractとする。

#1594では、後続Issueがproduct境界の失敗を再現できるように次のharness能力をself-testする。

| Harness能力 | Self-testする内容 |
|---|---|
| 実PTY | ANSI、alternate screen、Unicode / wide character、入力、resize、任意exit code |
| surface非接続 | renderer相当のconsumerが存在しなくてもbackend readerがPTYをdrainし、後からcaptureを観測できる |
| lifecycle独立経路 | terminal表示と分離したSubmit / Stop、provider session、attempt、transcript参照 |
| lifecycle fault注入 | 順序逆転、片側または両側欠落、Submit / Stop重複、遅延、別session / attempt、sequence異常、不正payload |
| process境界 | provider process終了をStop signalとして扱わず、両者を独立して観測できる |
| contract / release gate | 全scenario IDがcanonical contractに存在し、integration branchでCIを実行しつつrelease sourceにしない |

PTYのOS bufferをrenderer向けretentionとして使わない。rendererが外れている間もbackend readerは停止せず、#1595で実装するbackend-owned terminal stateへ継続して取り込む。

このコマンドは試験設備の自己テストであり、ATUI-010以降のproduct acceptance成功を表さない。fixtureの共通moduleは`src-tauri/tests/support/agent_tui_fixture.rs`とする。各Owner Issueは同moduleを利用するproduct acceptanceを追加し、自身のscenarioを実装前に失敗、実装後に成功させる。

## Branchとrelease gate

- integration branchは`feature/milestone/87`とする。
- 各Issueの子branchは`feature/milestone/87`をbaseにし、同branchへ統合する。
- `main`では現行製品のreleaseを継続する。
- `feature/milestone/87`の途中状態からtagまたは製品releaseを作成しない。
- #1599以外の変更で旧Agent GUIと新Agent TUIの混在状態を`main`へmergeしない。
- #1599の最終PRは本表の全scenario、通常CI、package / release buildが成功してから`feature/milestone/87`を`main`へ一括統合する。

## 保証外

- App process restartを跨ぐ同一PTY / processの継続
- provider resumeの成功とresume後のprovider挙動
- Releash管理外へescapeしたbackground processの停止
- 旧AgentSessionデータのmigration、互換reader、物理削除
- provider表示文言の固定
