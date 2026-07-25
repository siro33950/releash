# Behavior

Primary Spec: [requirements.md](requirements.md) / [design.md](design.md)

## B-001: 通常 send の初回受理

GIVEN 送信可能な Session と新しい operation identity がある。
WHEN 利用者が message を送信する。
THEN backend は受理事実を永続化してから Accepted を返し、同じ input を一つの turn または queue item に結び付ける。

## B-002: 応答喪失後の通常 send 再試行

GIVEN send が受理されたが caller が応答を受け取れなかった。
WHEN caller が同じ identity と同じ input で再試行する。
THEN 同じ receipt と現在状態を返し、新しい message、turn、queue item、provider effect を作らない。

## B-003: Restart 後の通常 send 再試行

GIVEN Accepted send の response が失われた後にアプリを再起動した。
WHEN caller が同じ identity を照会または再試行する。
THEN SQLite に保存された同じ operation と receipt を返す。

## B-004: Tauri と WebSocket の並行再試行

GIVEN 同じ authorized caller の同じ send が Tauri と WebSocket から並行して届く。
WHEN 両方が同じ identity と input を使用する。
THEN 一つの operation だけを受理し、両 surface は意味的に同じ結果へ収束する。

## B-005: 新規 Session 作成を伴う通常 send の再試行

GIVEN send が新しい Session の作成を必要とする。
WHEN response loss または並行再試行が起きる。
THEN Session と send の受理は同じ結果へ収束し、Session を重複作成しない。

## B-006: Active turn 中の queued send

GIVEN Session に active turn があり、入力を queue できる。
WHEN 通常 send が受理される。
THEN receipt は queued input を一意に示し、restart 後も同じ queue item として取得できる。

## B-007: 通常 send の保存結果不明

GIVEN send の保存結果を応答だけから確定できない。
WHEN caller が結果を受け取る。
THEN backend は未受理を推測せず同じ operation の結果不明を返し、lookup または same-input retry で解決させる。

## B-008: Accepted 後の実行結果不明と恒久 failure

GIVEN send は Accepted だが provider 実行の結果を確認できない。
WHEN operation を照会する。
THEN receipt を維持した結果確認必要状態または failure を返し、未受理へ戻さず自動再送しない。

## B-009: Operation identity の入力制約

GIVEN 1 byte と 128 bytes の許可文字だけからなる identity、および空、129 bytes、non-ASCII、または `[A-Za-z0-9._:-]` 以外を含む identity がある。
WHEN Tauri と WebSocket の各 surface から通常 send を要求する。
THEN 1 byte と 128 bytes は受理でき、不正な identity は durable state と外部作用を変更せず invalid request として拒否する。

## B-010: Operation payload conflict

GIVEN 既存 operation identity がある。
WHEN 同じ caller が異なる input で再利用する。
THEN payload conflict を返し、既存 operation と provider state を変更しない。

## B-011: 受理後 state 変化を conflict にしない

GIVEN send 受理後に Session や operation status が進んだ。
WHEN original input で再試行する。
THEN 受理時の binding と比較して replay し、現在 state の変化を payload conflict と扱わない。

## B-012: Composer の Accepted clear 境界

GIVEN composer に送信 attempt の本文と添付がある。
WHEN その attempt が Accepted になる。
THEN その attempt に対応する本文と添付だけを clear する。

## B-013: Composer の未受理保持

GIVEN send が未受理、conflict、または結果不明である。
WHEN caller が結果を受け取る。
THEN composer の本文と添付を保持する。

## B-014: 通常 send の受理前 failure

GIVEN validation、authorization、capacity、または保存開始前の failure がある。
WHEN send を要求する。
THEN durable operation を作らず、入力を保持できる受理前 rejection を返す。

## B-015: Permission exact payload の restart 回復

GIVEN permission response が受理され、provider 確認前に crash した。
WHEN アプリが再起動する。
THEN 同じ response intent を安全に readback / reconcile し、blind resend しない。

## B-016: Permission exact payload の欠損

GIVEN provider が正確な permission response を必要とするが、安全な再利用根拠を復元できない。
WHEN recovery を行う。
THEN response を推測せず、再入力または手動解決が必要な状態を表示する。

## B-017: Provider establish と send の依存

GIVEN send に provider session の確立が必要である。
WHEN send が Accepted になる。
THEN establish と send を同じ operation の回復可能な進行として表示し、確立前に provider start 完了を表示しない。

## B-018: Readback できる外部作用直後の crash

GIVEN durable intent 後に外部作用を開始し、結果保存前に crash した。
WHEN provider が結果を authoritative に確認できる。
THEN 同じ identity で readback し、重複作用なしで結果へ収束する。

## B-019: Readback できない外部作用直後の crash

