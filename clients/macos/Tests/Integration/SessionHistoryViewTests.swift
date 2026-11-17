import AppKit
import ApplicationServices
import SwiftProtobuf
import SwiftUI
import XCTest

@testable import Releash

final class SessionHistoryViewTests: IntegrationTestCase {
  @MainActor func test_B001_B002_B003_B004_B005_Historyの入口と見出しと閉じる操作() async throws {
    // Given
    let (_, model) = try await setup()
    let host = await ViewHarness(RootListView(model: model, path: "/repo"))
    defer { host.close() }
    // When
    try await MenuHarness().open(
      selection: ["History…"], press: { try await host.press("More") })
    try await eventually { !host.window.sheets.isEmpty }
    // Then
    XCTAssertEqual(
      NSHostingView(rootView: SessionHistoryView(model: model, path: "/repo")).fittingSize,
      NSSize(width: 1024, height: 800))
    try await eventuallyView(host) { host.element("feature/thread-ui") != nil }
    XCTAssertNotNil(host.element("History"))
    XCTAssertNotNil(host.element("行を選ぶと操作が出ます。ダブルクリックで復元・再開します。"))
    XCTAssertNotNil(host.element("閉じる"))
    XCTAssertFalse(elements(named: "名前・最初の指示で検索", in: host).isEmpty)
    XCTAssertEqual(
      try attribute(named: "名前・最初の指示で検索", in: host, key: kAXValueAttribute) as? String, "")
    for title in ["すべて", "Archive 済み", "Provider の履歴"] {
      XCTAssertNotNil(host.element(title))
    }
    XCTAssertEqual(
      try attribute(named: "すべて", in: host, key: kAXValueAttribute) as? NSNumber, NSNumber(value: 1)
    )
    // When
    try await host.press("閉じる")
    // Then
    try await eventually { host.window.sheets.isEmpty }
    await model.stop()
  }

  @MainActor func test_B006_B011_B012_区画の見出しと二段の行を表示する() async throws {
    // Given
    let (http, model) = try await setup()
    var calendar = Calendar(identifier: .gregorian)
    calendar.timeZone = try XCTUnwrap(TimeZone(secondsFromGMT: 9 * 3600))
    let now = try XCTUnwrap(
      calendar.date(from: DateComponents(year: 2026, month: 11, day: 15, hour: 12)))
    let yesterday = try XCTUnwrap(calendar.date(byAdding: .day, value: -1, to: now))
    let workflowTime = try XCTUnwrap(
      calendar.date(bySettingHour: 18, minute: 42, second: 0, of: yesterday))
    let archive = [
      session("Thread 面を GitHub の Conversation の見せ方にする", at: now.addingTimeInterval(-7200)),
      session("別の Session", id: "second", at: yesterday),
      workflow("development-v4", id: "completed", at: workflowTime, completed: true),
      workflow("中止した Workflow", id: "aborted", at: yesterday, completed: false),
    ]
    http.state.withLock {
      $0.history.items.items = [
        candidate("Source Control 面の Commit メニューに Push を足したい", at: now.addingTimeInterval(-10800))
      ]
    }
    // When
    let host = try await history(model, http: http, archive: archive, now: now, calendar: calendar)
    defer { host.close() }
    // Then
    try await eventuallyView(host) { host.element("development-v4") != nil }
    for title in [
      "ARCHIVE 済み", "PROVIDER の履歴", "このディレクトリで provider が残した Session",
      "Thread 面を GitHub の Conversation の見せ方にする", "Workflow", "完了", "中止", "昨日 18:42",
      "Source Control 面の Commit メニューに Push を足したい",
    ] {
      XCTAssertTrue(host.texts().contains { $0.contains(title) }, title)
    }
    for (title, expected) in [
      ("ARCHIVE 済み", ["4"]),
      ("PROVIDER の履歴", ["このディレクトリで provider が残した Session"]),
    ] {
      let heading = try XCTUnwrap(elements(named: title, in: host).first)
      let section = try XCTUnwrap(attribute(heading, kAXParentAttribute)) as! AXUIElement
      let children = try XCTUnwrap(attribute(section, kAXChildrenAttribute) as? [AXUIElement])
      let headingIndex = try XCTUnwrap(children.firstIndex { CFEqual($0, heading) })
      let headerTexts = children.dropFirst(headingIndex + 1).prefix {
        attribute($0, kAXRoleAttribute) as? String != kAXHeadingRole
      }.compactMap { attribute($0, kAXValueAttribute) as? String }
      XCTAssertEqual(headerTexts, expected, title)
    }
    await model.stop()
  }

