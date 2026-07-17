# Behavior

関連: #1445（milestone 84「Agentチャット安定化」／ Phase 0 ／ D1 ／依存なし）

この文書は、#1445 の design gate で正本に確定する Agent 実行設定（mode / Goal / Reasoning effort / launch / permission）の**外部から観測可能なビジネスルール**を Gherkin で定義する。実装は後続 Issue（#1446〜#1451）が担うため、ここでは経路・型・永続化の詳細ではなく、確定した設計が保証すべき振る舞いだけを記述する。

## 仮定

- 本 Issue の成果物は正本ドキュメントの確定（design gate）であり、runtime code・typed fixture・compatibility table・`LocalEventTransactionStore` のコード実装は含まない。以下の Scenario は、確定する設計が最終的に保証すべき観測可能な振る舞いを示す（2026-07-16 確認済み）。
- Agent mode は `Ask / Edit / Plan / Auto / Bypass` の排他的 5 値 enum を採用する（V-D10 改訂で確定済み）。旧 `PermissionMode { Ask, Edit, Full } + plan_mode: bool` を supersede する。
- Agent 実行設定・Goal・Reasoning effort・permission の判断は全て Rust-owned state が所有し、frontend は selected / effective / pending projection の mirror に留まる。
- provider 仕様の規範は正本 V-D10 に列挙された Claude / Codex 公式ドキュメントと、dependency に pin した CLI / SDK tag が生成する schema・fixture とする。
- 対象 provider は Claude と Codex の 2 種とする。