GIVEN 外部作用の結果を authoritative に確認できない。
WHEN crash recovery がその intent を発見する。
THEN 成功または未開始を推測せず、manual reconciliation を要求する。

## B-020: Streaming part の保存 failure

GIVEN streaming 中に canonical persistence が失敗する。
WHEN runtime がその failure を検出する。
THEN live 成功を確定扱いせず、既知の parts を保全して対象 turn を結果確認必要状態へ着地させる。

## B-021: Invalid または stale な effect intent

GIVEN 外部作用を安全に相関・回復できない、または durable acceptance 後に対象 owner が変わった intent がある。
WHEN effect の開始直前に canonical state を再確認する。
THEN effect を開始せず、安全な failure または既存 operation の reconciliation として公開する。

## B-022: Terminal 確定中 crash の原子性

GIVEN turn terminal に複数の state 更新が必要である。
WHEN 確定中に crash する。
THEN 再起動後は全て未確定または全て確定のどちらかになり、partial terminal を公開しない。

## B-023: Terminal 確定後の通知 failure

GIVEN terminal は durable に確定した。
WHEN UI 通知または直後の query が失敗する。
THEN terminal は維持され、reload / direct read で同じ結果を取得できる。

## B-024: Normal completion 後の queue 継続

GIVEN turn が normal completion し、queue が明示的に利用可能である。
WHEN 次 item の実行条件を再検証できる。
THEN 一件だけ開始し、条件を満たさなければ queue を変更せず理由を表示する。

## B-025: Stop または close terminal の queue pause

GIVEN turn が Stop、Session close、quit、failure、または crash で終わる。
WHEN terminal が確定する。
THEN queue を保持したまま pause し、明示 resume まで開始しない。

## B-026: 競合する terminal

GIVEN 同じ turn に複数の terminal candidate が到着する。
WHEN backend が確定を試みる。
THEN 一つの canonical terminal だけが勝ち、他は同じ結果へ収束する。

## B-027: 過去 turn の遅延 event

GIVEN 過去 turn の event が current turn 開始後に到着する。
WHEN event を適用する。
THEN 過去 turn の結果だけへ収束させ、current turn を変更しない。

## B-028: Stop の 10 秒 deadline

GIVEN active turn に Stop を要求する。
WHEN backend または storage が遅延する。
THEN request から 10 秒以内に terminal または同じ Stop identity の結果確認必要状態を返す。

## B-029: Stop 後の stale result

GIVEN Stop 後に古い provider result が到着する。
WHEN terminal を適用する。
THEN canonical winner を変更せず、別 turn や queue を開始しない。

## B-030: Stop request identity の payload conflict

GIVEN Stop request identity が既に target turn に結び付いている。
WHEN 同じ identity を別 target に再利用する。
THEN conflict として拒否し、interrupt を開始しない。

## B-031: Stop capacity の境界

GIVEN 異なる target の未解決 Stop が process 全体で 32 件ある。
WHEN 33 件目の target へ Stop が届く。
THEN 先の 32 件は 10 秒保証を維持し、33 件目は Accepted にせず作用開始前に capacity failure を返す。

## B-032: Stop 受理情報の保存 failure

GIVEN Stop の受理を確定できない。
WHEN interrupt 開始前に failure が判明する。
THEN interrupt を開始せず、turn と queue を変更しない。

## B-033: Accepted Stop 後の terminal 保存 failure

GIVEN Stop は Accepted で interrupt を開始した。
WHEN terminal を保存できない。
THEN 同じ Stop identity を結果確認必要状態に保ち、通常 Idle を表示しない。

## B-034: Stop recovery の一回性

GIVEN Accepted Stop が restart 時に未解決である。
WHEN recovery が開始する。
THEN 同じ Stop identity と既知 observation を使い、interrupt や terminal を重複させない。

## B-035: Startup recovery discovery

GIVEN SQLite に未解決の durable work がある。
WHEN normal startup が store を開く。
THEN 1 page 最大 200 件かつ encoded 4 MiB の recovery inventory と continuation から発見し、Session の個別 open や全履歴 scan を待たない。

## B-036: Recovery crash 境界

GIVEN recovery の途中で再び crash する。
WHEN 次回 startup が同じ work を発見する。
THEN 保存済み進行から再開し、完了済み作用を重複させない。

## B-037: Recovery owner partition

GIVEN 未解決 work が Session、Workflow、closed history、または unowned runtime に属する。
WHEN recovery 一覧を表示する。
THEN 正しい owner surface だけに表示し、owner を推測で変更しない。

## B-038: Public collection の一貫性

GIVEN pending recovery、feedback、shutdown target / history / associated recovery のいずれかが複数あり、取得中にも状態が変わり得る。
WHEN caller が各 public boundary 内の page と continuation を使って collection を取得する。
THEN 同じ collection revision の結果だけを返し、異なる revision を混在させず、一貫した有限 page を返せない場合は partial result のない再取得可能な failure を返す。