  @MainActor func test_B009_さらに読み込むと既存行を残して続きが出る() async throws {
    // Given
    let (http, model) = try await setup()
    let first = candidate("新しい相談")
    http.state.withLock {
      $0.history.items.items = [first]
      $0.history.hasMore_p = true
    }
    let host = try await history(model, http: http)
    defer { host.close() }
    try await eventuallyView(host) { host.element("新しい相談") != nil }
    let previousSubscriptions = http.state.withLock { $0.subscriptions.map(\.subscriptionID) }
    // When
    try await host.press("さらに読み込む")
    try await eventually {
      http.state.withLock {
        $0.subscriptions.contains {
          $0.target == "session-history" && $0.args == ["/repo", "40"]
            && !previousSubscriptions.contains($0.subscriptionID)
        }
      }
    }
    var payload = Releash_Client_V1_StatePayload()
    payload.sessionHistory.items.items = [first, candidate("古い相談", id: "older")]
    try http.publish(payload, target: "session-history", args: ["/repo", "40"])
    // Then
    try await eventuallyView(host) { host.element("古い相談") != nil }
    XCTAssertNotNil(host.element("新しい相談"))
    let texts = host.texts()
    XCTAssertLessThan(
      try XCTUnwrap(texts.firstIndex(of: "新しい相談")), try XCTUnwrap(texts.firstIndex(of: "古い相談")))
    await model.stop()
  }

  @MainActor func test_B024_B026_区画を切り替えてすべてに戻す() async throws {
    // Given
    let (http, model) = try await setup()
    http.state.withLock { $0.history.items.items = [candidate("Provider の相談")] }
    let host = try await history(model, http: http, archive: [session("Archive の相談")])
    defer { host.close() }
    try await eventuallyView(host) { host.element("Archive の相談") != nil }
    // When
    try await host.press("Archive 済み")
    // Then
    try await eventuallyView(host) { host.element("PROVIDER の履歴") == nil }
    XCTAssertNotNil(host.element("ARCHIVE 済み"))
    XCTAssertNotNil(host.element("Archive の相談"))
    // When
    try await host.press("すべて")
    // Then
    try await eventuallyView(host) { host.element("PROVIDER の履歴") != nil }
    XCTAssertNotNil(host.element("ARCHIVE 済み"))
    XCTAssertNotNil(host.element("Provider の相談"))
    await model.stop()
  }

  @MainActor func test_B027_B028_B029_行の選択だけでは操作せず選択行に操作を出す() async throws {
    // Given
    let (http, model) = try await setup()
    http.state.withLock { $0.history.items.items = [candidate("Provider の相談")] }
    let host = try await history(
      model, http: http,
      archive: [
        session("Archive の相談"), workflow("development-v4", id: "flow", at: Date(), completed: true),
      ])
    defer { host.close() }
    try await eventuallyView(host) { host.element("Archive の相談") != nil }
    // When / Then
    for (row, action, deletable) in [
      ("Archive の相談", "復元", true), ("development-v4", "復元", false), ("Provider の相談", "再開", false),
    ] {
      try await host.press(row)
      try await eventuallyView(host) { host.element(action) != nil }
      XCTAssertEqual(
        elements(named: action, in: host).filter {
          attribute($0, kAXRoleAttribute) as? String == kAXButtonRole
        }.count, 1)
      XCTAssertEqual(host.element("削除") != nil, deletable)
      XCTAssertNil(host.element(action == "復元" ? "再開" : "復元"))
      for method in [
        "RestoreAgentSession", "RestoreWorkspaceWorkflowExecution",
        "ResumeAgentSessionHistoryCandidate", "DeleteAgentSession",
      ] {
        XCTAssertTrue(http.requests(method).isEmpty, method)
      }
    }
    await model.stop()
  }

