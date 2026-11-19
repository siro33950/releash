import AppKit
import ApplicationServices
import Connect
import SwiftProtobuf
import SwiftUI
import XCTest

@testable import Releash

final class SourceControlAcceptanceTests: IntegrationTestCase {
  @MainActor private func attribute(_ element: AXUIElement, _ name: String) -> CFTypeRef? {
    var value: CFTypeRef?
    AXUIElementCopyAttributeValue(element, name as CFString, &value)
    return value
  }

  @MainActor private func descendants(_ element: AXUIElement) -> [AXUIElement] {
    [element]
      + (attribute(element, kAXChildrenAttribute) as? [AXUIElement] ?? [])
      .flatMap { descendants($0) }
  }

  @MainActor private func text(_ element: AXUIElement) -> String {
    [kAXTitleAttribute, kAXDescriptionAttribute, kAXValueAttribute].compactMap {
      let value = attribute(element, $0)
      return value as? String ?? (value as? NSNumber)?.stringValue
    }.joined(separator: " ").replacingOccurrences(of: "-", with: "−")
  }

  @MainActor private func hasText(_ value: String, in element: AXUIElement) -> Bool {
    if ["STAGED CHANGES", "CHANGES", "CONFLICTS"].contains(value) {
      return [kAXTitleAttribute, kAXDescriptionAttribute, kAXValueAttribute].contains {
        guard let raw = attribute(element, $0) as? String else { return false }
        return raw == value || raw.hasPrefix(value + " ")
      }
    }
    return text(element).range(
      of: "(?<![\\p{L}\\p{N}_])"
        + NSRegularExpression.escapedPattern(for: value.replacingOccurrences(of: "-", with: "−"))
        + "(?![\\p{L}\\p{N}_])", options: .regularExpression) != nil
  }

  @MainActor private func windowElement(_ host: ViewHarness) throws -> AXUIElement {
    let app = AXUIElementCreateApplication(ProcessInfo.processInfo.processIdentifier)
    return try XCTUnwrap(
      (attribute(app, kAXWindowsAttribute) as? [AXUIElement])?.first {
        attribute($0, kAXTitleAttribute) as? String == host.window.title
      })
  }

  @MainActor private func parents(of element: AXUIElement) -> [AXUIElement] {
    var result = [element]
    var current = element
    while let parent = attribute(current, kAXParentAttribute) {
      current = parent as! AXUIElement
      result.append(current)
      if attribute(current, kAXRoleAttribute) as? String == kAXWindowRole { break }
    }
    return result
  }

  @MainActor private func section(_ name: String, in host: ViewHarness) throws -> AXUIElement {
    let headings = ["STAGED CHANGES", "CHANGES", "CONFLICTS"]
    func heading(_ element: AXUIElement) -> String? {
      let value =
        attribute(element, kAXValueAttribute) as? String
        ?? attribute(element, kAXTitleAttribute) as? String
        ?? attribute(element, kAXDescriptionAttribute) as? String ?? ""
      return headings.first { value == $0 || value.hasPrefix($0 + " ") }
    }
    let anchor = try XCTUnwrap(descendants(windowElement(host)).first { heading($0) == name })
    return parents(of: anchor).prefix { candidate in
      !descendants(candidate).contains { element in
        heading(element).map { $0 != name } ?? false
      }
    }.last ?? anchor
  }

  @MainActor private func assertSection(
    _ name: String, files: [String], excluding: [String] = [], in host: ViewHarness
  ) throws {
    let elements = descendants(try section(name, in: host))
    for file in files {
      XCTAssertTrue(
        elements.contains { hasText(file, in: $0) }, "\(name): \(file) \(host.debugDump())")
    }
    for file in excluding { XCTAssertFalse(elements.contains { hasText(file, in: $0) }, file) }
  }

  @MainActor private func rowNames(
    in sectionName: String, among names: [String], host: ViewHarness
  ) throws -> [String] {
    let root = try section(sectionName, in: host)
    return descendants(root).compactMap { element in
      guard let name = names.first(where: { hasText($0, in: element) }),
        !descendants(element).dropFirst().contains(where: { hasText(name, in: $0) })
      else { return nil }
      return name
    }
  }

  @MainActor private func hasFile(
    _ file: String, section name: String, in host: ViewHarness
  ) throws -> Bool {
    let root = try windowElement(host)
    guard descendants(root).contains(where: { hasText(name, in: $0) }) else { return false }
    return descendants(try section(name, in: host)).contains { hasText(file, in: $0) }
  }

  @MainActor private func assertGroup(
    _ anchor: String, contains expected: [String], excluding: [String] = [],
    in root: AXUIElement
  ) throws {
    let element = try XCTUnwrap(descendants(root).first { hasText(anchor, in: $0) })
    var candidates: [AXUIElement] = []
    for candidate in parents(of: element) {
      candidates.append(candidate)
      if CFEqual(candidate, root) { break }
    }
    let group = candidates.first { candidate in
      let contents = descendants(candidate)
      return expected.allSatisfy { value in contents.contains { hasText(value, in: $0) } }
        && excluding.allSatisfy { value in !contents.contains { hasText(value, in: $0) } }
    }
    XCTAssertNotNil(group, "\(anchor): \(expected)")
  }

  @MainActor private func assertConflictCount(_ count: Int, in host: ViewHarness) throws {
    let root = try windowElement(host)
    let banner = try XCTUnwrap(
      descendants(root).first {
        hasText("main への rebase の途中です", in: $0)
      })
    let pattern =
      "(?:conflicts?|競合)[^0-9]{0,12}" + String(count)
      + "(?![0-9])|(?<![0-9])" + String(count) + "\\s*(?:件(?:の)?\\s*)?(?:conflicts?|競合)"
    let found = parents(of: banner).contains { candidate in
      let contents = descendants(candidate)
      guard !contents.contains(where: { hasText("CONFLICTS", in: $0) }) else { return false }
      return contents.map { text($0) }.joined(separator: " ").range(
        of: pattern, options: .regularExpression) != nil
    }
    XCTAssertTrue(found, "帯に conflict が \(count) 件であることを表示する")
  }

  @MainActor private func pressAction(
    _ action: String, near anchor: String, in host: ViewHarness, section sectionName: String? = nil
  ) async throws {
    let root = try sectionName.map { try section($0, in: host) } ?? windowElement(host)
    let element = try XCTUnwrap(descendants(root).first { hasText(anchor, in: $0) })
    let button = parents(of: element).lazy.compactMap { candidate in
      self.descendants(candidate).first {
        guard self.hasText(action, in: $0) else { return false }
        var names: CFArray?
        AXUIElementCopyActionNames($0, &names)
        return (names as? [String] ?? []).contains(kAXPressAction)
      }
    }.first
    XCTAssertEqual(
      AXUIElementPerformAction(try XCTUnwrap(button), kAXPressAction as CFString), .success)
    await host.refresh()
  }

  @MainActor private func menu(
    _ host: ViewHarness, selecting choice: String? = nil,
    inspect: @escaping @MainActor ([NSMenuItem]) -> Void = { _ in }
  ) async throws {
    let probe = SourceControlMenuProbe()
    NotificationCenter.default.addObserver(
      probe, selector: #selector(SourceControlMenuProbe.began(_:)),
      name: NSMenu.didBeginTrackingNotification, object: nil)
    defer { NotificationCenter.default.removeObserver(probe) }
    try await host.menu("▾", selecting: nil) { _ in
      inspect(probe.menu?.items.filter { !$0.isSeparatorItem } ?? [])
    }
    if let choice {
      let menu = try XCTUnwrap(probe.menu)
      let index = try XCTUnwrap(menu.items.firstIndex { commandName($0.title) == choice })
      XCTAssertTrue(menu.items[index].isEnabled, choice)
      guard menu.items[index].isEnabled else { return }
      menu.performActionForItem(at: index)
      await host.refresh()
    }
  }

  private func commandName(_ title: String) -> String {
    title.components(separatedBy: CharacterSet(charactersIn: "↑↓")).first?
      .trimmingCharacters(in: .whitespacesAndNewlines) ?? title
  }