## B-039: 未解決 shutdown による new quit の拒否

GIVEN 前回の shutdown に未解決結果が残る。
WHEN 新しい quit を要求する。
THEN 新しい flight や effect を作らず、blocking shutdown と解決操作を返す。

## B-040: Recovery 中の mutation 抑止

GIVEN 対象 resource が結果確認必要状態にある。
WHEN 競合する通常 mutation を要求する。
THEN 受理せず、既存 recovery identity と解決操作を維持する。

## B-041: Meta を読めない failure feedback

GIVEN 対象 Session の通常 read model を構築できない。
WHEN 操作 failure を表示する必要がある。
THEN 別の Rust-owned feedback access から安全な session-scoped failure を取得できる。

## B-042: Failure feedback collection の独立性

GIVEN 一つの Session に複数の未解決 failure がある。
WHEN 一覧を取得する。
THEN 各 failure identity を独立して返し、別 attempt の success で消さない。

## B-043: Failure identity による clear

GIVEN 表示中の failure がある。
WHEN 同じ identity の解決または明示 dismiss が成功する。
THEN 対象一件だけを更新または除去する。

## B-044: Failure feedback capacity

GIVEN process 全体の未解決 feedback が 512 件に達した。
WHEN 新しい failure を生成し得る operation が届く。
THEN 対象 mutation 前に拒否し、既存 feedback の 1 page 最大 32 件の閲覧・dismiss・解決を引き続き許可する。

## B-045: Feedback revision conflict

GIVEN caller が古い feedback revision を使う。
WHEN dismiss または解決を要求する。
THEN 現表示を変更せず revision conflict を返す。

## B-046: Feedback 表示上限

GIVEN failure の label が UTF-8 160 bytes、または detail が 2048 bytes を超える。
WHEN public feedback を構築する。
THEN 各上限内へ安全に省略して省略 marker を表示し、secret、path、SQL、provider raw payload を露出しない。

## B-047: Production runtime event golden

GIVEN `src-tauri/src/infrastructure/agent_session/fixtures/{claude,codex}/normal_turn/` の `wire.jsonl`、`convert.golden`、`read_model.golden` がある。
WHEN 各 wire fixture を production adapter、usecase、SQLite persistence、reopen projection へ通す。
THEN provider conversion と live / reopen read model が各 golden に一致し、fixture ごとの canonical terminal は一件だけになる。

## B-048: Wire 互換と projection 互換の独立検出

GIVEN B-047 の `wire.jsonl`、`canonical_events.json`、二つの golden がある。
WHEN provider conversion だけを検証する suite と、canonical event から projection だけを検証する suite を別々に実行する。
THEN 一方の mapping だけを変えた場合は対応する suite が独立して失敗し、production-path suite は両方の退行を検出する。

## B-049: Hermetic F1b

GIVEN repository CI で B-047 / B-048 の checked-in fixture を検証する。
WHEN Claude / Codex の各 suite を実行する。
THEN 実 provider process、CLI、network、credential を使用せず同じ golden result を得る。

## B-050: 恒久 SQLite mutation の atomicity

GIVEN 一つの operation が複数の domain state を変更する。
WHEN persistence が成功、失敗、または結果不明になる。
THEN 全参加者が同じ結果へ収束し、partial public state を返さない。

## B-051: Close / quit decision table

GIVEN 各 close / quit surface がある。
WHEN 同じ Session / application state で操作する。
THEN [close-quit-decision-table.md](../../../specs/milestone-84-agent-chat-stabilization/close-quit-decision-table.md) の該当行どおりに観測できる。

## B-052: View close の意味論

GIVEN chat、workflow、workspace、window の view が開いている。
WHEN view close を行う。
THEN 表示だけを閉じ、turn、permission、queue、runtime、workflow、shutdown を変更しない。

## B-053: Active normal Session close と open archive

GIVEN active turn の Session がある。
WHEN normal close または open archive を受理する。
THEN final parts、SessionClosed terminal、permission settlement、queue pause、Closed / Archived を一つの outcome として表示する。

## B-054: Idle close と archive

GIVEN Idle Session がある。
WHEN close または archive を受理する。
THEN synthetic turn terminal を追加せず、Session state と queue pause だけを確定する。

## B-055: Backend switch

GIVEN Session が Idle で未解決 permission / recovery / effect がない。
WHEN backend switch を受理する。
THEN old runtime の結果を確認してから new backend を effective にし、結果不明では old backend と queue pause を維持する。

## B-056: Close 系 command の 10 秒結果

GIVEN Session lifecycle command が Accepted である。
WHEN runtime または storage が遅延する。
THEN request から 10 秒以内に完了または同じ operation identity の結果確認必要状態を返す。