  @MainActor func test_B034_B036_削除確認のキャンセルは行を残す() async throws {
    // Given
    let (http, model) = try await setup()
    let host = try await history(model, http: http, archive: [session("Thread 面の見せ方")])
    defer { host.close() }
    try await host.press("Thread 面の見せ方")
    // When
    try await host.press("削除")
    // Then
    try await eventuallyView(host) { host.element("「Thread 面の見せ方」を削除しますか？") != nil }
    XCTAssertNotNil(host.element("削除した Session は History から消え、復元できません。"))
    XCTAssertNotNil(host.element("削除"))
    XCTAssertNotNil(host.element("キャンセル"))
    XCTAssertTrue(http.requests("DeleteAgentSession").isEmpty)
    // When
    try await host.pressAlertButton("キャンセル")
    // Then
    try await eventuallyView(host) { host.element("「Thread 面の見せ方」を削除しますか？") == nil }
    XCTAssertNotNil(host.element("Thread 面の見せ方"))
    XCTAssertTrue(http.requests("DeleteAgentSession").isEmpty)
    await model.stop()
  }

  @MainActor func test_B030_B035_復元と確認後の削除をAPIへ送り一覧の更新を表示する() async throws {
    for deleting in [false, true] {
      // Given
      let (http, model) = try await setup()
      let host = try await history(model, http: http, archive: [session("Thread 面の見せ方")])
      defer { host.close() }
      try await host.press("Thread 面の見せ方")
      // When
      try await host.press(deleting ? "削除" : "復元")
      if deleting {
        try await eventuallyView(host) { host.element("キャンセル") != nil }
        try await host.pressAlertButton("削除")
      }
      let method = deleting ? "DeleteAgentSession" : "RestoreAgentSession"
      try await eventually { !http.requests(method).isEmpty }
      if deleting {
        let request = try Releash_Client_V1_DeleteAgentSessionRequest(
          serializedBytes: XCTUnwrap(http.requests(method).last?.message))
        XCTAssertEqual(request.agentSessionID, "archived")
      } else {
        let request = try Releash_Client_V1_RestoreAgentSessionRequest(
          serializedBytes: XCTUnwrap(http.requests(method).last?.message))
        XCTAssertEqual(request.agentSessionID, "archived")
      }
      var payload = Releash_Client_V1_StatePayload()
      payload.archivedHistory = .init()
      try http.publish(payload, target: "archived-history")
      // Then
      try await eventuallyView(host) { host.element("Thread 面の見せ方") == nil }
      if !deleting {
        payload = .init()
        payload.workspaces.repositories.items = [
          .with {
            $0.path = "/repo"
            $0.worktrees.items = [
              .with {
                $0.path = "/repo"
                $0.snapshot.nodes.items = [
                  .with {
                    $0.node.id = "archived"
                    $0.node.title = "Thread 面の見せ方"
                    $0.node.contentKind.value = .session
                    $0.node.sessionID = "archived"
                  }
                ]
              }
            ]
          }
        ]
        try http.publish(payload, target: "workspaces")
        let nodes = await ViewHarness(RootListView(model: model, path: "/repo"))
        try await eventuallyView(nodes) { nodes.element("root:archived") != nil }
        nodes.close()
      }
      await model.stop()
    }
  }

  @MainActor func test_B014_B015_Providerのラベルを行の一段目に表示する() async throws {
    // Given
    let (http, model) = try await setup()
    http.state.withLock {
      $0.history.items.items = [
        candidate("PR 面の CHECKS の並び順を相談したい", id: "prompt"),
        candidate("Claude 1a2b3c4d…", id: "1a2b3c4d-rest"),
      ]
    }
    // When
    let host = try await history(model, http: http)
    defer { host.close() }
    // Then
    try await eventuallyView(host) { host.element("PR 面の CHECKS の並び順を相談したい") != nil }
    XCTAssertNotNil(host.element("Claude 1a2b3c4d…"))
    await model.stop()
  }