  @MainActor private func messageEditor(_ view: NSView) -> NSTextView? {
    if let editor = view as? NSTextView, !editor.isFieldEditor { return editor }
    return view.subviews.lazy.compactMap(messageEditor).first
  }

  @MainActor private func enterMessage(_ value: String, in host: ViewHarness) async throws {
    let editor: NSTextView
    if let area = messageEditor(try XCTUnwrap(host.window.contentView)) {
      XCTAssertTrue(host.window.makeFirstResponder(area))
      editor = area
    } else {
      try await host.focusTextField(index: 1)
      editor = try XCTUnwrap(host.window.firstResponder as? NSTextView)
    }
    editor.insertText(
      value, replacementRange: NSRange(location: 0, length: editor.string.utf16.count))
    await host.refresh()
  }

  @MainActor private func dialogText(_ host: ViewHarness) throws -> String {
    func strings(_ view: NSView) -> [String] {
      if let field = view as? NSTextField { return [field.stringValue] }
      if let field = view as? NSTextView { return [field.string] }
      return view.subviews.flatMap(strings)
    }
    return strings(try XCTUnwrap(host.window.sheets.first?.contentView)).joined(separator: " ")
  }

  private func warnsIrreversible(_ text: String) -> Bool {
    ["取り消せません", "取り消せない", "取り消すことはできません", "元に戻せません"]
      .contains { text.contains($0) }
  }

  private func change(
    _ name: String, kind: Releash_Client_V1_SourceControlChangeKind = .modified,
    additions: UInt32 = 1, deletions: UInt32 = 0
  ) -> Releash_Client_V1_SourceControlChange {
    .with {
      $0.path = name
      $0.name = name
      $0.kind = kind
      $0.additions = additions
      $0.deletions = deletions
    }
  }

  @MainActor private func show(
    _ snapshot: Releash_Client_V1_SourceControlSnapshot
  ) async throws -> (AppModel, ViewHarness, TestHTTPClient) {
    let http = TestHTTPClient()
    let model = try await testModel(http)
    model.selectWorktree("/repo")
    let host = await ViewHarness(
      RightPanelView(expanded: .constant(true), model: model),
      size: NSSize(width: 600, height: 1000))
    try await host.press("right-panel:sourceControl")
    try await eventually {
      http.state.withLock { $0.subscriptions.contains { $0.target == "source-control" } }
    }
    guard http.state.withLock({ $0.subscriptions.contains { $0.target == "source-control" } })
    else {
      host.close()
      await model.stop()
      throw NSError(
        domain: "SourceControlAcceptance", code: 1,
        userInfo: [NSLocalizedDescriptionKey: "Source Control の購読が開始されませんでした"])
    }
    try http.publish(.with { $0.sourceControl = snapshot }, target: "source-control")
    try await eventuallyView(host) {
      let names =
        snapshot.staged.map(\.name) + snapshot.unstaged.map(\.name) + snapshot.conflicts.map(\.name)
      return names.allSatisfy { host.element($0) != nil }
        && host.texts().contains(
          snapshot.changedFileCount == 0 ? "変更なし" : "\(snapshot.changedFileCount) ファイルの変更")
        && (snapshot.operation.message.isEmpty || host.texts().contains(snapshot.operation.message))
    }
    return (model, host, http)
  }

  // B-001, B-004
  @MainActor func test_SourceControl_変更数と区画とupstreamを表示する() async throws {
    // Given
    var snapshot = Releash_Client_V1_SourceControlSnapshot()
    snapshot.changedFileCount = 5
    snapshot.branch = "feature/thread-ui"
    snapshot.tracking = .with {
      $0.upstream = "origin/feature/thread-ui"
      $0.ahead = 2
      $0.aheadAfterCommit = 3
    }
    snapshot.staged = [
      .with {
        $0.path = "src/bridge.js"
        $0.name = "bridge.js"
        $0.directory = "src"
        $0.kind = .modified
        $0.additions = 23
        $0.deletions = 9
      },
      .with {
        $0.path = "src/added.js"
        $0.name = "added.js"
        $0.directory = "src"
        $0.kind = .added
        $0.additions = 1
      },
    ]
    snapshot.unstaged = [
      .with {
        $0.path = "FileTabPane.swift"
        $0.name = "FileTabPane.swift"
        $0.kind = .modified
        $0.additions = 1
        $0.deletions = 1
      },
      .with {
        $0.path = "deleted.js"
        $0.name = "deleted.js"
        $0.kind = .deleted
        $0.deletions = 1
      },
      .with {
        $0.path = "thread-card.css"
        $0.name = "thread-card.css"
        $0.kind = .untracked
        $0.additions = 1
      },
    ]
    snapshot.canCommit = true
    // When
    let (model, host, _) = try await show(snapshot)
    defer { host.close() }
    // Then
    for text in [
      "5 ファイルの変更", "STAGED CHANGES", "CHANGES", "bridge.js", "added.js",
      "FileTabPane.swift", "deleted.js", "thread-card.css", "src", "M", "A", "D", "U",
      "feature/thread-ui", "origin/feature/thread-ui", "↑ 2", "↓ 0",
    ] {
      try await eventuallyView(host) { host.texts().contains(text) }
    }
    let root = try windowElement(host)
    try assertGroup("STAGED CHANGES", contains: ["2"], excluding: ["CHANGES"], in: root)
    try assertGroup("CHANGES", contains: ["3"], excluding: ["STAGED CHANGES"], in: root)
    try assertSection(
      "STAGED CHANGES", files: ["bridge.js", "added.js"],
      excluding: ["FileTabPane.swift", "deleted.js", "thread-card.css"], in: host)
    try assertSection(
      "CHANGES", files: ["FileTabPane.swift", "deleted.js", "thread-card.css"],
      excluding: ["bridge.js", "added.js"], in: host)
    let names = ["bridge.js", "added.js", "FileTabPane.swift", "deleted.js", "thread-card.css"]
    XCTAssertEqual(
      try rowNames(in: "STAGED CHANGES", among: names, host: host).sorted(),
      ["added.js", "bridge.js"])
    XCTAssertEqual(
      try rowNames(in: "CHANGES", among: names, host: host).sorted(),
      ["FileTabPane.swift", "deleted.js", "thread-card.css"])
    for (name, values) in [
      ("bridge.js", ["M", "src", "+23", "−9"]),
      ("added.js", ["A", "src", "+1", "−0"]),
      ("FileTabPane.swift", ["M", "+1", "−1"]),
      ("deleted.js", ["D", "+0", "−1"]),
      ("thread-card.css", ["U", "+1", "−0"]),
    ] {
      let others = ["bridge.js", "added.js", "FileTabPane.swift", "deleted.js", "thread-card.css"]
        .filter { $0 != name }
      try assertGroup(name, contains: values, excluding: others, in: root)
    }
    await model.stop()
  }

  // B-008
  @MainActor func test_SourceControl_stage済み変更かメッセージが無ければcommit操作を選べない() async throws {
    for (staged, message) in [(false, "commit message"), (true, ""), (true, " \t\n\r　")] {
      // Given
      let snapshot = Releash_Client_V1_SourceControlSnapshot.with {
        $0.changedFileCount = 1
        $0.canCommit = staged
        if staged { $0.staged = [change("file.txt")] } else { $0.unstaged = [change("file.txt")] }
      }
      let (model, host, _) = try await show(snapshot)
      defer { host.close() }
      try await enterMessage(message, in: host)
      // When / Then
      try await eventuallyView(host) { host.element("Commit") != nil }
      XCTAssertFalse(try XCTUnwrap(host.element("Commit")).enabled)
      try await menu(host) { items in
        for name in ["Commit", "Commit & Push", "Commit & Sync"] {
          let item = items.first { self.commandName($0.title) == name }
          XCTAssertNotNil(item, name)
          XCTAssertEqual(item?.isEnabled, false, name)
        }
      }
      await model.stop()
    }
  }