## Feature: Agent 実行設定の新 domain 確定（configuration / Goal / Reasoning effort / launch / permission）

  Background:
    Given milestone 84 の正本 4 文書（audit / vocabulary / lifecycle / presentation）とマイルストーン説明・Phase 依存が参照可能である
    And Agent 実行設定は Rust-owned state が所有する
    And frontend は domain decision と action enablement を所有しない

  # --- Agent mode の supersede と migration ---

  Rule: Agent mode は排他的 5 値であり、旧 PermissionMode + plan_mode を migration 付きで supersede する

    Scenario: 新 mode は排他的 5 値である
      Given Agent 実行設定を参照する
      When mode を確認する
      Then mode は Ask / Edit / Plan / Auto / Bypass のいずれか 1 値である
      And plan_mode のような独立した boolean は存在しない

    Scenario Outline: 解決可能な旧設定は Ready な AgentMode へ写像される
      Given 旧 plan_mode が <plan_mode> で permission_mode が <permission_mode> である
      When 新 domain へ写像する
      Then command / migration load 用 AgentSessionConfigurationState は Ready になる
      And AgentMode は <mode> になる
      And Ready payload は query 専用 available_actions を含まない

      Examples:
        | plan_mode | permission_mode              | mode |
        | true      | Ask                          | Plan |
        | false     | Ask（legacy readonly 含む）   | Ask  |
        | false     | Edit                         | Edit |

    Scenario: plan_mode=true は permission_mode=Full より優先される
      Given 旧設定が plan_mode=true かつ permission_mode=Full である
      When 新 domain へ写像する
      Then command / migration load 用 AgentSessionConfigurationState は Ready になる
      And AgentMode は Plan になる

    Scenario: plan_mode=false の legacy Full は mode 未確定の解決待ちになる
      Given 旧設定が plan_mode=false かつ permission_mode=Full である
      When 新 domain へ写像する
      Then command / migration load 用 AgentSessionConfigurationState は NeedsConfigurationResolution(ConfigurationResolutionProblem) になる
      And unresolved field は Mode で reason は LegacyBypassConfirmationRequired になる
      And 解決前の AgentMode は存在しない

    Scenario: migration は lazy であり自動 write-back しない
      Given 旧 permission_mode / plan_mode を持つ既存 Session がある
      When その Session を読み出す
      Then 新しい command / migration load 用 AgentSessionConfigurationState として解釈される
      But 永続化された旧値への自動 write-back は発生しない

    Scenario: legacy Full Session は解決前の送信を拒否し再 challenge を要求する
      Given command 側 AgentSessionConfigurationState が Mode の LegacyBypassConfirmationRequired を持つ NeedsConfigurationResolution である
      When user が送信を試みる
      Then 送信は block される
      And Bypass 相当の再 challenge が要求される

  # --- Goal と configuration の分離 ---

  Rule: Goal aggregate は configuration aggregate から独立している

    Scenario: Goal は configuration revision と独立した lifecycle を持つ
      Given ある Session に configuration と Goal が存在する
      When configuration の mode / model / effort を更新する
      Then Goal の id / revision / pending / sync state は影響を受けない

    Scenario: current Goal は同時に最大 1 件である
      Given current Goal が 1 件設定されている
      When 別の Goal を設定する
      Then current Goal は新しい Goal に置き換わる
      And current Goal が同時に 2 件以上存在することはない

    Scenario: Completed / Failed の Goal も clear または replace まで current に保持される
      Given current Goal が Completed または Failed になった
      When user が clear も replace も行わない
      Then その Goal は current として保持され続ける

    Scenario: automatic continuation は AgentMode::Auto と独立して扱われる
      Given Goal に紐づく automatic continuation の振る舞いがある
      When AgentMode を Auto 以外に変更する
      Then automatic continuation は AgentMode の値によって決定されない

  # --- Reasoning effort（工数） ---

  Rule: Reasoning effort は usage / budget と別概念であり selected / effective / unknown を区別する

    Scenario: Reasoning effort は TokenUsage / cost / time / turn / budget と混同されない
      Given Session の実行設定と accounting を参照する
      When Reasoning effort を確認する
      Then Reasoning effort は TokenUsage / cost / time / turn / 各種 budget のいずれとも別の概念として表現される

    Scenario Outline: selected effort の状態
      Given selected effort が <selected> である
      Then それは ProviderDefault または Explicit(value) として区別される

      Examples:
        | selected          |
        | ProviderDefault   |
        | Explicit(value)   |

    Scenario Outline: effective effort は Known または Unknown の直和である
      Given provider へ effort を送信した結果を参照する
      Then effective effort は <effective> として区別される

      Examples:
        | effective                                             |
        | Known { value, source: ExplicitSelection }            |
        | Known { value, source: ProviderDefault }              |
        | Unknown { selected, expected, reason }                |

    Scenario: effort の選択肢は pin した compatibility に駆動される
      Given provider API または protocol identity に pin した compatibility が定義されている
      When 選択可能な effort を提示する
      Then 選択肢は provider × version × model の compatibility に基づく

  # --- 更新確定と reconciliation ---

  Rule: 実行に影響する設定更新は確定と reconciliation の規則に従う

    Scenario: 初期実装では execution-affecting 更新は Idle 限定である
      Given Session が実行中（Idle ではない）である
      When user が execution-affecting な設定更新を要求する
      Then その更新は初期実装では受け付けられない

    Scenario: selected commit 前に provider が reject を確定した場合だけ旧値を維持する
      Given user が設定更新を要求した
      And SessionConfigurationSelected はまだ append されていない
      When provider が reject を確定する
      Then ConfigurationUpdateRejected が記録される
      And pending update は消去される
      And 旧 selected / effective が維持される

    Scenario: selected commit 後の NextTurn / Restart activation reject は selected を巻き戻さない
      Given SessionConfigurationSelected が append 済みで new selected / old effective が保持されている
      When NextTurn または Restart の activation を provider が reject または timeout する
      Then new selected / old effective が維持される
      And 設定は ReconciliationRequired 状態になる
      And selected は旧 revision へ巻き戻らない

    Scenario Outline: 確定できない結果は ReconciliationRequired になる
      Given 設定更新を適用しようとした
      When 結果が <outcome> になる
      Then 設定は ReconciliationRequired 状態になる

      Examples:
        | outcome                     |
        | timeout                     |
        | partial apply               |
        | ack 後の canonical append 失敗 |
        | provider conflict           |

    Scenario: NextTurn / Restart は activation ack と event 後に TurnStarted で反映される
      Given selected と effective が分けて保持されている
      When NextTurn または Restart で設定を activation する
      Then activation ack と event の後に TurnStarted として反映される

  # --- Auto / Bypass と権限 ---

  Rule: Auto / Bypass は workflow checkpoint を越えず Bypass は Rust が gate する

    Scenario: Auto は workflow checkpoint を自動で越えない
      Given AgentMode が Auto の Session が workflow checkpoint に到達する
      When checkpoint が human 判断を要求する
      Then Auto は checkpoint を自動で越えない

    Scenario: Bypass は workflow checkpoint を自動で越えない
      Given AgentMode が Bypass の Session が workflow checkpoint に到達する
      When checkpoint が human 判断を要求する
      Then Bypass は checkpoint を自動で越えない

    Scenario: Bypass は Rust の challenge と provider gate を経て初めて有効になる
      Given user が Bypass を有効化しようとする
      When Rust の challenge または provider gate が未充足である
      Then Bypass は有効にならない

    Scenario: Bypass template の保存は権限付与ではない
      Given Bypass を含む workflow template が保存されている
      When その template を execution する
      Then execution ごとに challenge と provider gate が検証される
      And template 保存自体は Bypass 権限を付与しない

  # --- Provider 差 ---

  Rule: Goal operation capability は GoalCapabilitySupport で表現される

    Scenario Outline: Goal operation は Goal 専用の support として分類される
      Given provider が <provider> である
      When Goal operation <operation> の対応方針を参照する
      Then GoalCapabilitySupport は Native / Emulated / Unsupported のいずれかで表現される
      And Native / Emulated は strategy / scope / GoalSideEffect を持つ
      And Unsupported の場合は理由が付与される

      Examples:
        | provider | operation        |
        | Claude   | set（/goal）     |
        | Claude   | pause / resume  |
        | Codex    | set / clear     |

  Rule: mode capability は ModeCapabilitySupport で表現される

    Scenario: Claude Bypass は mode 専用 capability と追加 gate を持つ
      Given provider が Claude で mode が Bypass である
      When mode の対応方針を参照する
      Then ModeCapabilitySupport は Native / Composed / Unsupported のいずれかで表現される
      And Unsupported の場合は理由が付与される
      And requires_launch_opt_in と residual_protections が mode capability に保持される
      And Rust challenge は provider の dangerous launch opt-in を置き換えない

  Rule: Reasoning effort capability は schema / runtime validation と readback 可否で表現される

    Scenario: Claude effort は ReasoningEffortCapability として検証される
      Given provider が Claude である
      When Reasoning effort の対応方針を参照する
      Then schema_supported / runtime_available / authoritative_runtime_validation / effective_readback_supported が区別される
      And runtime capability が無い場合は pin した version × model compatibility table を availability source とする
      And authoritative に検証できない model / value は runtime_available=false と unavailable_reason で表現される

  Rule: Codex の provider status と review result は exhaustive mapping で表現される

    Scenario: Codex の Goal status は全域が写像される
      Given Codex の Goal status を参照する
      When status が active / paused / complete / blocked / usageLimited / budgetLimited のいずれかである
      Then すべて欠落なく Goal projection へ写像される
      And read-only accounting は ReasoningEffort と分離される

    Scenario Outline: Codex の Auto review は結果によって解決先が異なる
      Given Codex の Auto review 結果が <result> である
      Then それは <resolution> として扱われる

      Examples:
        | result     | resolution              |
        | approved   | Auto 解決                |
        | denied     | Auto 解決                |
        | inProgress | activity / 未解決へ       |
        | timedOut   | activity / 未解決へ       |
        | aborted    | activity / 未解決へ       |

  # --- Protocol identity ---

  Rule: pin した schema と実行 binary の identity mismatch は fail-closed で検出される

    Scenario: identity が一致すれば Session は確立される
      Given compiled generated schema と spawn した CLI/flags/capabilities が一致する
      When initialize 時に identity を照合する
      Then Session は確立される

    Scenario: control-plane の drift は ProtocolIncompatible として fail-closed になる
      Given pin した schema と実行 binary の control-plane identity が mismatch する
      When initialize 時に identity を照合する
      Then ProtocolIncompatible として fail-closed になる
      And Session 確立後は session-level、確立前は durable launch attempt として保存される

    Scenario: parse 可能な content-plane の unknown は低強調で継続される
      Given parse 可能かつ content-plane と分類できる unknown message または part がある
      When それを受信する
      Then Session は継続する
      And payload長・digest・content分類・上限付きsecret-redacted sampleを持つ低強調の UnsupportedMessage として提示される
      But full body は durable event または構造化ログへ恒久保存されない

    Scenario Outline: 分類または decode できない payload は fail-closed になる
      Given <payload> を受信する
      When content-plane と control-plane の分類と typed decode を行う
      Then payload長・digest・分類/失敗種別・上限付きsecret-redacted sampleと取得済みの部分 protocol identity が保存される
      And ProtocolIncompatible として新規 turn が block される
      But full body は durable event または構造化ログへ恒久保存されない

    Scenario: 完全な provider evidence は bounded evidence store からだけ参照される
      Given protocol incompatibility の調査に完全な payload evidence が必要である
      When adaptor が evidence を保存する
      Then secret plaintext は保存前に redaction される
      And payload は暗号化 at rest、per-session quota、単一 object size 上限、TTL、参照認可を持つ bounded evidence store に保存される
      And durable event は ProviderEvidenceRef だけを保持する
      And quota または size 上限超過時に full body を event または log へ fallback しない

      Examples:
        | payload                                      |
        | content/control を分類できない malformed frame |
        | 既知 message variant の壊れた payload           |
        | 単一 frame の size 上限を超える payload          |

  # --- frontend の責務境界 ---

  Rule: multi-stream query と watch は同じ pinned snapshot と subscription lifecycle に固定される

    Scenario: 複数 repository を横断する query は batch の半分を返さない
      Given 複数 stream の event が 1 つの local atomic batch で commit されている
      When backend query が有効期限付き snapshot lease を取得して各 event / projection source を読む
      Then 全 source は同じ lease の read_at で読まれる
      And lease の barrier より後の projection を混ぜない
      And 同じ batch の participant は全件見えるか全件見えないかのどちらかである
      And query 完了時に lease は解放される

    Scenario: projector が event commit より遅れても half-visible read を返さない
      Given global commit N の event row は可視だが required projection source の watermark は N-1 である
      When backend query が required source を列挙して snapshot lease を取得する
      Then lease barrier は全 source の common readable watermark N-1 以下に置かれる
      And event N と stale projection N-1 を同じ結果へ合成しない
      And read_at 時に source watermark が barrier 未満なら ProjectionBehind を返してquery全体を破棄する
      When 全 required projection source の watermark が N へ追随する
      Then fresh lease では同じbatchのparticipantとprojectionが全件見える

    Scenario: query 中に snapshot lease が失効する
      Given 複数 repository の一部を読み終えた後に snapshot lease が失効する
      When 残りの source が SnapshotExpired を返す
      Then backend は部分結果を破棄する
      And fresh lease で query 全体を bounded retry する
      And source 単位の latest read と旧 lease の結果を混ぜない

    Scenario: watch の bootstrap と subscription の間に更新を取りこぼさない
      Given client が surface cursor を指定して watch を開始する
      When backend が replay または snapshot を選ぶ
      Then replay 判定と snapshot lease 取得と barrier 後の subscription / receiver 登録は同じ commit 境界で行われる
      And bootstrap 中に発生した barrier 後の通知は登録済み subscription の bounded buffer へ入る
      And finish_bootstrap 後は同じ watch port の receive から順に配信される

    Scenario: live notice は commit 固定 lease から typed update へ変換される
      Given watch buffer に global commit N の notice があり required projection source はまだ N-1 までしか追随していない
      When usecase-owned watch service が次の update を受ける
      Then receive は required source が N へ追随するまでnoticeだけを返さない
      And N へ厳密にpinしたleaseを持つLocalWatchUpdateFenceを返す
      And watch serviceはそのleaseだけでAgentLaunchChangedまたはSessionのtyped snapshot/deltaをquery serviceに構築させる
      And finish_update がlive leaseを解放してから次のnoticeへ進む
      And Tauri / WebSocket handlerはRepositoryやQueryServiceを直接呼ばずtyped frameだけを送る

    Scenario: watch の client が切断または lag する
      Given client が watch handle を保持している
      When client が切断する、bounded buffer の上限を超える、またはprojection追随がbounded timeoutする
      Then backend は close_watch または WatchLagged により subscription と未解放 lease を回収する
      And commit 処理を subscriber の backpressure で block しない
      And lag 時は部分 bootstrap / delta を捨てて snapshot から watch 全体を再開する

  Rule: configuration projection は mutation authority ではない

    Scenario: configuration を更新して read model を再構築する
      Given Rust-owned AgentSessionConfiguration aggregate が configuration update を受理する
      When SessionConfigurationSelected または SessionConfigurationActivated が canonical commit される
      Then durable authority は canonical event に置かれる
      And AgentSessionConfigurationProjection は committed event / projection data から backend query が構築する
      And projection cache を mutation authority として直接更新しない

    Scenario: provider runtime observation と client-facing configuration delta を分離する
      Given provider がmodel / permission / reasoning effortの現在値を報告する
      When adaptorがprovider wireをAgentRuntimeEventへ変換する
      Then runtime eventはprovider-neutralなProviderConfigurationStateObservedとevidence refだけを運ぶ
      And AgentSessionConfigurationProjectionやavailable_actionsをruntime gatewayで構築しない
      When canonical observation eventがcommitされclient watchを更新する
      Then query/watch境界がpinned sourceとpolicy evaluation contextからSessionConfigurationChangedのfull read-model deltaを構築する

  Rule: frontend は projection の mirror に留まり domain decision を所有しない

    Scenario: action enablement は backend projection が決定する
      Given frontend が selected / effective / pending projection を受け取る
      When 設定変更や Goal 操作の可否を表示する
      Then mode / model / effort変更の可否は AgentSessionConfigurationProjection の available_actions に従う
      And Goal操作の可否は SessionGoalProjection の available_actions に従う
      And frontend は独自に enablement を計算しない

## Open Questions

なし（2026-07-16 に全て解消済み）。