  @MainActor func test_B020_B023_B025_検索結果を表示し区画を変えても語を保つ() async throws {
    // Given
    let (http, model) = try await setup()
    let archive = [
      session("Thread 面の見せ方"), workflow("development-v4", id: "flow", at: Date(), completed: true),
    ]
    let provider = [candidate("Thread の並び順"), candidate("Push を足したい", id: "push")]
    http.state.withLock { $0.history.items.items = provider }
    let host = try await history(model, http: http, archive: archive)
    defer { host.close() }
    try await eventuallyView(host) { host.element("development-v4") != nil }
    var previousSubscriptions = http.state.withLock { $0.subscriptions.map(\.subscriptionID) }
    // When
    try await host.setTextField("thread")
    try await eventually {
      http.state.withLock { state in
        [
          ("archived-history", ["/repo", "search=thread"]),
          ("session-history", ["/repo", "20", "search=thread"]),
        ].allSatisfy { target, args in
          state.subscriptions.contains {
            $0.target == target && $0.args == args
              && !previousSubscriptions.contains($0.subscriptionID)
          }
        }
      }
    }
    var payload = Releash_Client_V1_StatePayload()
    payload.archivedHistory.items = [archive[0]]
    try http.publish(payload, target: "archived-history", args: ["/repo", "search=thread"])
    payload = .init()
    payload.sessionHistory.items.items = [provider[0]]
    try http.publish(payload, target: "session-history", args: ["/repo", "20", "search=thread"])
    // Then
    try await eventuallyView(host) {
      host.element("development-v4") == nil && host.element("Push を足したい") == nil
    }
    XCTAssertNotNil(host.element("Thread 面の見せ方"))
    XCTAssertNotNil(host.element("Thread の並び順"))
    XCTAssertTrue(
      elements(named: "1", in: host).contains {
        attribute($0, kAXRoleAttribute) as? String == kAXStaticTextRole
      })
    // When
    try await host.press("Provider の履歴")
    // Then
    try await eventuallyView(host) { host.element("ARCHIVE 済み") == nil }
    XCTAssertNotNil(host.element("PROVIDER の履歴"))
    XCTAssertNotNil(host.element("Thread の並び順"))
    XCTAssertNil(host.element("Push を足したい"))
    XCTAssertNotNil(host.element("thread"))
    // When
    try await host.press("すべて")
    previousSubscriptions = http.state.withLock { $0.subscriptions.map(\.subscriptionID) }
    try await host.setTextField("")
    try await eventually {
      http.state.withLock { state in
        [
          ("archived-history", ["/repo"]),
          ("session-history", ["/repo", "20"]),
        ].allSatisfy { target, args in
          state.subscriptions.contains {
            $0.target == target && $0.args == args
              && !previousSubscriptions.contains($0.subscriptionID)
          }
        }
      }
    }
    payload = .init()
    payload.archivedHistory.items = archive
    try http.publish(payload, target: "archived-history", args: ["/repo"])
    payload = .init()
    payload.sessionHistory.items.items = provider
    try http.publish(payload, target: "session-history", args: ["/repo", "20"])
    // Then
    try await eventuallyView(host) {
      host.element("development-v4") != nil && host.element("Push を足したい") != nil
    }
    XCTAssertNotNil(host.element("Thread 面の見せ方"))
    XCTAssertNotNil(host.element("Thread の並び順"))
    await model.stop()
  }

  @MainActor func test_B031_WorkflowをダブルクリックするとNodeに復元してArchiveから消える() async throws {
    // Given
    let (http, model) = try await setup()
    let host = try await history(
      model, http: http,
      archive: [workflow("development-v4", id: "archived-flow", at: Date(), completed: true)])
    defer { host.close() }
    // When
    try await doubleClickRow("development-v4", in: host)
    try await eventually { !http.requests("RestoreWorkspaceWorkflowExecution").isEmpty }
    let request = try Releash_Client_V1_RestoreWorkspaceWorkflowExecutionRequest(
      serializedBytes: XCTUnwrap(http.requests("RestoreWorkspaceWorkflowExecution").last?.message))
    XCTAssertEqual(request.worktreePath, "/repo")
    XCTAssertEqual(request.executionID, "archived-flow")
    var payload = Releash_Client_V1_StatePayload()
    payload.archivedHistory = .init()
    try http.publish(payload, target: "archived-history")
    payload = .init()
    payload.workspaces.repositories.items = [
      .with {
        $0.path = "/repo"
        $0.worktrees.items = [
          .with {
            $0.path = "/repo"
            $0.snapshot.nodes.items = [
              .with {
                $0.sequence.id = "archived-flow"
                $0.sequence.title = "development-v4"
              }
            ]
          }
        ]
      }
    ]
    try http.publish(payload, target: "workspaces")
    // Then
    try await eventuallyView(host) { host.element("development-v4") == nil }
    let nodes = await ViewHarness(RootListView(model: model, path: "/repo"))
    defer { nodes.close() }
    try await eventuallyView(nodes) { nodes.element("root:archived-flow") != nil }
    await model.stop()
  }