## B-057: Graceful quit surface の single flight

GIVEN 複数の graceful quit surface が同時に要求される。
WHEN 最初の request が Accepted になる。
THEN 一つの flight へ join し、最初の intent と deadline を変更しない。

## B-058: Quit request identity の payload conflict

GIVEN quit request identity が既存 intent に結び付いている。
WHEN 同じ identity を異なる exit / restart intent に再利用する。
THEN conflict として拒否し、既存 flight を変更しない。

## B-059: Shutdown admission

GIVEN application quit が Accepted になる。
WHEN shutdown が進行する。
THEN 新しい通常 mutation を受理せず、先に受理した operation を安全な outcome へ着地させる。

## B-060: Shutdown target 上限

GIVEN open Session と running Workflow の shutdown target が合計 4096 件または 4097 件ある。
WHEN quit を要求する。
THEN 4096 件は一つの flight として受理でき、4097 件は effect を開始せず capacity failure で安全に abort する。

## B-061: Previous shutdown の未解決状態

GIVEN previous shutdown の未解決結果がある。
WHEN new quit を要求する。
THEN 理由を示して新 flight を作らず、既存 result を解決または保持する。

## B-062: Shutdown summary と detail の一貫性

GIVEN shutdown に複数 target がある。
WHEN current summary、history summary、target detail、または continuation page を読む。
THEN 同じ committed shutdown identity と revision に属する plan と ordered target だけから一貫した結果を返し、target page は最大 128 件かつ response envelope を除く encoded entry 合計 1 MiB 以内になる。
AND page 取得中に revision が変わった場合は異なる revision の target を混ぜず、partial result のない revision conflict を返す。
AND 旧 page file、page reference、root hash、root page、または current recovery collection の有無や内容を summary / detail / effect 可否の代替根拠にしない。

## B-063: Shutdown activation 前 failure の abort

GIVEN shutdown effect がまだ開始されていない。
WHEN 準備または開始可否の確定が安全に失敗する。
THEN 外部作用を開始せず abort し、通常利用を再開できる結果を表示する。

## B-064: Shutdown activation 後の bounded exit

GIVEN shutdown effect を開始した、または開始結果を確認できない。
WHEN quit request から 15 秒が経過する。
THEN abort せず、未完了 identity を残して指定 intent で exit または restart する。

## B-065: Durable plan activation の結果不明

GIVEN shutdown 開始の保存結果を確認できない。
WHEN deadline に達する。
THEN 未開始を推測して別 command を始めず、同じ shutdown identity を結果確認必要状態にする。

## B-066: Process exit に伴う暗黙作用

GIVEN process exit が child process や pipe に影響し得る。
WHEN 明示 command の結果を確認できないまま終了する。
THEN 成功または無作用を推測せず、restart 後の readback 対象にする。

## B-067: Shutdown 遅延結果の fence

GIVEN 旧 shutdown の遅延結果が new flight 後に届く。
WHEN 結果を適用する。
THEN 元 flight だけへ収束させ、new flight、Session、Workflow を変更しない。

## B-068: 履歴件数に依存しない bounded operation

GIVEN 未解決件数を固定し、無関係な Session または event history が 10 件と 1,000,000 件の fixture がある。
WHEN operation / terminal identity lookup と各 collection の first page を同一 release 環境で各 1,000 sample 実行する。
THEN identity、result、page、continuation は一致し、大規模 fixture の p95 は小規模 fixture の 1.25 倍以下、pending recovery first 200 は p95 50 ms 以下、identity lookup は p95 20 ms / p99 50 ms 以下になる。

## B-069: Shutdown query の bounded failure

GIVEN 一貫した shutdown view を有限時間で構築できない。
WHEN current または history query が 2 秒に達する。
THEN partial result を返さず、安全な busy または deadline failure を返す。

## B-070: SQLite-only startup と旧 file-store 非参照

GIVEN `sessions/`、`session_titles.json`、`workflow_runs/`、`workflow_logs/`、`workflow_execution_logs/`、`workflow_executions/`、`workflow_event_logs/` に不正 byte と変更検出 sentinel を置き、filesystem operation を path 単位で記録できる app-data がある。
WHEN production composition で cold startup、通常 send / query、idle maintenance、background GC、retention、cleanup、graceful shutdown、restart を順に実行する。
THEN 各legacy path自身とその配下に対するopen、metadata / stat、read_dir、read、write、rename、remove、decode、およびlegacy entryを列挙し得るapp-data rootへのread_dirを0件とし、sentinelのbyte、metadata、directory entryを全て維持する。
AND import、変換、merge、fallback、dual write を示す operation、state、progress、query、API、gate、checkpoint、parity、cutover、特殊 quit を作らない。
AND 固定 SQLite store の通常 schema evolution、configuration migration、watch subscription initialization は別の入力と test anchor で引き続き実行できる。

