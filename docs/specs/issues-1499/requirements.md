# Context

Issue #1499 は、milestone 84「Agent チャット安定化」で確定した send、terminal、Stop、recovery、close、quit の保証を、一つの恒久 local event store 上で成立させるための Primary Spec である。

現状は、同じ操作の受理事実、実行状態、回復状態、shutdown 状態が複数の保存経路と一時状態に分かれ、response loss、crash、restart、並行操作の境界で一意な結果を返せない。また、旧 file-store の物理設計を前提にした移行語彙が現行契約へ残り、恒久 SQLite store の責務と競合している。

本 Issue は、固定 path の SQLite store を直接 create / open し、正常稼働時の唯一の read / write authority とする。変更前の file-store data は互換対象にせず、アプリケーションの production lifecycle 全体で探索、参照、変換、変更しない。

本 Primary Spec の受入条件と実装方針は次を正本とする。

- [behavior.md](behavior.md)
- [design.md](design.md)

利用者可視の語彙、lifecycle、presentation、close / quit の横断契約と、後続 Issue への routing は次を正本とする。

- [agent-chat-ideal-vocabulary.md](../../../specs/milestone-84-agent-chat-stabilization/agent-chat-ideal-vocabulary.md)
- [agent-chat-ideal-lifecycle.md](../../../specs/milestone-84-agent-chat-stabilization/agent-chat-ideal-lifecycle.md)
- [agent-chat-ideal-presentation.md](../../../specs/milestone-84-agent-chat-stabilization/agent-chat-ideal-presentation.md)
- [close-quit-decision-table.md](../../../specs/milestone-84-agent-chat-stabilization/close-quit-decision-table.md)
- [agent-chat-instability-audit.md](../../../specs/milestone-84-agent-chat-stabilization/agent-chat-instability-audit.md)
- [phase-plan.md](../../../specs/milestone-84-agent-chat-stabilization/phase-plan.md)

# Outcome

Releash の利用者は、同じ操作について response loss や restart を跨いでも同じ受理事実と結果を確認できる。未完了の作用は同じ identity で安全に監督・解決でき、「未受理」「受理済みで進行中」「結果確認が必要」「完了」を区別できる。

永続化の正本は固定 path の SQLite store 一つになる。通常の SQLite schema evolution は normal admission 前に完了し、成功した場合だけ workbench を利用できる。起動に失敗した場合は Rust-owned の安全な failure surface と終了操作だけを提供する。

# Current Behavior

実装 commit `69b81d34953e8303efbd04e97258c59bda8f2dfe` の source と checked-in test を照合した。外部 provider は実行していない。元の監査所見は [agent-chat-instability-audit.md](../../../specs/milestone-84-agent-chat-stabilization/agent-chat-instability-audit.md) を正本とし、現在の実装が本 Spec と食い違う最小再現だけを次に示す。

| 最小再現 | 現在の実際の結果 | 根拠 |
| --- | --- | --- |
| 変更前の file-store がある app-data で起動する | SQLite を直接開かず旧 data を検出して import し、完了まで normal admission を migration 状態で制御する | `src-tauri/src/adaptor/gateway/local_event_store/store.rs::LocalEventStore::open`、`src-tauri/src/adaptor/gateway/local_event_store/migration.rs::import_legacy` |
| 新しい app-data で初回起動する | 固定 SQLite file 一つではなく、生成した database file と別の authority file を順に公開する | `src-tauri/src/adaptor/gateway/local_event_store/store.rs::LocalEventStore::open`、`src-tauri/src/adaptor/gateway/local_event_store/authority.rs::StoreLayout` |
| Tauri または WebSocket から startup state を読む | legacy migration の進捗を public result として返す command / route が存在する | `src-tauri/src/adaptor/protocol/application_lifecycle_v1.rs::LocalStoreMigrationResultDtoV1`、`src-tauri/src/adaptor/controller/command/application_lifecycle.rs::get_local_store_migration` |
| migration 中に application quit を要求する | normal application quit とは別の migration quit flight と projection を作る | `src-tauri/src/usecase/shutdown_coordinator.rs::MigrationQuitFlightView`、`src-tauri/src/adaptor/protocol/application_lifecycle_v1.rs::MigrationApplicationQuitProjectionDtoV1` |
| migration / cutover の crash test を実行する | migration checkpoint、parity、cutover authority の存続を正解として検証する | `src-tauri/src/adaptor/gateway/local_event_store/tests.rs::b098_projection_parity_failure_keeps_legacy_authority_and_reports_migration_blocked` |

# Scope / Non-goals

Scope は次のとおり。