  @MainActor func test_B032_B033_ボタンとダブルクリックでProvider履歴をSessionタブに開く() async throws {
    for doubleClick in [false, true] {
      // Given
      let (http, model) = try await setup()
      http.state.withLock {
        $0.history.items.items = [candidate("再開する相談")]
        $0.sessionTabs["agent-session"] = .with {
          $0.session.id = "agent-session"
          $0.session.workspaceWorktreePath = "/repo"
          $0.session.availability = .paused
          $0.node.title = "再開する相談"
        }
      }
      let host = try await history(model, http: http)
      defer { host.close() }
      // When
      if doubleClick {
        try await doubleClickRow("再開する相談", in: host)
      } else {
        try await host.press("再開する相談")
        try await host.press("再開")
      }
      try await eventually { !http.requests("ResumeAgentSessionHistoryCandidate").isEmpty }
      let request = try Releash_Client_V1_ResumeAgentSessionHistoryCandidateRequest(
        serializedBytes: XCTUnwrap(
          http.requests("ResumeAgentSessionHistoryCandidate").last?.message))
      XCTAssertEqual(request.provider, "claude")
      XCTAssertEqual(request.providerSessionID, "provider")
      XCTAssertEqual(request.worktreePath, "/repo")
      guard case .pane(let pane, _, _) = try model.panes.layout() else {
        return XCTFail("Expected a single pane")
      }
      let tab = PaneTab(kind: .session, agentSessionID: "agent-session")
      var payload = Releash_Client_V1_StatePayload()
      payload.workspaceState.value.version = 1
      payload.workspaceState.value.paneLayout =
        PaneLayout.pane(id: pane, tabs: [tab], active: tab.id).message
      try http.publish(payload, target: "workspace-state")
      // Then
      let tabs = await ViewHarness(WorkspacePaneView(model: model))
      defer { tabs.close() }
      try await eventuallyView(tabs) { tabs.element("再開する相談") != nil }
      await model.stop()
    }
  }

  @MainActor func test_B007_ArchiveのSessionとWorkflowを一つの並びで表示する() async throws {
    // Given
    let (http, model) = try await setup()
    let now = Date()
    let archive = [
      session("Session A", id: "a", at: now.addingTimeInterval(-7200)),
      workflow("Workflow W", id: "w", at: now.addingTimeInterval(-86400), completed: true),
      session("Session B", id: "b", at: now.addingTimeInterval(-100000)),
    ]
    // When
    let host = try await history(model, http: http, archive: archive)
    defer { host.close() }
    // Then
    try await eventuallyView(host) { host.element("Session B") != nil }
    let texts = host.texts()
    let positions = try ["Session A", "Workflow W", "Session B"].map {
      try XCTUnwrap(texts.firstIndex(of: $0))
    }
    XCTAssertLessThan(positions[0], positions[1])
    XCTAssertLessThan(positions[1], positions[2])
    await model.stop()
  }

  @MainActor func test_B010_B013_B016_今日の時刻を秒と分と時間で表示する() async throws {
    // Given
    var calendar = Calendar(identifier: .gregorian)
    calendar.timeZone = try XCTUnwrap(TimeZone(secondsFromGMT: 9 * 3600))
    let now = try XCTUnwrap(
      calendar.date(from: DateComponents(year: 2026, month: 11, day: 15, hour: 12)))
    let (http, model) = try await setup()
    let archive = [
      session("たった今の相談", id: "now", at: now.addingTimeInterval(-30)),
      session("25分前の相談", id: "minutes", at: now.addingTimeInterval(-1500)),
      session(
        "Thread 面を GitHub の Conversation の見せ方にする", id: "hours", at: now.addingTimeInterval(-7200)),
    ]
    http.state.withLock {
      $0.history.items.items = [
        candidate("Source Control 面の Commit メニューに Push を足したい", at: now.addingTimeInterval(-10800))
      ]
    }
    // When
    let host = try await history(model, http: http, archive: archive, now: now, calendar: calendar)
    defer { host.close() }
    // Then
    for subtitle in [
      "Session · Claude · たった今に Archive", "Session · Claude · 25 分前に Archive",
      "Session · Claude · 2 時間前に Archive", "Claude · 3 時間前",
    ] {
      try await eventuallyView(host) { host.element(subtitle) != nil }
    }
    await model.stop()
  }