  // B-024
  @MainActor func test_SourceControl_変更なしと送れるcommit数を表示する() async throws {
    // Given
    var snapshot = Releash_Client_V1_SourceControlSnapshot.with {
      $0.branch = "feature/thread-ui"
      $0.tracking = .with {
        $0.upstream = "origin/feature/thread-ui"
        $0.ahead = 2
        $0.aheadAfterCommit = 3
      }
    }
    // When
    let (model, host, http) = try await show(snapshot)
    defer { host.close() }
    // Then
    for text in ["変更なし", "Worktree に変更はありません", "Push（2 commit）"] {
      try await eventuallyView(host) { host.element(text) != nil }
    }
    XCTAssertFalse(try XCTUnwrap(host.element("Commit")).enabled)
    try await menu(host) { items in
      for name in ["Push", "Pull", "Sync", "Fetch"] {
        XCTAssertEqual(items.first { self.commandName($0.title) == name }?.isEnabled, true, name)
      }
    }
    // When
    try await host.press("Push（2 commit）")
    // Then
    try await eventually { http.requests("GitPush").count == 1 }
    let request = try Releash_Client_V1_GitPushRequest(
      serializedBytes: XCTUnwrap(http.requests("GitPush").first?.message))
    XCTAssertEqual(request.worktreePath, "/repo")
    XCTAssertFalse(request.forceWithLease)
    snapshot.tracking.ahead = 0
    snapshot.tracking.aheadAfterCommit = 1
    try http.publish(.with { $0.sourceControl = snapshot }, target: "source-control")
    try await eventuallyView(host) { host.element("Push（2 commit）") == nil }
    await model.stop()
  }

  // B-025
  @MainActor func test_SourceControl_rebase中のconflictと解決後の続行可否を表示する() async throws {
    // Given
    let snapshot = Releash_Client_V1_SourceControlSnapshot.with {
      $0.changedFileCount = 2
      $0.branch = "feature/thread-ui"
      $0.operation = .with {
        $0.rebase = .with {
          $0.onto = "main"
          $0.step = 2
          $0.total = 3
        }
        $0.conflictCount = 2
        $0.message = "second commit"
      }
      $0.conflicts = ["bridge.js", "other.js"].map { name in
        .with {
          $0.path = "src/\(name)"
          $0.name = name
          $0.directory = "src"
          $0.kind = .bothModified
        }
      }
    }
    // When
    let (model, host, _) = try await show(snapshot)
    defer { host.close() }
    // Then
    for text in [
      "main への rebase の途中です", "3 commit 中 2 つ目", "中止", "feature/thread-ui", "rebase 中",
      "CONFLICTS", "bridge.js", "other.js", "src", "両方で変更", "Rebase を続ける",
      "conflict をすべて解決すると続けられます", "second commit",
    ] {
      try await eventuallyView(host) { host.texts().contains(text) }
    }
    XCTAssertFalse(try XCTUnwrap(host.element("Rebase を続ける")).enabled)
    XCTAssertNil(host.element("origin/feature/thread-ui"))
    XCTAssertNil(host.element("Commit"))
    XCTAssertNil(host.element("▾"))
    XCTAssertFalse(host.texts().contains { $0.contains("↑") || $0.contains("↓") })
    let root = try windowElement(host)
    try assertGroup("CONFLICTS", contains: ["2"], excluding: ["main への rebase の途中です"], in: root)
    try assertConflictCount(2, in: host)
    for name in ["bridge.js", "other.js"] {
      try assertGroup(
        name, contains: ["+"], excluding: [name == "bridge.js" ? "other.js" : "bridge.js"], in: root
      )
    }
    await model.stop()
  }

  // B-028
  @MainActor func test_SourceControl_merge中の帯とメッセージと続行を表示する() async throws {
    // Given
    let snapshot = Releash_Client_V1_SourceControlSnapshot.with {
      $0.branch = "feature/thread-ui"
      $0.operation = .with {
        $0.merge = .init()
        $0.message = "Merge main into feature/thread-ui"
        $0.canContinue = true
      }
    }
    // When
    let (model, host, _) = try await show(snapshot)
    defer { host.close() }
    // Then
    for text in ["中止", "Merge を続ける", "Merge main into feature/thread-ui"] {
      try await eventuallyView(host) { host.element(text) != nil }
    }
    XCTAssertTrue(try XCTUnwrap(host.element("Merge を続ける")).enabled)
    XCTAssertNil(host.element("Commit"))
    XCTAssertNil(host.element("▾"))
    await model.stop()
  }
  // B-005
  @MainActor func test_SourceControl_upstreamが無ければ未送信数とPublishBranchを表示する() async throws {
    // Given
    var snapshot = Releash_Client_V1_SourceControlSnapshot.with {
      $0.branch = "feature/thread-ui"
      $0.unpublished.unpushedCommits = 2
    }
    // When
    let (model, host, http) = try await show(snapshot)
    defer { host.close() }
    // Then
    for text in ["feature/thread-ui", "origin にまだありません（2 commit）", "Publish Branch"] {
      try await eventuallyView(host) { host.element(text) != nil }
    }
    XCTAssertFalse(host.texts().contains { $0.contains("↑") || $0.contains("↓") })
    // When
    try await host.press("Publish Branch")
    // Then
    try await eventually { http.requests("GitPush").count == 1 }
    let request = try Releash_Client_V1_GitPushRequest(
      serializedBytes: XCTUnwrap(http.requests("GitPush").first?.message))
    XCTAssertEqual(request.worktreePath, "/repo")
    XCTAssertFalse(request.forceWithLease)
    snapshot.tracking = .with { $0.upstream = "origin/feature/thread-ui" }
    try http.publish(.with { $0.sourceControl = snapshot }, target: "source-control")
    try await eventuallyView(host) {
      ["origin/feature/thread-ui", "↑ 0", "↓ 0"].allSatisfy { host.element($0) != nil }
    }
    await model.stop()
  }

  // B-009
  @MainActor func test_SourceControl_commitメニューに操作順と送受信数が出る() async throws {
    // Given
    let snapshot = Releash_Client_V1_SourceControlSnapshot.with {
      $0.changedFileCount = 1
      $0.branch = "feature/thread-ui"
      $0.tracking = .with {
        $0.upstream = "origin/feature/thread-ui"
        $0.ahead = 2
        $0.aheadAfterCommit = 3
      }
      $0.canCommit = true
      $0.staged = [
        .with {
          $0.path = "file.txt"
          $0.name = "file.txt"
          $0.kind = .modified
        }
      ]
    }
    let (model, host, http) = try await show(snapshot)
    defer { host.close() }
    try await enterMessage("commit message", in: host)
    // When / Then
    try await host.menu("▾", selecting: nil) { titles in
      let items = titles.filter { !$0.isEmpty }
      XCTAssertEqual(items.count, 7)
      guard items.count == 7 else { return }
      for (title, name) in zip(
        items, ["Commit", "Commit & Push", "Commit & Sync", "Push", "Pull", "Sync", "Fetch"])
      {
        XCTAssertTrue(title.hasPrefix(name), title)
      }
      for index in [0, 6] {
        XCTAssertFalse(items[index].contains("↑") || items[index].contains("↓"))
      }
      for (index, counts) in [
        (1, ["↑ 3"]), (2, ["↓ 0", "↑ 3"]), (3, ["↑ 2"]),
        (4, ["↓ 0"]), (5, ["↓ 0", "↑ 2"]),
      ] {
        for count in counts { XCTAssertTrue(items[index].contains(count), items[index]) }
      }
    }
    // When
    try await menu(host, selecting: "Commit")
    // Then
    try await eventually { http.requests("GitCommit").count == 1 }
    let request = try Releash_Client_V1_GitCommitRequest(
      serializedBytes: XCTUnwrap(http.requests("GitCommit").first?.message))
    XCTAssertEqual(request.worktreePath, "/repo")
    XCTAssertEqual(request.message, "commit message")
    XCTAssertEqual(request.followUp, .none)
    await model.stop()
  }