- 通常 send の再試行可能な受理契約と composer の clear 境界
- terminal、Stop、recovery、Session lifecycle、application quit の一回性と crash / restart 結果
- 未解決作用と操作 failure の、安全で session-scoped な表示・解決
- domain-owned event と一つの canonical `MessagePart`
- Rust-owned state authority と、Tauri / WebSocket に共通する意味論
- 固定 path の恒久 SQLite store、通常の SQLite schema evolution、safe startup failure
- production startup、background maintenance、retention、cleanup、shutdown を含む旧 file-store 非参照保証
- history size に依存しない identity lookup と bounded collection access

Non-goals は次のとおり。

- 変更前の file-store data の migration、compatibility read、import、merge、fallback、dual write、変更、削除
- legacy-data migration 用の state、progress、query、API、gate、checkpoint、parity、cutover、特殊 quit、または別名の同等機構
- managed backup / restore、export / import、app-data reset、privacy purge
- active-turn steer 全体、queue lifecycle 全体、provider wire 全体の typed adapter 化、runtime 全体の module 分解
- hard kill、power loss、OS による即時強制終了そのものの阻止
- Phase 0、F2、F3、D3 など計画上の名称を runtime module、type、table、physical store identity に使用すること

# Requirements

次の値は外部から観測できる boundary の単一正本である。内部 queue、table、cursor 表現、処理手順は規定しない。

| Boundary | Public contract |
| --- | --- |
| Caller request identity | UTF-8 1〜128 bytes、許可文字は `[A-Za-z0-9._:-]` |
| Stop | request ingress から 10 秒、異なる未解決 target は process 全体で最大 32 件 |
| Pending recovery | 1 page は最大 200 件かつ encoded 4 MiB |
| Session feedback | 1 page は最大 32 件、未解決総数は process 全体で最大 512 件、label は UTF-8 160 bytes、detail は 2048 bytes |
| Session lifecycle | request ingress から 10 秒 |
| Application quit | 最初の request ingress から 15 秒、target は最大 4096 件 |
| Shutdown detail | target page は最大 128 件で、response envelope を除く encoded target entry の合計は 1 MiB、1 target entry も encoded 1 MiB。full detail を保持する terminal shutdown は最大 2 件、関連 recovery page は最大 200 件かつ encoded 4 MiB |
| Shutdown query | 一貫した snapshot を 2 秒以内に返せなければ partial result なしの deadline failure |
| Startup attempt | 固定 SQLite path とその writer lock / initial-create evidence だけを対象に一回の create / open / schema evolution / validation を行う。writer lock は待たず、SQLite busy wait は最大 2 秒、同一 process 内の自動再試行は 0 回 |
| History-independent read | 無関係な履歴 10 件と 1,000,000 件を各 1,000 sample 比較し、大規模 fixture の p95 は小規模 fixture の 1.25 倍以下。pending recovery first 200 は p95 50 ms 以下、identity lookup は p95 20 ms / p99 50 ms 以下 |
| WebSocket | loopback Bearer 認証、1 process 16 connections、1 connection 32 in-flight、60 requests/s・burst 120、request / response 16 MiB、outbound 32 responses / 16 MiB |
| Semantic integer | `0` または先頭ゼロのない ASCII decimal string で `9223372036854775807` まで。JSON number、負数、正符号、指数表記、空白、範囲超過は拒否する。page / byte limit は JSON 非負整数、exit code は JSON signed integer |

