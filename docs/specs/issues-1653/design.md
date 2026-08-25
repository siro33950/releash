# Design

## The actual design

### Architecture

#### terminal spawn失敗理由の所有と伝播

terminal spawn失敗の分類は、既に発生源で区別されている`TerminalSurfaceSpawnReservationError`と`TerminalSurfaceGatewayError`を正本とし、`src-tauri/src/usecase/terminal_surface/error.rs`および`src-tauri/src/usecase/terminal_surface/spawn_usecase.rs`で文字列へ潰さずに保持する。PTY runtime生成を呼ぶ箇所のエラーはPTYのopen／fork失敗として保持し、checkpoint読込、output reader開始、既存exited surfaceのdrain、runtime lifecycleによるmutation拒否など呼び出し元へ到達する残りの失敗は、発生源のmessageを保ったまま「上記以外のspawn失敗」として保持する。owner衝突として呼び出し元へ到達するのは`spawn_usecase.rs`のowner identity collision経路であり、これをowner衝突として保持する。`TerminalSurfaceSpawnReservationError::OwnerOccupied`は呼び出し元へ返らないため分類対象にしない。

`src-tauri/src/domain/agent_session/provider_terminal_gateway.rs`では、`ProviderAgentTerminalGateway::spawn`だけがspawn固有のtyped errorを返すようにする。presence、stop、deleteなどの既存操作は従来の`ProviderAgentTerminalGatewayError`を使い続ける。`src-tauri/src/adaptor/gateway/agent_session/provider_agent_terminal_gateway.rs`はTerminal Surface側のtyped errorを、per-worktree cap、総数cap、owner衝突、PTY runtime生成失敗、およびそれ以外のspawn失敗の区別と必要なpayloadを保ったAgentSession側のerrorへ変換する。error messageの内容を再解析して分類しない。

`src-tauri/src/usecase/agent_session/agent_session_launch.rs`の`AgentSessionLaunchUsecaseError`はterminal spawn失敗のtyped errorを保持する。新規起動の共通`spawn_prepared`経路とhistory resume経路は、失敗理由を取得した時点で記録し、その後に既存のrollback／cleanupを行う。これによりcleanupが別の失敗を返しても、最初のspawn失敗理由は失われない。既存のcap値、待機、retry、queue、AgentSession lifecycle遷移は変更しない。

#### 経路ごとの記録owner

- workflowのSession Nodeでterminal spawnを行うのは`activate_workflow_node`経路である。`src-tauri/src/adaptor/gateway/workflow/node_session_boundary.rs`の`activate_workflow_agent_session`でtyped errorの`Display`表現を`WorkflowRuntimeError::AgentSession`へ載せる。terminal spawnを行わない`prepare_workflow_agent_session`側のformattingは変更しない。既存の`workflow_host`の失敗収束から`WorkflowEvent::NodeFailed`、`ProcessExitedFact.failure_reason`へ至る経路をそのまま使い、event storeにNodeの失敗理由を永続化する。
- standalone起動とhistory resumeは、`agent_session_launch.rs`がspawn失敗を`log::error!`で記録する。standaloneは呼び出し元へ既存の汎用エラーを返す前、history resumeは既存のPaused収束処理へ進む前に記録する。
- workflow経路でも同じ`log::error!`記録を残す。workflowの判定材料となる正本はNodeの既存failure fact、プロセス横断の障害調査記録はローカルログとし、両者のownerを混同しない。

根拠は、AgentSessionがTerminal ownershipを持ち、NodeExecutionはAgentSessionを参照するという`docs/architecture/README.md`および`docs/glossary/DOMAIN.md`の境界、domain portの外部エラーをgatewayで変換する`docs/architecture/GATEWAY.md`、業務手順とrollback順序をusecaseが所有する`docs/architecture/USECASE.md`である。

#### ローカルloggerの責務