  // B-011, B-012, B-013
  @MainActor func test_SourceControl_メニューからPullとSyncとFetchを送る() async throws {
    for (choice, method) in [("Pull", "GitPull"), ("Sync", "GitSync"), ("Fetch", "GitFetch")] {
      // Given
      let snapshot = Releash_Client_V1_SourceControlSnapshot.with {
        $0.branch = "feature/thread-ui"
        $0.tracking = .with {
          $0.upstream = "origin/feature/thread-ui"
          $0.ahead = choice == "Sync" ? 1 : 0
          $0.behind = choice == "Fetch" ? 0 : 1
        }
      }
      let (model, host, http) = try await show(snapshot)
      defer { host.close() }
      // When
      try await menu(host, selecting: choice)
      // Then
      try await eventually { http.requests(method).count == 1 }
      let bytes = try XCTUnwrap(http.requests(method).first?.message)
      switch choice {
      case "Pull":
        XCTAssertEqual(
          try Releash_Client_V1_GitPullRequest(serializedBytes: bytes).worktreePath, "/repo")
      case "Sync":
        XCTAssertEqual(
          try Releash_Client_V1_GitSyncRequest(serializedBytes: bytes).worktreePath, "/repo")
      default:
        XCTAssertEqual(
          try Releash_Client_V1_GitFetchRequest(serializedBytes: bytes).worktreePath, "/repo")
      }
      await model.stop()
    }
  }

  // B-014
  @MainActor func test_SourceControl_メニューからCommit後のPushとSyncを送る() async throws {
    for followUp: Releash_Client_V1_GitCommitFollowUp in [.push, .sync] {
      // Given
      let snapshot = Releash_Client_V1_SourceControlSnapshot.with {
        $0.branch = "feature/thread-ui"
        $0.tracking.upstream = "origin/feature/thread-ui"
        $0.changedFileCount = 1
        $0.staged = [change("file.txt")]
        $0.canCommit = true
      }
      let (model, host, http) = try await show(snapshot)
      defer { host.close() }
      try await enterMessage("commit message", in: host)
      // When
      try await menu(host, selecting: followUp == .push ? "Commit & Push" : "Commit & Sync")
      // Then
      try await eventually { http.requests("GitCommit").count == 1 }
      let request = try Releash_Client_V1_GitCommitRequest(
        serializedBytes: XCTUnwrap(http.requests("GitCommit").first?.message))
      XCTAssertEqual(request.worktreePath, "/repo")
      XCTAssertEqual(request.message, "commit message")
      XCTAssertEqual(request.followUp, followUp)
      await model.stop()
    }
  }

  // B-006
  @MainActor func test_SourceControl_行をstageしてCommitすると入力欄が空になる() async throws {
    // Given
    var snapshot = Releash_Client_V1_SourceControlSnapshot.with {
      $0.branch = "feature/thread-ui"
      $0.tracking.upstream = "origin/feature/thread-ui"
      $0.changedFileCount = 1
      $0.unstaged = [change("file.txt")]
    }
    let (model, host, http) = try await show(snapshot)
    defer { host.close() }
    // When
    try await pressAction("stage", near: "file.txt", in: host, section: "CHANGES")
    // Then
    try await eventually { http.requests("GitStage").count == 1 }
    let stage = try Releash_Client_V1_GitStageRequest(
      serializedBytes: XCTUnwrap(http.requests("GitStage").first?.message))
    XCTAssertEqual(stage.repoPath, "/repo")
    XCTAssertEqual(stage.paths.items, ["file.txt"])
    snapshot.staged = snapshot.unstaged
    snapshot.unstaged = []
    snapshot.canCommit = true
    try http.publish(.with { $0.sourceControl = snapshot }, target: "source-control")
    try await eventuallyView(host) {
      (try? self.hasFile("file.txt", section: "STAGED CHANGES", in: host)) == true
        && (try? self.hasFile("file.txt", section: "CHANGES", in: host)) == false
    }
    // When
    try await enterMessage("commit message", in: host)
    try await eventuallyView(host) { host.element("Commit")?.enabled == true }
    try await host.press("Commit")
    // Then
    try await eventually { http.requests("GitCommit").count == 1 }
    let commit = try Releash_Client_V1_GitCommitRequest(
      serializedBytes: XCTUnwrap(http.requests("GitCommit").first?.message))
    XCTAssertEqual(commit.worktreePath, "/repo")
    XCTAssertEqual(commit.message, "commit message")
    XCTAssertEqual(commit.followUp, .none)
    let content = try XCTUnwrap(host.window.contentView)
    try await eventuallyView(host) {
      self.messageEditor(content)?.isEditable == true && self.messageEditor(content)?.string == ""
    }
    XCTAssertEqual(messageEditor(content)?.string, "")
    snapshot.staged = []
    snapshot.changedFileCount = 0
    snapshot.canCommit = false
    snapshot.tracking.ahead = 1
    try http.publish(.with { $0.sourceControl = snapshot }, target: "source-control")
    try await eventuallyView(host) {
      host.element("file.txt") == nil && host.element("↑ 1") != nil
    }
    await model.stop()
  }

  // B-016
  @MainActor func test_SourceControl_push失敗の理由を表示する() async throws {
    // Given
    let snapshot = Releash_Client_V1_SourceControlSnapshot.with {
      $0.branch = "main"
      $0.tracking = .with {
        $0.upstream = "origin/main"
        $0.behind = 1
      }
    }
    let (model, host, http) = try await show(snapshot)
    defer { host.close() }
    http.state.withLock { $0.failedMethod = "GitPush" }
    // When
    try await menu(host, selecting: "Push")
    // Then
    try await eventually { model.failure?.contains("GitPush failed") == true }
    await model.stop()
  }

  @MainActor func test_SourceControl_ファイル面でpushが失敗しても理由を知らせる() async throws {
    // Given
    let snapshot = Releash_Client_V1_SourceControlSnapshot.with {
      $0.branch = "main"
      $0.tracking.upstream = "origin/main"
      $0.changedFileCount = 1
      $0.unstaged = [change("bridge.js")]
    }
    let (model, host, http) = try await show(snapshot)
    defer { host.close() }
    try await eventually { model.layoutLoaded }
    http.state.withLock { $0.deferGitPush = true }
    // When
    try await menu(host, selecting: "Push")
    try await eventually { http.state.withLock { $0.pendingGitPush != nil } }
    try await host.press("bridge.js")
    try await eventuallyView(host) { host.element("right-panel:files")?.value == "1" }
    let respond = try XCTUnwrap(http.state.withLock { $0.pendingGitPush })
    respond(
      HTTPResponse(
        code: .internalError, headers: [:], message: nil, trailers: [:],
        error: ConnectError(code: .internalError, message: "GitPush failed"), tracingInfo: nil))
    // Then
    try await eventually { model.failure?.contains("GitPush failed") == true }
    await model.stop()
  }

  // B-003: 確認の取りやめと了承を、画面と公開 RPC で確かめる。
  @MainActor func test_SourceControl_forcePushは確認後だけ送信する() async throws {
    // Given
    let snapshot = Releash_Client_V1_SourceControlSnapshot.with {
      $0.branch = "main"
      $0.tracking.upstream = "origin/main"
    }
    let (model, host, http) = try await show(snapshot)
    defer { host.close() }
    // When
    try await host.menu("…", selecting: "Force Push") { _ in }
    // Then
    try await eventually { !host.window.sheets.isEmpty }
    XCTAssertTrue(http.requests("GitPush").isEmpty)
    // When
    try await host.pressAlertButton("キャンセル")
    // Then
    XCTAssertTrue(http.requests("GitPush").isEmpty)
    // When
    try await eventually { host.window.sheets.isEmpty }
    try await host.menu("…", selecting: "Force Push") { _ in }
    try await eventually { !host.window.sheets.isEmpty }
    try await host.pressAlertButton("Force Push")
    // Then
    try await eventually { http.requests("GitPush").count == 1 }
    let request = try Releash_Client_V1_GitPushRequest(
      serializedBytes: XCTUnwrap(http.requests("GitPush").first?.message))
    XCTAssertTrue(request.forceWithLease)
    XCTAssertEqual(request.worktreePath, "/repo")
    await model.stop()
  }
  // B-002
  @MainActor func test_SourceControl_絞り込み入力が購読に渡り一覧から一致しない行が消える() async throws {
    // Given
    var snapshot = Releash_Client_V1_SourceControlSnapshot.with {
      $0.changedFileCount = 2
      $0.unstaged = [change("bridge.js"), change("FileTabPane.swift")]
    }
    let (model, host, http) = try await show(snapshot)
    defer { host.close() }
    try await eventuallyView(host) { host.element("FileTabPane.swift") != nil }
    // When
    try await host.focusTextField(index: 0)
    let editor = try XCTUnwrap(host.window.firstResponder as? NSTextView)
    editor.insertText(
      "bridge", replacementRange: NSRange(location: 0, length: editor.string.utf16.count))
    // Then
    let args = ["/repo", "search=bridge"]
    try await eventually {
      http.state.withLock {
        $0.subscriptions.contains { $0.target == "source-control" && $0.args == args }
      }
    }
    snapshot.unstaged = [change("bridge.js")]
    try http.publish(.with { $0.sourceControl = snapshot }, target: "source-control", args: args)
    try await eventuallyView(host) {
      host.element("bridge.js") != nil && host.element("FileTabPane.swift") == nil
    }
    // When
    try await host.focusTextField(index: 0)
    let blankEditor = try XCTUnwrap(host.window.firstResponder as? NSTextView)
    blankEditor.insertText(
      " 　", replacementRange: NSRange(location: 0, length: blankEditor.string.utf16.count))
    await host.refresh()
    // Then
    try await eventually {
      http.state.withLock {
        $0.subscriptions.last(where: { $0.target == "source-control" })?.args == ["/repo"]
      }
    }
    snapshot.unstaged = [change("bridge.js"), change("FileTabPane.swift")]
    try http.publish(
      .with { $0.sourceControl = snapshot }, target: "source-control", args: ["/repo"])
    try await eventuallyView(host) { host.element("FileTabPane.swift") != nil }
    await model.stop()
  }