  @MainActor func test_B017_B018_B019_昨日以前の時刻を暦日で表示する() async throws {
    // Given
    var calendar = Calendar(identifier: .gregorian)
    calendar.timeZone = try XCTUnwrap(TimeZone(secondsFromGMT: 9 * 3600))
    let now = try XCTUnwrap(
      calendar.date(from: DateComponents(year: 2026, month: 11, day: 15, hour: 1)))
    let yesterdayAt23 = try XCTUnwrap(
      calendar.date(from: DateComponents(year: 2026, month: 11, day: 14, hour: 23)))
    let older = try XCTUnwrap(
      calendar.date(from: DateComponents(year: 2026, month: 11, day: 12, hour: 12)))
    let previousYear = try XCTUnwrap(
      calendar.date(from: DateComponents(year: 2025, month: 11, day: 12, hour: 12)))
    let (http, model) = try await setup()
    let archive = [
      session("昨日の相談", id: "yesterday", at: yesterdayAt23), session("今年の相談", id: "year", at: older),
      session("去年の相談", id: "previous", at: previousYear),
    ]
    // When
    let host = try await history(model, http: http, archive: archive, now: now, calendar: calendar)
    defer { host.close() }
    // Then
    for subtitle in [
      "Session · Claude · 昨日 23:00に Archive", "Session · Claude · 11月12日に Archive",
      "Session · Claude · 2025年11月12日に Archive",
    ] {
      try await eventuallyView(host) { host.element(subtitle) != nil }
    }
    await model.stop()
  }