## B-071: SQLite startup failure と初回作成再開

GIVEN 固定 SQLite path と initial-create evidence が次のいずれかである。

| Case | Fixed SQLite path | Initial-create evidence |
| --- | --- | --- |
| A | absent | absent、またはpartial / invalid |
| B | absent、0 byte、または application table のない検証可能な SQLite | valid かつ未完了 |
| C | application identity、ready metadata、schema が全て検証可能 | 任意。存在する evidence は Ready 前に除去する |
| D | B / C に該当しない既存 file。0 byte、または application store と識別できない未初期化 / 別用途 SQLite / 非 SQLite | 任意 |
| E | application store と識別できるが、integrity、schema、metadata、key のいずれかを検証不能 | 任意 |

WHEN production startup を一回実行する。
THEN A はdatabaseがabsentであることを再確認してpartial / invalid evidenceだけを作り直し、evidenceをdurability順に確定して同じfixed pathを初期化する。Bは同じpathの初回作成だけを安全に再試行し、Cは同じinstallation identityとkeyを保持してReadyになる。
AND D は `InitializationStateInvalid`、E の未対応schemaは`UnsupportedStoreVersion`、E の既知schemaに対するmetadata / key / integrity不整合は`StoreValidationFailed`を返す。
AND writer lock失敗は`StoreInUse`、SQLite runtime不足は`UnsupportedRuntime`、filesystem / permission / capacity failureは`StorageUnavailable`、supported schema stepのtransactionまたはreadback failureは`SchemaEvolutionFailed`を返し、既存SQLite fileを置換、削除、truncate、再初期化しない。
AND failure は safe description、correlation、再起動時の扱い、利用可能 action `Quit` だけを返し、raw path、SQL、database error、Session、Workflow、normal command、durable quit / shutdown progress を公開または作成しない。
AND startup用二command以外のTauri commandはdomain stateを解決する前に同じ`ApplicationUnavailable`として拒否され、WebSocket serverはlistenせず、空のnormal read modelへfallbackしない。
AND Quit は SQLite を開き直さず request ingress から15秒以内にprocess-local effectを一度だけ開始し、同じprocess内の重複要求は二重effectを作らない。
AND writer lock は待たず、SQLite busy wait は最大 2 秒、create / open / evolution / validation の自動再試行は 0 回である。

## B-072: Tauri と WebSocket の surface 一致

GIVEN 同じ authorized operation と state がある。
WHEN Tauri と WebSocket から同じ command / query を使う。
THEN transport 差を除き同じ receipt、status、failure、action semantics を返す。

## B-073: WebSocket 認証と resource 上限

GIVEN loopback local API に 16 connections、1 connection 32 in-flight、60 requests/s・burst 120、request / response 16 MiB、outbound 32 responses / 16 MiB の各境界と、未認証・権限外の request がある。
WHEN 各上限内と一件超過を処理する。
THEN 上限内だけを処理し、未認証・権限外・超過は state と effect を変更せず安全に拒否して他 request の進行を阻害しない。

## B-074: WebSocket request identity と切断

GIVEN WebSocket command が Accepted 後に接続が切れる。
WHEN 再接続して同じ operation identity を照会する。
THEN 接続 identity ではなく durable operation identity から同じ結果を返す。

## B-075: 公開整数 field の lossless 境界

GIVEN `0`、`1`、`9223372036854775807`、負数、先頭ゼロ、正符号、指数表記、空白、`9223372036854775808`、JSON number を semantic integer field へ入力する。
WHEN Tauri / WebSocket を round-trip する。
THEN field が許す 0 と正数は canonical ASCII decimal string の同じ値を返し、範囲外または lossless でない表現は state 変更前に拒否する。

## B-076: Current application shutdown の error 境界

GIVEN shutdown が存在しない、same-boot で進行中、previous-boot で未完了、previous-boot で完了、最初の受理結果不明、または authority を安全に読めない状態のいずれかである。
WHEN current shutdown を照会する。
THEN それぞれ no current、current status、同じ shutdown の needs resolution、no current かつ history から取得可能、同じ quit の結果不明、安全な internal failure を返し、別 shutdown や成功へ fallback しない。

## B-077: 完了済み契約の非退行

GIVEN 次の compatibility baseline と checked-in fixture / test anchor がある。
WHEN #1499 の production composition を通して同じ入力を実行する。
THEN 各行の public result を維持し、message、terminal、queue item、Notice、external effect を重複させず、旧 physical model を再導入しない。
AND `issue_1499_d1_contract_is_not_redefined` で D1 #1445 の configuration / Goal / capability と frontend action enablementを再定義しないことを検査する。