- **R-001**: 通常 send は public boundary に従う caller 保持の stable operation identity を持ち、同じ authorized caller、identity、意味的に同じ入力の再試行は、response loss、並行要求、restart を跨いでも同じ受理事実、同じ入力、同じ turn または queue item へ収束する。
- **R-002**: 同じ operation identity を異なる入力へ再利用した要求は、既存操作を変更せず、provider effect を開始せず、payload conflict として拒否する。
- **R-003**: composer は send が durable に受理された場合だけ、その送信 attempt に対応する本文と添付を clear する。未受理、conflict、結果不明では保持し、受理後の実行 failure を新規 send failure として再送しない。
- **R-004**: provider、runtime、workflow への外部作用は、その作用を同じ identity で追跡・回復できる durable intent が確定し、開始直前にも対象 owner と intent が有効だと確認できた場合だけ開始する。作用結果を一意に確認できない場合は成功または未開始を推測せず、同じ identity の reconciliation として公開する。
- **R-005**: 一つの利用者操作として不可分な event、state、receipt、terminal、recovery 参加者は、全て確定するか全て未変更でなければならない。結果確認不能時は同じ操作を解決し、部分成功を公開しない。
- **R-006**: terminal 確定後の通知や readback failure は terminal を未確定へ戻さない。通常完了だけが許可された次の queue item を開始でき、Stop、close、quit、failure、crash は queue を pause する。
- **R-007**: 一つの turn に競合する terminal が到着しても canonical terminal は一つだけであり、遅延 event は別 turn または別 operation の状態を変更しない。
- **R-008**: Stop は backend に依存せず、request から 10 秒以内に terminal または同じ Stop identity の結果確認必要状態へ到達する。重複 Stop は同じ進捗へ join し、保証可能な capacity を超える要求は作用開始前に拒否する。
- **R-009**: Accepted Stop の terminal を保存できない場合、対象 turn を通常 Idle と扱わず、queue を再開せず、restart 後も同じ Stop identity と既知の観測結果から回復する。
- **R-010**: 未解決の durable work は startup で発見でき、owner、status、safe observation、利用可能な解決操作を public boundary に従う bounded collection と direct identity lookup で取得できる。回復の response loss と restart は同じ action result へ収束する。
- **R-011**: 操作 failure は対象 session に安全な文言で表示され、別 session の成功や古い attempt で消えない。明示 dismiss または同じ failure identity を解決した結果だけが表示を更新し、public boundary の capacity 到達時にも既存 failure の閲覧・解決手段を失わない。
- **R-012**: Claude / Codex の代表 input は production composition を通して既存の公開 event、read model、terminal semantics を維持する。provider 入力から domain への互換性と、domain から public surface への互換性を独立して検証できる。Tauri / WebSocket は同じ operation に同じ意味を返し、未認証・権限外・公開 resource limit 超過を作用開始前に拒否し、公開整数を lossless に往復する。
- **R-013**: 固定 path の bundled SQLite store は正常稼働時の唯一の persistence authority である。domain event と操作状態は同じ atomic persistence boundary で確定し、commit 結果不明は同じ identity で解決する。public read は commit 済みの一貫した state だけを返す。
- **R-014**: view close、Session close、open / closed archive、backend switch は異なる操作である。view close は表示だけを閉じる。Session lifecycle 操作は同じ request の replay と conflict を区別し、10 秒以内に完了または同じ operation identity の結果確認必要状態へ到達する。
- **R-015**: Cmd-Q、menu、Dock、tray、native cooperative exit、programmatic exit / restart は一つの application quit operation へ収束する。最初に受理した intent が flight を所有し、全 surface は同じ shutdown 結果を表示する。startup failure 中は durable quit operation を作らず、安全な process-local exit だけを提供する。
- **R-016**: graceful application quit は最初の request から 15 秒以内に、作用開始前の安全な abort、exit、または restart を決定する。作用開始後または開始結果不明の場合は未完了 identity を残して終了し、再起動後に確認できる。
- **R-017**: operation / terminal の direct lookup と、startup recovery、feedback、shutdown target / history / associated recovery の collection query は、無関係な session や event history の件数に依存しない。collection は同じ revision の有限 page と continuation を返す。public boundary の limit、capacity、deadline 内に完全な結果を返せない場合は、partial result を返さず安全な failure とする。
- **R-018**: 起動時は Startup attempt boundary に従って固定 path の SQLite store を直接 create / open し、対応可能な SQLite schema だけを normal admission 前に進化させる。成功後だけ normal workbench を開く。未初期化の初回作成残骸は、初回作成が未完了だったという durable evidence がある場合だけ安全に再試行する。初期化済み store、または未初期化と証明できない既存 file の検証 failure は、自動置換・削除・再初期化せず、Rust が分類した safe startup failure にする。failure surface は安全な説明、correlation、再起動時の扱い、process-local Quit だけを返し、normal command、durable quit、provider / workflow effect を admission しない。変更前の file-store は startup、通常処理、background maintenance、retention、cleanup、shutdown の入力にせず、探索、stat、列挙、読込、decode、import、変換、merge、fallback、dual write、変更、削除しない。
- **R-019**: #1499 は完了済み milestone 84 契約の利用者可視結果を退行させず、D1 #1445で確定したdesign-only境界を再定義しない。過去 Issue の Spec を現行正本として書き換えず、本 Primary Spec と milestone 84 現行正本で解決を定義する。
- **R-020**: Stop と normal completion、failure、Session close、quit が競合した場合も、turn terminal、Stop result、queue pause、Session / shutdown state は一つの canonical outcome へ収束する。保存 failure は capacity 解放や次の実行開始の根拠にしない。
- **R-021**: pending recovery と shutdown target の解決操作は backend 発行の stable action identity を持ち、同じ action の再試行は保存済み結果を返す。安全性を証明できない操作を提示せず、結果不明は別 action による blind retry へ変換しない。
- **R-022**: supported message content は、保存、restart、Tauri、WebSocket を跨いでも同じ意味を lossless に保つ。未知の必須 semantics は別の意味へ推測せず、安全な incompatibility として扱う。

# Assumptions / Open Questions

- OPEN 事項はない。