  // B-020
  @MainActor func test_SourceControl_行を開くとファイル面でHEADとの差分を表示する() async throws {
    // Given
    let snapshot = Releash_Client_V1_SourceControlSnapshot.with {
      $0.changedFileCount = 1
      $0.unstaged = [change("bridge.js")]
    }
    let (model, host, http) = try await show(snapshot)
    defer { host.close() }
    try await eventually { model.layoutLoaded }
    // When
    let filename = try XCTUnwrap(
      descendants(windowElement(host)).first {
        hasText("bridge.js", in: $0) && attribute($0, kAXRoleAttribute) as? String == kAXButtonRole
      })
    let row = try XCTUnwrap(
      parents(of: filename).dropFirst().first { element in
        var actions: CFArray?
        AXUIElementCopyActionNames(element, &actions)
        return (actions as? [String] ?? []).contains(kAXPressAction)
      })
    XCTAssertEqual(AXUIElementPerformAction(row, kAXPressAction as CFString), .success)
    // Then
    let args = ["/repo", "bridge.js", "head"]
    try await eventually {
      http.state.withLock {
        $0.subscriptions.contains { $0.target == "file-tab" && $0.args == args }
      }
    }
    var payload = Releash_Client_V1_StatePayload()
    payload.fileTab.initialMode = "diff"
    payload.fileTab.view.textDiff.path = "bridge.js"
    payload.fileTab.view.textDiff.original = "const before = 1;\n"
    payload.fileTab.view.textDiff.modified = "const after = 2;\n"
    try http.publish(payload, target: "file-tab", args: args)
    try await eventuallyView(host) {
      host.element("file-path")?.label == "bridge.js"
        || host.element("file-path")?.value == "bridge.js"
    }
    XCTAssertEqual(host.element("right-panel:files")?.value, "1")
    XCTAssertEqual(host.element("file-mode")?.value, "1")
    XCTAssertEqual(host.element("HEAD と比較")?.value, "1")
    XCTAssertNotNil(host.element("bridge.js"))
    await model.stop()
  }

  // B-021, B-017
  @MainActor func test_SourceControl_行の破棄は警告し取りやめでは送信せず了承後もstage済みを残す() async throws {
    // Given
    var snapshot = Releash_Client_V1_SourceControlSnapshot.with {
      $0.changedFileCount = 1
      $0.staged = [change("bridge.js", additions: 2, deletions: 1)]
      $0.unstaged = [change("bridge.js", additions: 23, deletions: 9)]
    }
    let (model, host, http) = try await show(snapshot)
    defer { host.close() }
    try assertSection("STAGED CHANGES", files: ["bridge.js"], in: host)
    try assertSection("CHANGES", files: ["bridge.js"], in: host)
    try assertGroup("bridge.js", contains: ["+2", "−1"], in: section("STAGED CHANGES", in: host))
    try assertGroup("bridge.js", contains: ["+23", "−9"], in: section("CHANGES", in: host))
    // When
    try await pressAction("破棄", near: "bridge.js", in: host, section: "CHANGES")
    // Then
    try await eventually { !host.window.sheets.isEmpty }
    let warning = try dialogText(host)
    XCTAssertTrue(warning.contains("bridge.js"), warning)
    XCTAssertTrue(warning.contains("+23"), warning)
    XCTAssertTrue(warning.replacingOccurrences(of: "-", with: "−").contains("−9"), warning)
    XCTAssertTrue(warnsIrreversible(warning), warning)
    XCTAssertTrue(http.requests("GitDiscard").isEmpty)
    // When
    try await host.pressAlertButton("キャンセル")
    // Then
    XCTAssertTrue(http.requests("GitDiscard").isEmpty)
    try assertSection("CHANGES", files: ["bridge.js"], in: host)
    try assertGroup("bridge.js", contains: ["+23", "−9"], in: section("CHANGES", in: host))
    // When
    try await pressAction("破棄", near: "bridge.js", in: host, section: "CHANGES")
    try await eventually { !host.window.sheets.isEmpty }
    try await host.pressAlertButton("破棄")
    // Then
    try await eventually { http.requests("GitDiscard").count == 1 }
    let request = try Releash_Client_V1_GitDiscardRequest(
      serializedBytes: XCTUnwrap(http.requests("GitDiscard").first?.message))
    XCTAssertEqual(request.worktreePath, "/repo")
    XCTAssertEqual(request.paths, ["bridge.js"])
    snapshot.unstaged = []
    try http.publish(.with { $0.sourceControl = snapshot }, target: "source-control")
    try await eventuallyView(host) {
      (try? self.hasFile("bridge.js", section: "CHANGES", in: host)) == false
    }
    try assertSection("STAGED CHANGES", files: ["bridge.js"], in: host)
    try assertGroup("bridge.js", contains: ["+2", "−1"], in: section("STAGED CHANGES", in: host))
    await model.stop()
  }

  // B-022, B-023
  @MainActor func test_SourceControl_未追跡ファイルと区画全体の破棄に対応する警告を表示する() async throws {
    for all in [false, true] {
      // Given
      var snapshot = Releash_Client_V1_SourceControlSnapshot.with {
        $0.changedFileCount = all ? 3 : 1
        $0.unstaged = [change("thread-card.css", kind: .untracked, additions: 7)]
        $0.discardAll.trashedFiles = 1
        if all {
          $0.unstaged += [
            change("bridge.js", additions: 23, deletions: 9),
            change("other.js", additions: 4, deletions: 2),
          ]
          $0.discardAll.additions = 27
          $0.discardAll.deletions = 11
        }
      }
      let (model, host, http) = try await show(snapshot)
      defer { host.close() }
      // When
      try await pressAction("破棄", near: all ? "CHANGES" : "thread-card.css", in: host)
      // Then
      try await eventually { !host.window.sheets.isEmpty }
      let warning = try dialogText(host)
      XCTAssertTrue(warning.contains("ゴミ箱"), warning)
      if all {
        XCTAssertTrue(warning.contains("+27"), warning)
        XCTAssertTrue(warning.replacingOccurrences(of: "-", with: "−").contains("−11"), warning)
        XCTAssertTrue(warnsIrreversible(warning), warning)
      } else {
        XCTAssertTrue(warning.contains("thread-card.css"), warning)
        XCTAssertFalse(warnsIrreversible(warning), warning)
      }
      // When
      try await host.pressAlertButton("破棄")
      // Then
      try await eventually { http.requests("GitDiscard").count == 1 }
      let request = try Releash_Client_V1_GitDiscardRequest(
        serializedBytes: XCTUnwrap(http.requests("GitDiscard").first?.message))
      XCTAssertEqual(request.worktreePath, "/repo")
      XCTAssertEqual(Set(request.paths), Set(snapshot.unstaged.map(\.path)))
      snapshot.unstaged = []
      snapshot.changedFileCount = 0
      try http.publish(.with { $0.sourceControl = snapshot }, target: "source-control")
      try await eventuallyView(host) {
        host.element("thread-card.css") == nil && host.element("bridge.js") == nil
          && host.element("other.js") == nil
      }
      await model.stop()
    }
  }