| Baseline | 現行正本 | Existing anchor | Expected public result |
| --- | --- | --- | --- |
| F1 #1383 | B-047〜B-049、V-D9 / V-D12 | `fixtures/{claude,codex}/normal_turn/`、`b047_*_wire_converter_runtime_sqlite_reopen_matches_read_model_golden` | convert / read-model golden と一致し、terminal は一件 |
| L1 #1402 / L2 #1403 | I4〜I6、B-006 / B-012〜B-014 / B-028〜B-034 | `production_interrupt_watchdog_finalizes_at_the_ten_second_boundary`、`queue_pause_and_explicit_resume_survive_runtime_state_restart`、`MessageInput.test.tsx` | Stop は 10 秒境界へ収束し queue は pause、Accepted attempt だけを clear |
| L4 #1405 / L6 #1407 / L8 #1409 / S10a #1398 | I1〜I3、I7〜I9、I12 | `close_session_finalizes_streaming_turn_and_persists_terminal_projection`、`completed_recovery_notice_is_restored_once_before_the_next_turn_after_restart`、`crash_emits_projected_error_snapshot_before_state_change_and_matches_reload` | terminal / permission / queue は一つの outcome、recovery Notice は一回、failure は live / reload で一致 |
| L7 #1408 / L10 #1411 | I10 / I13 | `mixed_stdout_{claude,codex}.jsonl`、`repeated_session_runtime_locks_do_not_accumulate_registry_entries`、`session_runtime_lock_reentry_is_detected_in_tests` | malformed / oversize 後も valid event を処理し、lock entry は蓄積せず reentry だけを拒否 |
| D1 #1445 / P2 #1414 / X1 #1417 | I14〜I16、P4 / P5、R-022 | `BoundSessionChat.test.tsx`、`claude/wire.rs::test_claude_user_message画像のみなら空text_blockを含めない` | configuration / Goal 境界を再定義せず、feedback は発生元だけに残り、image-only input の意味を維持 |

## B-078: Stop winner の解決結果

GIVEN Stop が canonical terminal の winner である。
WHEN operation を照会する。
THEN 同じ Stop receipt と stopped result を返し、queue は pause のままにする。

## B-079: Stop superseded の解決結果

GIVEN normal completion または別 terminal が Stop より先に確定した。
WHEN Stop operation を照会する。
THEN 重複 terminal を作らず、既存 canonical outcome により Stop が解決済みであることを返す。

## B-080: Terminal 保存 failure と Stop capacity

GIVEN Accepted Stop の terminal が未確定である。
WHEN 別 Stop の capacity を判定する。
THEN 未解決 Stop を引き続き capacity に含め、保存 failure を slot 解放とみなさない。

## B-081: Pending recovery の safe action set

GIVEN pending recovery がある。
WHEN 利用可能な action を表示する。
THEN [agent-chat-ideal-vocabulary.md](../../../specs/milestone-84-agent-chat-stabilization/agent-chat-ideal-vocabulary.md) が定める safe action のうち Rust が現在安全と判断したものだけを返し、frontend が generic retry を追加しない。

## B-082: Recovery action の response 喪失と restart

GIVEN recovery action が受理されたが response が失われた。
WHEN 同じ action identity を再試行または restart 後に照会する。
THEN 同じ attempt と保存済み result へ収束する。

## B-083: Recovery action の invalid identity と stale view

GIVEN action identity が不正、未知、権限外、または対象 revision が古い。
WHEN 解決を要求する。
THEN resource と effect を変更せず安全な rejection を返す。

## B-084: Recovery action classification の組合せ

GIVEN recovery action の観測と安全な実行結果が、成功、effect 未開始、曖昧、開始前取消、または変更なしのいずれかである。
WHEN recovery action を完了する。
THEN vocabulary が定める Succeeded、Confirmed no effect、Ambiguous、Cancelled before effect、Unchanged のいずれかと、対応する owner state を一つの canonical outcome として確定する。

## B-085: Shutdown target action と plan terminal

GIVEN 最後の未解決 shutdown target が安全に解決される。
WHEN target action を確定する。
THEN target result と shutdown summary / terminal を同じ outcome として公開し、partial completion を作らない。

## B-086: Recovery action の保存結果不明

GIVEN recovery action の保存結果を確認できない。
WHEN caller が結果を受け取る。
THEN 別 action を開始せず、同じ action identity の結果不明として解決させる。

## B-087: Stop と quit の request identity 境界

GIVEN Stop と quit が同じ文字列の caller request identity を使用する。
WHEN 両 command を処理する。
THEN command kind の異なる identity scope として扱い、相互に replay / conflict させない。

## B-088: Known quit operation の読取境界