ローカルファイルloggerの構築と`log` facadeへの登録は、新設する`src-tauri/src/infrastructure/local_log.rs`が所有する。ファイル出力、rotation、プロセス間の排他をOS資源として扱う責務であり、`docs/architecture/INFRASTRUCTURE.md`の境界に従う。

loggerはTauri pluginとして登録しない。`src-tauri/src/main.rs`は引数がある場合にTauri builderを構築せずCLIへ分岐するため、plugin登録だけではCLIプロセスの出力先が無いままになる。GUIプロセスは`src-tauri/src/lib.rs`がcomposition rootとしてapplication setupの冒頭で登録し、CLIプロセスは`src-tauri/src/cli/mod.rs`の`run`がsubcommand dispatchより前に登録する。これによりsetup内およびその後の既存`log::warn!`／`log::error!`と、CLI経路の同じ呼び出しが同じloggerを使う。

出力先は各プロセスがそのプロセスのdata dirを解決する既存の経路に従う。GUIは`infrastructure/platform/app_data_dir.rs`の`resolve_data_dir`が返すTauri解決のdata dirを使い、event store、terminal checkpoint、子プロセスへ伝搬する`RELEASH_DATA_DIR`と同じ値になる。この解決は`AppHandle`を要するため、GUIのlogger登録はsetupより前へ移せない。CLIは`RELEASH_DATA_DIR`を優先する`cli/common.rs`の`resolve_data_dir`の結果を使う。GUIが子プロセスへ自分のdata dirを伝搬するため既定の起動経路では同一ファイルを共有するが、CLI起動時に任意の`RELEASH_DATA_DIR`を明示した場合は別の出力先になる。

### Interface

terminal spawn失敗の記録は、次の`kind`とpayloadで外部から区別できる表現に統一する。

| 失敗 | 記録上のkind | 必須payload |
| --- | --- | --- |
| per-worktree PTY cap到達 | `per_worktree_cap` | `worktree_path` |
| PTY総数cap到達 | `total_cap` | なし |
| owner衝突 | `owner_conflict` | なし |
| PTYのopen／process spawn失敗 | `pty_spawn` | 発生源のerror message |
| 上記以外のspawn失敗 | `other_spawn_failure` | 発生源のerror message |

ローカルログは`AgentSession terminal spawn failed`という事象名、AgentSession ID、上表のkindとpayloadを一つのerror recordの`message`へ含める。これらを個別のJSON fieldへ分けず、workflowのNode失敗理由と同じ文字列表現を使って、同じ事実の表現を二つにしない。workflowのNode失敗理由は`activate_workflow_agent_session`が組み立てる既存の`activate Workflow AgentSession '<agent_session_id>': ...`というcontextを維持し、その末尾に同じkindとpayloadを含むtyped errorの`Display`表現を置く。`workflow_host`が前置する`workflow runtime activation failed: `は既存表現のまま変更しない。`Debug`表現には依存しない。

Tauri command `create_agent_session`と`resume_agent_session_history_candidate`の名前、引数、戻り値は変更しない。standalone起動が失敗したときのcode `AGENT_SESSION_TERMINAL_UNAVAILABLE`と利用者向けmessageも維持し、失敗理由をUI responseへ追加しない。local API、CLI、terminal protocolにも新しい公開契約を追加しない。

sharedなTerminal Surface error型の変更後も、`src-tauri/src/adaptor/controller/command/terminal_surface/commands.rs`はper-worktree／total capを既存の`PTY_ERROR_CODE_CAP_REACHED`へ、それ以外を既存のgeneric codeへ変換する。AgentSessionのworkflow error formattingを変更するのはterminal spawn固有errorだけとし、その他の`AgentSessionLaunchUsecaseError`がNode失敗理由へ載る既存表現は維持する。

### Data Model