  // B-018, B-019
  @MainActor func test_SourceControl_行のunstageと区画全体のstage切替を操作する() async throws {
    // Given
    var snapshot = Releash_Client_V1_SourceControlSnapshot.with {
      $0.changedFileCount = 1
      $0.staged = [change("file.txt")]
      $0.canCommit = true
    }
    let (model, host, http) = try await show(snapshot)
    defer { host.close() }
    // When
    try await pressAction("unstage", near: "file.txt", in: host)
    // Then
    try await eventually { http.requests("GitUnstage").count == 1 }
    let rowRequest = try Releash_Client_V1_GitUnstageRequest(
      serializedBytes: XCTUnwrap(http.requests("GitUnstage").first?.message))
    XCTAssertEqual(rowRequest.repoPath, "/repo")
    XCTAssertEqual(rowRequest.paths.items, ["file.txt"])
    snapshot.unstaged = snapshot.staged
    snapshot.staged = []
    snapshot.canCommit = false
    try http.publish(.with { $0.sourceControl = snapshot }, target: "source-control")
    try await eventuallyView(host) {
      (try? self.hasFile("file.txt", section: "STAGED CHANGES", in: host)) == false
    }
    try assertSection("CHANGES", files: ["file.txt"], in: host)
    // Given
    snapshot.unstaged += [change("second.txt"), change("third.txt")]
    snapshot.changedFileCount = 3
    try http.publish(.with { $0.sourceControl = snapshot }, target: "source-control")
    try await eventuallyView(host) { host.element("third.txt") != nil }
    // When
    try await pressAction("stage", near: "CHANGES", in: host)
    // Then
    try await eventually { http.requests("GitStage").count == 1 }
    let stageRequest = try Releash_Client_V1_GitStageRequest(
      serializedBytes: XCTUnwrap(http.requests("GitStage").first?.message))
    let files = ["file.txt", "second.txt", "third.txt"]
    XCTAssertEqual(stageRequest.repoPath, "/repo")
    XCTAssertTrue(stageRequest.paths.items.isEmpty || Set(stageRequest.paths.items) == Set(files))
    snapshot.staged = snapshot.unstaged
    snapshot.unstaged = []
    snapshot.canCommit = true
    try http.publish(.with { $0.sourceControl = snapshot }, target: "source-control")
    try await eventuallyView(host) {
      (try? self.hasFile("file.txt", section: "CHANGES", in: host)) == false
    }
    try assertSection("STAGED CHANGES", files: files, in: host)
    for file in files { XCTAssertFalse(try hasFile(file, section: "CHANGES", in: host)) }
    // When
    try await pressAction("unstage", near: "STAGED CHANGES", in: host)
    // Then
    try await eventually { http.requests("GitUnstage").count == 2 }
    let unstageRequest = try Releash_Client_V1_GitUnstageRequest(
      serializedBytes: XCTUnwrap(http.requests("GitUnstage").last?.message))
    XCTAssertEqual(unstageRequest.repoPath, "/repo")
    XCTAssertTrue(
      unstageRequest.paths.items.isEmpty || Set(unstageRequest.paths.items) == Set(files))
    snapshot.unstaged = snapshot.staged
    snapshot.staged = []
    snapshot.canCommit = false
    try http.publish(.with { $0.sourceControl = snapshot }, target: "source-control")
    try await eventuallyView(host) {
      (try? self.hasFile("file.txt", section: "STAGED CHANGES", in: host)) == false
    }
    try assertSection("CHANGES", files: files, in: host)
    for file in files { XCTAssertFalse(try hasFile(file, section: "STAGED CHANGES", in: host)) }
    await model.stop()
  }

  // B-026
  @MainActor func test_SourceControl_conflict行の解決操作がstageを送り続行できる表示に変わる() async throws {
    // Given
    var snapshot = Releash_Client_V1_SourceControlSnapshot.with {
      $0.changedFileCount = 1
      $0.branch = "feature/thread-ui"
      $0.operation = .with {
        $0.rebase = .with {
          $0.onto = "main"
          $0.step = 2
          $0.total = 3
        }
        $0.conflictCount = 1
        $0.message = "second commit"
      }
      $0.conflicts = [
        .with {
          $0.path = "bridge.js"
          $0.name = "bridge.js"
          $0.kind = .bothModified
        }
      ]
    }
    let (model, host, http) = try await show(snapshot)
    defer { host.close() }
    // When
    try await pressAction("+", near: "bridge.js", in: host)
    // Then
    try await eventually { http.requests("GitStage").count == 1 }
    let request = try Releash_Client_V1_GitStageRequest(
      serializedBytes: XCTUnwrap(http.requests("GitStage").first?.message))
    XCTAssertEqual(request.repoPath, "/repo")
    XCTAssertEqual(request.paths.items, ["bridge.js"])
    snapshot.conflicts = []
    snapshot.staged = [change("bridge.js")]
    snapshot.operation.conflictCount = 0
    snapshot.operation.canContinue = true
    try http.publish(.with { $0.sourceControl = snapshot }, target: "source-control")
    try await eventuallyView(host) { host.element("Rebase を続ける")?.enabled == true }
    XCTAssertNil(host.element("conflict をすべて解決すると続けられます"))
    XCTAssertFalse(try hasFile("bridge.js", section: "CONFLICTS", in: host))
    try assertSection("STAGED CHANGES", files: ["bridge.js"], in: host)
    await model.stop()
  }

  // B-032
  @MainActor func test_SourceControl_breakで停止したrebaseはメッセージを入力せず続行できる() async throws {
    // Given
    var snapshot = Releash_Client_V1_SourceControlSnapshot.with {
      $0.branch = "feature/thread-ui"
      $0.operation = .with {
        $0.rebase = .with {
          $0.onto = "main"
          $0.step = 1
          $0.total = 1
        }
        $0.canContinue = true
      }
    }
    // When
    let (model, host, http) = try await show(snapshot)
    defer { host.close() }
    // Then
    try await eventuallyView(host) { host.element("main への rebase の途中です") != nil }
    XCTAssertNotNil(host.element("中止"))
    let editor = try XCTUnwrap(messageEditor(try XCTUnwrap(host.window.contentView)))
    XCTAssertEqual(editor.string, "")
    XCTAssertFalse(editor.isEditable)
    XCTAssertTrue(try XCTUnwrap(host.element("Rebase を続ける")).enabled)
    // When
    try await host.press("Rebase を続ける")
    // Then
    try await eventually { http.requests("GitContinueOperation").count == 1 }
    let request = try Releash_Client_V1_GitContinueOperationRequest(
      serializedBytes: XCTUnwrap(http.requests("GitContinueOperation").first?.message))
    XCTAssertEqual(request.message, "")
    snapshot.tracking = .with { $0.upstream = "origin/feature/thread-ui" }
    try http.publish(.with { $0.sourceControl = snapshot }, target: "source-control")
    try await eventuallyView(host) { host.element("Rebase を続ける") == nil }
    XCTAssertNil(host.element("main への rebase の途中です"))
    XCTAssertNil(host.element("中止"))
    await model.stop()
  }