GIVEN normal shutdown の known quit operation がある。
WHEN operation identity で照会する。
THEN その operation の保存済み shutdown result だけを返し、current shutdown や startup failure へ fallback しない。

## B-089: Shutdown が記録した recovery の照会境界

GIVEN shutdown が開始時点の recovery collection を記録している。
WHEN historical shutdown detail を照会する。
THEN current recovery collection と混ぜず、その shutdown が記録した意味を返す。

## B-090: Current recovery の shutdown association filter

GIVEN current pending recovery に shutdown と無関係な work もある。
WHEN 特定 shutdown の current target recovery を照会する。
THEN その shutdown に結び付く work だけを返す。

## B-091: 別 request identity の quit intent join

GIVEN current quit flight がある。
WHEN 別 request identity から異なる intent が届く。
THEN first accepted intent の flight へ join し、intent と deadline を変更しない。

## B-092: RetryQuit の提示条件

GIVEN quit が effect 開始前に安全に失敗した。
WHEN backend が normal admission と保存状態の安全性を確認できる。
THEN その場合だけ Retry Quit を提示し、開始結果不明または開始後には提示しない。

## B-093: Completed recovery action の完全 replay

GIVEN recovery action は Completed である。
WHEN 同じ identity を再び照会する。
THEN current resource から再構築せず、保存済み receipt と safe result を返す。

## B-094: Feedback resolution retry の再失敗

GIVEN feedback の解決 retry が再び失敗する。
WHEN 結果を表示する。
THEN 同じ feedback identity を新しい revision と failure へ更新し、重複 entry を作らない。

## B-095: Session close の crash 境界

GIVEN Session lifecycle operation の途中で crash する。
WHEN 再起動後に operation を照会する。
THEN 同じ receipt と Session outcome または結果確認必要状態を返し、Session を自動 reopen しない。

## B-096: Shutdown history detail の可用性

GIVEN full detail を持つ terminal shutdown が 2 件あり、3 件目を保持する。
WHEN history と各 exact detail を照会する。
THEN oldest は同じ identity、intent、terminal status、counts、deadline、safe failure を持つ summary-only へ一貫して切り替わり、古い detail や current collection と混在しない。

## B-097: Previous shutdown cleanup 中の new quit

GIVEN previous shutdown の保存済み結果の更新が new flight と安全に共存できない。
WHEN new quit を要求する。
THEN 作用開始前に待機理由を返し、既存結果を破壊せず new flight を作らない。

## B-098: SQLite schema evolution の atomicity

GIVEN 対応可能な旧 SQLite schema がある。
WHEN normal admission 前に schema evolution を行い crash または response loss が起きる。
THEN 再起動後は旧または新の検証可能な schema へ収束し、既存 operation、terminal、recovery、shutdown の意味と、同じ installation identity、cursor HMAC key、operation-binding HMAC key を維持する。
AND 各 supported version step は一回の SQLite transaction 内で適用し、未対応 version、step failure、commit 結果不明では検証できるまで normal workbench を開かず safe startup failure にする。
AND schema evolution は legacy path、initial-create evidence の progress、別 database、authority pointer、generation directory、legacy migration state を作成または参照しない。

## B-099: 通常 send operation の principal 分離

GIVEN 別 principal が既存 send operation identity を推測する。
WHEN query または replay を要求する。
THEN operation の存在を開示せず state と effect を変更しない。

## B-100: Quit の最初の受理結果不明

GIVEN 最初の quit を受理する保存結果が不明である。
WHEN caller が再試行する。
THEN 別 shutdown flight を作らず、同じ request の受理結果を解決する。

## B-101: Session lifecycle operation の replay と principal 分離

GIVEN Session lifecycle operation が Accepted である。
WHEN response loss、restart、または別 principal からの照会が起きる。
THEN authorized replay には同じ receipt を返し、別 principal には存在を開示しない。

## B-102: Session lifecycle operation の conflict と join

GIVEN 同じ Session に未解決 lifecycle operation がある。
WHEN 同じ action の別 request または異なる action が届く。
THEN 同じ action は既存 operation へ join し、異なる action は新 effect なしで拒否する。

## B-103: Session lifecycle operation の 10 秒結果と stable query

GIVEN lifecycle operation が 10 秒以内に external result を確認できない。
WHEN deadline 後または restart 後に照会する。
THEN 同じ operation identity の結果確認必要状態を返し、current Session から別 result を合成しない。

## B-104: Message content の永続化・公開互換

GIVEN 全ての supported message content と、未知の必須 semantics がある。
WHEN 保存、reopen、public projection を行う。
THEN supported content は意味を失わず round-trip し、未知の必須 semantics は別の意味へ推測せず safe incompatibility にする。

## 要件 ID と検証方法の対応表