AgentSessionのterminal portにspawn専用errorを追加し、分類とR-002／R-003で必要なpayloadだけを保持する。これは一回のspawn試行結果を表す値であり、identity、lifecycle、独立した永続状態を持たない。`AgentSessionLaunchUsecaseError`はこの値をsourceとして保持し、standalone request registryで再利用できるよう`Clone`可能にするが、文字列化した複製は保持しない。

ローカルログrecordはtimestamp、level、Rust target、message、および書き手がGUIプロセスとCLIプロセスのどちらかを判別できる値を持つ1行形式とする。terminal spawn失敗の事象名、AgentSession ID、kind、payloadは`message`の内容であり、独立したfieldにしない。AgentSession本文、PTY出力、workflow state全体はログへ保持しない。ログファイルにschema versionは導入しない。

### Database

新しいtable、event種別、migrationは追加しない。workflowの失敗理由は既存の`ProcessExitedFact.failure_reason`へ保存する。standalone／history resumeの記録はdata dir解決に従うローカルのローテーション対象ファイルであり、SQLite event storeへ複製しない。

### UI/UX

画面、操作フロー、表示文言は変更しない。失敗理由のアプリ内閲覧・検索・案内UIは追加せず、standalone起動失敗時は既存の汎用エラー表示を維持する。

### Algorithm

terminal spawn失敗時は、発生源のtyped errorをAgentSession側のspawn errorへ一度だけ変換する。新規起動では、同じerror値からローカルログと`AgentSessionLaunchUsecaseError`を生成し、一次原因を保持したまま既存rollbackを最後まで実行する。workflow境界はそのerrorを`Display`でNode失敗理由へ変換する。history resumeでは、同じerror値をローカルログへ書いた後に既存cleanupを実行し、cleanup成功時は従来どおりPausedを返す。

分類にmessage prefix／substringの照合を使わない。worktree pathは`WorktreeCapReached`が保持する値を、PTY error messageは`spawn_runtime`が返した`TerminalSurfaceGatewayError`の内容をそのまま伝播させる。

### Infra

新しいlogging crateは追加せず、`local_log.rs`が標準のファイルI/Oと既存依存の`fs2`によるadvisory file lockでfile loggerを構成し、`src-tauri/src/infrastructure/mod.rs`から公開する。次を固定する。

- level filterは`Warn`とし、要求対象のwarning／errorだけを記録する。
- 出力先はdata dir解決配下のログディレクトリの単一ファイルとし、basenameは`releash`とする。OS固有の絶対pathはコードへ固定しない。stdout、stderr、Webview、network targetへは出力しない。
- 同じdata dirを解決したGUIプロセスとCLIプロセスは同じファイルを共有する。各プロセスのbounded channelと専用writer threadがfile I/Oを所有し、`log::Log::log`はrecordを非ブロッキングでenqueueする。queue満杯時は新しいrecordを捨て、drain再開後のrecordに破棄件数を示す。`log::Log::flush`はそれ以前に受理したrecordのdrain完了を待ち、CLIは`run`の戻り際にflushする。data dirとlogsディレクトリの生成、書き込み、rotationはwriter threadで初回record処理時に行い、初期化時にはfile I/Oを行わない。
- process間の排他対象はログディレクトリ内の専用lock file `releash.lock` に固定し、active fileと世代fileをlock対象にしない。writer threadはこのlockを`fs2`のblocking exclusive lockで取得し、保持したままactive fileのopen、書き込み、サイズ確認、rotation、世代削除、rotation後のactive file再openまでを完了させる。`local_event_store`はforegroundのopen処理で単一writer所有権の競合を即時に返す必要があるため`try_lock_exclusive`を使う一方、local loggerは呼び出し側から分離したwriter thread内で一時的なfile操作を順番に完了させるためblocking lockを使う。
- rotationはrecordを書き込んだ後にactive fileが10 MiBを超えていた場合、次のrecordに備えて行う。1 recordを複数fileへ分割せずJSON行境界を保つため、1ファイルの最大サイズは10 MiBに直近record 1件分を加えた値となる。active fileを含む最大5ファイルの世代上限は厳密に守る。
- loggerはGUIではTauri application setupの冒頭、CLIではsubcommand dispatchより前に登録する。既存のOTLP trace／metrics／crash logs providerとは接続せず、`infrastructure/telemetry/`の送信条件と内容を変更しない。