  @MainActor func test_B011_B012_Workflowの状態を文字と緑または赤の点で表示する() async throws {
    for completed in [true, false] {
      // Given
      let (http, model) = try await setup()
      let host = try await history(
        model, http: http,
        archive: [workflow("状態を確かめる Workflow", id: "flow", at: Date(), completed: completed)])
      defer { host.close() }
      // When / Then
      try await eventuallyView(host) {
        host.texts().contains { $0.contains(completed ? "完了" : "中止") }
      }
      let view = try XCTUnwrap(host.window.contentView)
      view.layoutSubtreeIfNeeded()
      let bitmap = try XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: view.bounds))
      view.cacheDisplay(in: view.bounds, to: bitmap)
      let pixels = try IconRaster(bitmap: bitmap).pixels
      XCTAssertTrue(
        pixels.contains { pixel in
          completed
            ? pixel.y > pixel.x + 0.15 && pixel.y > pixel.z + 0.1
            : pixel.x > pixel.y + 0.2 && pixel.x > pixel.z + 0.2
        }, completed ? "完了を示す緑の点" : "中止を示す赤の点")
      await model.stop()
    }
  }

  @MainActor private func setup() async throws -> (TestHTTPClient, AppModel) {
    let http = TestHTTPClient()
    http.state.withLock {
      $0.repositories = [
        .with {
          $0.path = "/repo"
          $0.branches.items = [
            .with {
              $0.name = "feature/thread-ui"
              $0.worktreePath = "/repo"
            }
          ]
          $0.worktrees.items = [.with { $0.path = "/repo" }]
        }
      ]
    }
    let model = try await testModel(http)
    model.selectWorktree("/repo")
    try await eventually { model.layoutLoaded }
    return (http, model)
  }

  @MainActor private func history(
    _ model: AppModel, http: TestHTTPClient,
    archive: [Releash_Client_V1_ArchivedHistoryItem] = [],
    now: Date = Date(), calendar: Calendar = .current
  ) async throws -> ViewHarness {
    let host = await ViewHarness(
      SessionHistoryView(model: model, path: "/repo", now: now, calendar: calendar),
      size: NSSize(width: 1024, height: 800))
    try await eventually {
      http.state.withLock { $0.subscriptions.contains { $0.target == "archived-history" } }
    }
    var payload = Releash_Client_V1_StatePayload()
    payload.archivedHistory.items = archive
    try http.publish(payload, target: "archived-history")
    return host
  }

  private func session(_ name: String, id: String = "archived", at date: Date = Date())
    -> Releash_Client_V1_ArchivedHistoryItem
  {
    .with {
      $0.session.agentSessionID = id
      $0.session.name = name
      $0.session.provider.value = .claude
      $0.session.archivedAt = date.timeIntervalSince1970
      $0.session.canDelete = true
    }
  }

  private func workflow(_ title: String, id: String, at date: Date, completed: Bool)
    -> Releash_Client_V1_ArchivedHistoryItem
  {
    .with {
      $0.workflow.executionID = id
      $0.workflow.worktreePath = "/repo"
      $0.workflow.title = title
      $0.workflow.status.value = completed ? .completed : .aborted
      $0.workflow.updatedAt = date.timeIntervalSince1970
    }
  }

  private func candidate(_ label: String, id: String = "provider", at date: Date = Date())
    -> Releash_Client_V1_AgentSessionHistoryCandidateDto
  {
    .with {
      $0.provider.value = .claude
      $0.providerSessionID = id
      $0.label = label
      $0.updatedAt = date.timeIntervalSince1970
    }
  }

  @MainActor private func doubleClickRow(_ name: String, in host: ViewHarness) async throws {
    try await eventuallyView(host) { host.element(name) != nil }
    let row = try XCTUnwrap(elements(named: name, in: host).first)
    var origin = CGPoint.zero
    var size = CGSize.zero
    let position = try XCTUnwrap(attribute(row, kAXPositionAttribute))
    let dimensions = try XCTUnwrap(attribute(row, kAXSizeAttribute))
    XCTAssertTrue(AXValueGetValue(position as! AXValue, .cgPoint, &origin))
    XCTAssertTrue(AXValueGetValue(dimensions as! AXValue, .cgSize, &size))
    let screenHeight = try XCTUnwrap(NSScreen.screens.first).frame.maxY
    let point = host.window.convertPoint(
      fromScreen: NSPoint(
        x: origin.x + size.width / 2, y: screenHeight - origin.y - size.height / 2))
    for count in 1...2 {
      let up = try XCTUnwrap(
        NSEvent.mouseEvent(
          with: .leftMouseUp, location: point, modifierFlags: [],
          timestamp: ProcessInfo.processInfo.systemUptime,
          windowNumber: host.window.windowNumber, context: nil, eventNumber: count,
          clickCount: count, pressure: 0))
      let down = try XCTUnwrap(
        NSEvent.mouseEvent(
          with: .leftMouseDown, location: point, modifierFlags: [],
          timestamp: ProcessInfo.processInfo.systemUptime,
          windowNumber: host.window.windowNumber, context: nil, eventNumber: count,
          clickCount: count, pressure: 1))
      NSApp.postEvent(up, atStart: true)
      host.window.sendEvent(down)
      try await Task.sleep(for: .milliseconds(20))
    }
    await host.refresh()
  }

  @MainActor private func elements(named name: String, in host: ViewHarness) -> [AXUIElement] {
    let app = AXUIElementCreateApplication(ProcessInfo.processInfo.processIdentifier)
    let windows = attribute(app, kAXWindowsAttribute) as? [AXUIElement] ?? []
    guard
      let window = windows.first(where: {
        attribute($0, kAXTitleAttribute) as? String == host.window.title
      })
    else { return [] }
    var visited: [AXUIElement] = []
    func collect(_ element: AXUIElement) -> [AXUIElement] {
      guard !visited.contains(where: { CFEqual($0, element) }) else { return [] }
      visited.append(element)
      let matches = [
        kAXTitleAttribute, kAXDescriptionAttribute, kAXIdentifierAttribute,
        kAXPlaceholderValueAttribute, kAXValueAttribute,
      ].contains {
        attribute(element, $0) as? String == name
      }
      return (matches ? [element] : [])
        + (attribute(element, kAXChildrenAttribute) as? [AXUIElement] ?? []).flatMap(collect)
    }
    return collect(window)
  }

  private func attribute(_ element: AXUIElement, _ key: String) -> CFTypeRef? {
    var value: CFTypeRef?
    guard AXUIElementCopyAttributeValue(element, key as CFString, &value) == .success else {
      return nil
    }
    return value
  }

  @MainActor private func attribute(named name: String, in host: ViewHarness, key: String) throws
    -> CFTypeRef?
  {
    attribute(try XCTUnwrap(elements(named: name, in: host).first), key)
  }
}