  // B-027, B-030
  @MainActor func test_SourceControl_続行ボタンが入力メッセージを送り完了後に通常表示へ戻る() async throws {
    for rebase in [true, false] {
      // Given
      var snapshot = Releash_Client_V1_SourceControlSnapshot.with {
        $0.changedFileCount = 1
        $0.branch = "feature/thread-ui"
        $0.operation = .with {
          if rebase {
            $0.rebase = .with {
              $0.onto = "main"
              $0.step = 3
              $0.total = 3
            }
          } else {
            $0.merge = .init()
          }
          $0.canContinue = true
          $0.message = rebase ? "third commit" : "merge commit"
        }
        $0.staged = [change("bridge.js")]
      }
      let (model, host, http) = try await show(snapshot)
      defer { host.close() }
      // When
      let message = rebase ? "rewritten third" : "merge commit"
      if rebase { try await enterMessage(message, in: host) }
      try await host.press(rebase ? "Rebase を続ける" : "Merge を続ける")
      // Then
      try await eventually { http.requests("GitContinueOperation").count == 1 }
      let request = try Releash_Client_V1_GitContinueOperationRequest(
        serializedBytes: XCTUnwrap(http.requests("GitContinueOperation").first?.message))
      XCTAssertEqual(request.worktreePath, "/repo")
      XCTAssertEqual(request.message, message)
      snapshot.changedFileCount = 0
      snapshot.tracking = .with { $0.upstream = "origin/feature/thread-ui" }
      snapshot.staged = []
      try http.publish(.with { $0.sourceControl = snapshot }, target: "source-control")
      try await eventuallyView(host) { host.element("Commit") != nil }
      XCTAssertNil(host.element("中止"))
      XCTAssertNil(host.element("Rebase を続ける"))
      XCTAssertNil(host.element("Merge を続ける"))
      await model.stop()
    }
  }

  // B-027
  @MainActor func test_SourceControl_rebase続行先の進捗とconflictと失敗理由を表示する() async throws {
    // Given
    var snapshot = Releash_Client_V1_SourceControlSnapshot.with {
      $0.changedFileCount = 1
      $0.branch = "feature/thread-ui"
      $0.operation = .with {
        $0.rebase = .with {
          $0.onto = "main"
          $0.step = 2
          $0.total = 3
        }
        $0.canContinue = true
        $0.message = "second commit"
      }
      $0.staged = [change("bridge.js")]
    }
    let (model, host, http) = try await show(snapshot)
    defer { host.close() }
    http.state.withLock { $0.failedMethod = "GitContinueOperation" }
    // When
    try await host.press("Rebase を続ける")
    // Then
    try await eventually { http.requests("GitContinueOperation").count == 1 }
    snapshot.operation.rebase.step = 3
    snapshot.operation.message = "third commit"
    snapshot.operation.canContinue = false
    snapshot.operation.conflictCount = 1
    snapshot.staged = []
    snapshot.conflicts = [
      .with {
        $0.path = "next.js"
        $0.name = "next.js"
        $0.kind = .bothModified
      }
    ]
    try http.publish(.with { $0.sourceControl = snapshot }, target: "source-control")
    try await eventuallyView(host) {
      host.element("3 commit 中 3 つ目") != nil && host.element("next.js") != nil
        && model.failure?.contains("GitContinueOperation failed") == true
    }
    try assertGroup(
      "CONFLICTS", contains: ["1"], excluding: ["main への rebase の途中です"], in: windowElement(host))
    try assertConflictCount(1, in: host)
    await model.stop()
  }

  // B-029
  @MainActor func test_SourceControl_mergeとrebaseを中止すると帯とconflictが消える() async throws {
    for rebase in [true, false] {
      // Given
      var snapshot = Releash_Client_V1_SourceControlSnapshot.with {
        $0.changedFileCount = 1
        $0.branch = "feature/thread-ui"
        $0.operation = .with {
          if rebase {
            $0.rebase = .with {
              $0.onto = "main"
              $0.step = 2
              $0.total = 3
            }
          } else {
            $0.merge = .init()
          }
          $0.conflictCount = 1
          $0.message = "interrupted"
        }
        $0.conflicts = [
          .with {
            $0.path = "bridge.js"
            $0.name = "bridge.js"
            $0.kind = .bothModified
          }
        ]
      }
      let (model, host, http) = try await show(snapshot)
      defer { host.close() }
      // When
      try await host.press("中止")
      // Then
      try await eventually { http.requests("GitAbortOperation").count == 1 }
      let request = try Releash_Client_V1_GitAbortOperationRequest(
        serializedBytes: XCTUnwrap(http.requests("GitAbortOperation").first?.message))
      XCTAssertEqual(request.worktreePath, "/repo")
      snapshot.changedFileCount = 0
      snapshot.tracking = .with { $0.upstream = "origin/feature/thread-ui" }
      snapshot.conflicts = []
      try http.publish(.with { $0.sourceControl = snapshot }, target: "source-control")
      try await eventuallyView(host) { host.element("Commit") != nil }
      XCTAssertNil(host.element("中止"))
      XCTAssertNil(host.element("CONFLICTS"))
      XCTAssertNil(host.element("Rebase を続ける"))
      XCTAssertNil(host.element("Merge を続ける"))
      await model.stop()
    }
  }

  @MainActor func test_SourceControl_ファイル面から戻ると入力したメッセージが残る() async throws {
    // Given
    let snapshot = Releash_Client_V1_SourceControlSnapshot.with {
      $0.changedFileCount = 1
      $0.unstaged = [change("bridge.js")]
    }
    let (model, host, http) = try await show(snapshot)
    defer { host.close() }
    try await eventually { model.layoutLoaded }
    try await enterMessage("draft message", in: host)
    // When
    try await host.press("bridge.js")
    try await eventuallyView(host) { host.element("right-panel:files")?.value == "1" }
    try await host.press("right-panel:sourceControl")
    try await eventually {
      http.state.withLock {
        $0.subscriptions.filter { $0.target == "source-control" && $0.args == ["/repo"] }.count == 2
      }
    }
    try http.publish(
      .with { $0.sourceControl = snapshot }, target: "source-control", args: ["/repo"])
    // Then
    let content = try XCTUnwrap(host.window.contentView)
    try await eventuallyView(host) {
      host.element("bridge.js") != nil && self.messageEditor(content)?.string == "draft message"
    }
    await model.stop()
  }

  @MainActor func test_SourceControl_merge中に書き換えたメッセージがパネルを戻っても残る() async throws {
    // Given
    let snapshot = Releash_Client_V1_SourceControlSnapshot.with {
      $0.operation.merge = .init()
      $0.operation.message = "merge message"
    }
    let (model, host, http) = try await show(snapshot)
    defer { host.close() }
    try await enterMessage("edited merge message", in: host)
    // When
    try await host.press("right-panel:roots")
    try await eventuallyView(host) { host.element("Merge を続ける") == nil }
    try await host.press("right-panel:sourceControl")
    try await eventually {
      http.state.withLock {
        $0.subscriptions.filter { $0.target == "source-control" && $0.args == ["/repo"] }.count == 2
      }
    }
    try http.publish(
      .with { $0.sourceControl = snapshot }, target: "source-control", args: ["/repo"])
    // Then
    let content = try XCTUnwrap(host.window.contentView)
    try await eventuallyView(host) {
      host.element("Merge を続ける") != nil
        && self.messageEditor(content)?.string == "edited merge message"
    }
    await model.stop()
  }

  @MainActor func test_SourceControl_再購読の初回snapshotを待つ間に入力したメッセージはmerge終了後も残る() async throws {
    // Given
    var snapshot = Releash_Client_V1_SourceControlSnapshot.with {
      $0.operation.merge = .init()
      $0.operation.message = "merge message"
    }
    let (model, host, http) = try await show(snapshot)
    defer { host.close() }
    // When
    try await host.press("right-panel:roots")
    try await eventuallyView(host) { host.element("Merge を続ける") == nil }
    try await host.press("right-panel:sourceControl")
    try await eventually {
      http.state.withLock {
        $0.subscriptions.filter { $0.target == "source-control" && $0.args == ["/repo"] }.count == 2
      }
    }
    try await enterMessage("new commit message", in: host)
    snapshot.branchState = .tracking(.with { $0.upstream = "origin/main" })
    try http.publish(
      .with { $0.sourceControl = snapshot }, target: "source-control", args: ["/repo"])
    // Then
    let content = try XCTUnwrap(host.window.contentView)
    try await eventuallyView(host) {
      host.texts().contains("origin/main") && host.element("Merge を続ける") == nil
        && self.messageEditor(content)?.string == "new commit message"
    }
    await model.stop()
  }