| Requirement ID | Behavior ID | Verification Method |
| --- | --- | --- |
| R-001 | B-001〜B-009、B-074、B-099 | 同じ send の response loss、並行要求、切断、restart、別 principal を入力し、authorized callerには一つの receipt / turn、別 principal には情報非開示となることを観測する |
| R-002 | B-010〜B-011 | 同じ identity へ同一入力と異なる入力を再送し、前者だけが replay、後者は既存 state・effect 0件の conflict となることを観測する |
| R-003 | B-012〜B-014 | Accepted、受理前 rejection、conflict、結果不明を composer へ返し、Accepted attemptだけが clear されることを観測する |
| R-004 | B-015〜B-021 | permission、provider start、readback可能・不能、stale ownerを入力し、durable intentと開始直前確認なしのeffectが0件であることを観測する |
| R-005 | B-022、B-050、B-095 | terminal、複数state mutation、Session closeの各commit境界でcrashさせ、変更前または全確定後だけが公開されることを観測する |
| R-006 | B-023〜B-025 | terminal確定後の通知失敗、normal completion、Stop / closeを入力し、terminal維持と規定どおりのqueue継続 / pauseを観測する |
| R-007 | B-026〜B-027 | 同じturnの競合terminalと過去turnの遅延eventを入力し、canonical terminal一件とcurrent turn不変を観測する |
| R-008 | B-028〜B-032 | active turnへのStop、response loss、別target conflict、capacity超過、受理保存失敗を入力し、requestから10秒以内の規定結果と重複effect 0件を観測する |
| R-009 | B-033〜B-034、B-078〜B-080 | Accepted Stop後のterminal保存失敗とrestartを入力し、同じStop identity、queue pause、capacity占有、canonical resolutionを観測する |
| R-010 | B-035〜B-040、B-081〜B-090、B-093 | 未解決workをowner別に用意し、startup discovery、page継続、action response loss、shutdown associationを通して同じidentityと保存済みresultを観測する |
| R-011 | B-041〜B-046、B-094 | meta read failure、別Session成功、stale revision、capacity到達、retry再失敗を入力し、session-scoped feedbackの独立性と安全な更新を観測する |
| R-012 | B-047〜B-049、B-072〜B-075 | Claude / Codex fixtureをproduction経路と両transportへ通し、同じpublic semantics、未認証・limit超過のeffect 0拒否、公開整数のlossless往復を観測する |
| R-013 | B-050、B-062、B-070、B-098 | SQLite mutation、shutdown summary / ordered target read、旧file-store併存、schema evolutionの各crash境界で、単一authority、atomic outcome、同じrevisionのpublic read、旧file非参照を観測する |
| R-014 | B-051〜B-056、B-095、B-101〜B-103 | view close、active / Idle close、archive、backend switch、response loss、restartを入力し、surface別結果とrequestから10秒以内のstable operationを観測する |
| R-015 | B-057〜B-067、B-071、B-076、B-085、B-087〜B-093、B-096〜B-097、B-100 | 全graceful surface、startup failure、競合intent、previous shutdown、target action、historyを入力し、normal時は一つのfirst-intent flight、startup failure時はdurable stateなしのprocess-local Quitとなることを観測する |
| R-016 | B-063〜B-067 | effect開始前failure、開始後のslow target、開始結果不明、process exitを入力し、最初のrequestから15秒以内のabortまたはrecovery付きexit / restartを観測する |
| R-017 | B-035、B-038、B-042、B-062、B-068〜B-069、B-089、B-096 | recovery、feedback、shutdown target / history / associated recoveryと異なる無関係履歴量を入力し、同じ有限page / continuation / direct identity resultまたはpartial resultなしのdeadline failureを観測する |
| R-018 | B-070〜B-071、B-098 | SQLite absent、evidence付き初回作成中断、evidenceなし空file、正常、対応可能 / 未対応schema、検証不能、旧file-store sentinelを入力し、closedなstartup分類、Ready前のeffect 0件、Quit一件、各legacy path access 0件とbyte / metadata不変を観測する |
| R-019 | B-077 | milestone 84正本の代表fixtureをproduction compositionへ通し、send、terminal、permission、recovery、close / quit、live / reloadの公開結果が維持されることを観測する |
| R-020 | B-078〜B-080 | Stop、normal completion、failure、closeのterminal競合と保存失敗を入力し、terminal一件、Stop resolution、queue pause、capacity不変を観測する |
| R-021 | B-081〜B-086、B-092〜B-094 | safe action、stale action、response loss、ambiguous readback、最後のshutdown targetを入力し、提示済みactionだけが同じidentityのcanonical resultへ収束することを観測する |
| R-022 | B-104 | supported message contentと未知の必須semanticsを保存・reopen・両transportへ通し、前者のlossless round-tripと後者のsafe incompatibilityを観測する |