デプロイ先、署名、Tauri capability、frontend packageは変更しない。

## Alternatives Considered

- standalone／history resumeにも新しいlocal eventを追加する案は採らない。新しいevent schemaとprojection ownerが必要になる一方、本件で必要な障害調査記録は同時に導入するローカルloggerで満たせる。workflowだけは既存のNode failure factが判断材料の正本なので、その経路を維持する。
- `log` facadeを既存のOTLP logs providerへbridgeする案は採らない。ローカルファイル内容を外部送信しないR-008と、既存telemetryの送信内容・条件を変えないNon-goalに反する。
- `tauri-plugin-log`を使う案は採らない。plugin登録も`Builder::split`も`&AppHandle`を要求するのに対し、`main.rs`は引数ありの起動でTauri builderを通らずCLIへ分岐するため、CLIプロセスを構造的に覆えない。GUIだけpluginを使いCLIへ別実装を置く案も、rotation policyと出力先の所有者が二重になるため採らない。
- Terminal Surface側のerror messageをAgentSession gatewayで解析する案は採らない。発生源の文言変更で分類が壊れ、R-001の区別を型で保証できないためである。

## Cross-cutting concerns

- セキュリティ: ローカルloggerはfile出力だけを持ち、OTLP、Webview、callbackへ転送しない。R-002で要求されたworktree pathとR-003で要求された発生源error以外のAgentSession内容やPTY出力を新たに記録しない。
- 性能／保持量: warning／errorだけをbounded channel経由で専用writer threadのfile targetへ送り、呼び出し側をfile I/Oやprocess間lockの待機から分離する。active fileを含む最大5ファイルを保持し、各ファイルは10 MiBに直近record 1件分を加えたサイズまで許容する。同じdata dirを解決して同一ファイルを共有する場合に限り、保持量の上限はGUIとCLIの合計に対して効く。
- 並行性: 同じdata dirを解決したGUIプロセスと複数のCLIプロセスが同じファイルへ書く場合は、writer threadによる書き込みとrotationをprocess間lockで直列化し、rotation中の書き込みで世代保持が壊れないようにする。process内queueが満杯の場合は新しいrecordを捨てるため、高頻度の警告／エラーが呼び出し側の処理速度をfile I/Oへ律速させない。
- 互換性: Tauri commandのerror code／message、history resumeのPaused結果、workflow event schema、SQLite schema、既存OTLP設定を維持する。
- 検証: `docs/architecture/TEST.md`に従い、Terminal Surface usecaseの分類保持、AgentSession gatewayの変換、AgentSession launchの一次原因保持を各レイヤーのRust testで検証する。workflowは既存のfailure factへの記録を複数レイヤー統合testで確認する。ローカルloggerはprocess-level integration testで、GUI経路とCLI経路それぞれのwarning／errorの終了後参照と、rotation後の保持ファイル数を確認する。同じdata dirを共有する複数プロセスの同時書き込みと同時rotationも同じtestに含め、recordが混線せず世代上限が保たれることを確認する。B-010はloggerがfile出力だけを持ちOTLP bridgeを生成しないことを構成testで確認する。

## Risks

- process内queueが満杯になると新しいrecordを取りこぼす。呼び出し側をブロックしないことを優先し、drain再開後のrecordへ破棄件数を含めて欠落を判別可能にする。CLIはcommand dispatchから戻った時点、GUIはTauriの`RunEvent::Exit`受信時に`flush`して受理済みrecordをdrainし、プロセス終了時の記録欠落を防ぐ。