  @MainActor func test_SourceControl_初回snapshot前にメッセージ入力と絞り込みをしてもmerge終了後にメッセージが残る() async throws {
    // Given
    var snapshot = Releash_Client_V1_SourceControlSnapshot.with {
      $0.operation.merge = .init()
      $0.operation.message = "merge message"
    }
    let (model, host, http) = try await show(snapshot)
    defer { host.close() }
    // When
    try await host.press("right-panel:roots")
    try await eventuallyView(host) { host.element("Merge を続ける") == nil }
    snapshot.branchState = .tracking(.with { $0.upstream = "origin/main" })
    try await host.press("right-panel:sourceControl")
    try await eventually {
      http.state.withLock {
        $0.subscriptions.filter { $0.target == "source-control" && $0.args == ["/repo"] }.count == 2
      }
    }
    try await enterMessage("new commit message", in: host)
    try await host.focusTextField(index: 0)
    let editor = try XCTUnwrap(host.window.firstResponder as? NSTextView)
    editor.insertText(
      "bridge", replacementRange: NSRange(location: 0, length: editor.string.utf16.count))
    let args = ["/repo", "search=bridge"]
    try await eventually {
      http.state.withLock {
        $0.subscriptions.contains { $0.target == "source-control" && $0.args == args }
      }
    }
    try http.publish(
      .with { $0.sourceControl = snapshot }, target: "source-control", args: args)
    // Then
    let content = try XCTUnwrap(host.window.contentView)
    try await eventuallyView(host) {
      host.texts().contains("origin/main") && host.element("Merge を続ける") == nil
        && self.messageEditor(content)?.string == "new commit message"
    }
    await model.stop()
  }

  @MainActor func test_SourceControl_再購読の初回snapshotまで入力しなければmerge終了後にメッセージが空になる() async throws {
    // Given
    var snapshot = Releash_Client_V1_SourceControlSnapshot.with {
      $0.operation.merge = .init()
      $0.operation.message = "merge message"
    }
    let (model, host, http) = try await show(snapshot)
    defer { host.close() }
    // When
    try await host.press("right-panel:roots")
    try await eventuallyView(host) { host.element("Merge を続ける") == nil }
    try await host.press("right-panel:sourceControl")
    try await eventually {
      http.state.withLock {
        $0.subscriptions.filter { $0.target == "source-control" && $0.args == ["/repo"] }.count == 2
      }
    }
    snapshot.branchState = .tracking(.with { $0.upstream = "origin/main" })
    try http.publish(
      .with { $0.sourceControl = snapshot }, target: "source-control", args: ["/repo"])
    // Then
    let content = try XCTUnwrap(host.window.contentView)
    try await eventuallyView(host) {
      host.texts().contains("origin/main") && host.element("Merge を続ける") == nil
        && self.messageEditor(content)?.string == ""
    }
    await model.stop()
  }

  @MainActor func test_SourceControl_commit後の失敗応答より先に届いたmergeメッセージを残す() async throws {
    // Given
    var snapshot = Releash_Client_V1_SourceControlSnapshot.with {
      $0.branch = "main"
      $0.tracking.upstream = "origin/main"
      $0.changedFileCount = 1
      $0.staged = [change("file.txt")]
      $0.canCommit = true
    }
    let (model, host, http) = try await show(snapshot)
    defer { host.close() }
    http.state.withLock { $0.deferGitCommit = true }
    try await enterMessage("sent message", in: host)
    // When
    try await menu(host, selecting: "Commit & Sync")
    try await eventually { http.state.withLock { $0.pendingGitCommit != nil } }
    snapshot.operation = .with {
      $0.merge = .init()
      $0.message = "merge message"
      $0.conflictCount = 1
    }
    try http.publish(
      .with { $0.sourceControl = snapshot }, target: "source-control", args: ["/repo"])
    let content = try XCTUnwrap(host.window.contentView)
    try await eventuallyView(host) { self.messageEditor(content)?.string == "merge message" }
    let detail = Releash_Client_V1_CommandError.with {
      $0.coded.code = "GIT_COMMIT_FOLLOW_UP_FAILED"
      $0.coded.message = "Pull conflict"
    }
    let failure = ConnectError.Detail(
      type: Releash_Client_V1_CommandError.protoMessageName, payload: try detail.serializedData())
    let respond = try XCTUnwrap(http.state.withLock { $0.pendingGitCommit })
    respond(
      HTTPResponse(
        code: .internalError, headers: [:], message: nil, trailers: [:],
        error: ConnectError(code: .internalError, message: "GitCommit failed", details: [failure]),
        tracingInfo: nil))
    // Then
    try await eventuallyView(host) {
      model.failure == "Pull conflict" && self.messageEditor(content)?.isEditable == true
    }
    XCTAssertEqual(messageEditor(content)?.string, "merge message")
    await model.stop()
  }

  @MainActor func test_SourceControl_commitの応答待ち中はメッセージを編集できない() async throws {
    for choice in ["Commit", "Commit & Push", "Commit & Sync"] {
      // Given
      let snapshot = Releash_Client_V1_SourceControlSnapshot.with {
        $0.branch = "main"
        $0.tracking.upstream = "origin/main"
        $0.changedFileCount = 1
        $0.staged = [change("file.txt")]
        $0.canCommit = true
      }
      let (model, host, http) = try await show(snapshot)
      defer { host.close() }
      http.state.withLock { $0.deferGitCommit = true }
      try await enterMessage("sent message", in: host)
      let editor = try XCTUnwrap(host.window.firstResponder as? NSTextView)
      XCTAssertTrue(editor.isEditable)
      try await eventuallyView(host) { host.element("Commit")?.enabled == true }
      // When
      if choice == "Commit" {
        try await host.press("Commit")
      } else {
        try await menu(host, selecting: choice)
      }
      try await eventually { http.state.withLock { $0.pendingGitCommit != nil } }
      // Then
      let content = try XCTUnwrap(host.window.contentView)
      try await eventuallyView(host) { self.messageEditor(content)?.isEditable == false }
      XCTAssertEqual(messageEditor(content)?.string, "sent message")
      let respond = try XCTUnwrap(http.state.withLock { $0.pendingGitCommit })
      respond(
        HTTPResponse(
          code: .ok, headers: ["content-type": ["application/proto"]], message: Data(),
          trailers: [:], error: nil, tracingInfo: nil))
      try await eventuallyView(host) {
        self.messageEditor(content)?.isEditable == true && self.messageEditor(content)?.string == ""
      }
      await model.stop()
    }
  }

  @MainActor func test_SourceControl_commit後の失敗だけ入力欄を消して理由を表示する() async throws {
    for code in ["GIT_COMMIT_FOLLOW_UP_FAILED", "OTHER_FAILURE"] {
      // Given
      let snapshot = Releash_Client_V1_SourceControlSnapshot.with {
        $0.branch = "main"
        $0.tracking.upstream = "origin/main"
        $0.changedFileCount = 1
        $0.staged = [change("file.txt")]
        $0.canCommit = true
      }
      let (model, host, http) = try await show(snapshot)
      defer { host.close() }
      let detail = Releash_Client_V1_CommandError.with {
        $0.coded.code = code
        $0.coded.message = "Push rejected"
      }
      let failure = ConnectError.Detail(
        type: Releash_Client_V1_CommandError.protoMessageName, payload: try detail.serializedData())
      http.state.withLock {
        $0.failedMethod = "GitCommit"
        $0.failureDetail = failure
      }
      try await enterMessage("commit message", in: host)
      // When
      try await menu(host, selecting: "Commit & Push")
      // Then
      let content = try XCTUnwrap(host.window.contentView)
      let expectedMessage = code == "GIT_COMMIT_FOLLOW_UP_FAILED" ? "" : "commit message"
      try await eventuallyView(host) {
        model.failure == "Push rejected" && self.messageEditor(content)?.isEditable == true
          && self.messageEditor(content)?.string == expectedMessage
      }
      XCTAssertEqual(messageEditor(content)?.string, expectedMessage)
      await model.stop()
    }
  }
}

@MainActor private final class SourceControlMenuProbe: NSObject {
  var menu: NSMenu?
  @objc func began(_ notification: Notification) { menu = notification.object as? NSMenu }
}
