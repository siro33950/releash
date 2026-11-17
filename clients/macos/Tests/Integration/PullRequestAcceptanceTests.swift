import AppKit
import ApplicationServices
import Connect
import SwiftProtobuf
import SwiftUI
import XCTest

@testable import Releash

final class PullRequestAcceptanceTests: XCTestCase {
  @MainActor func test_PR作成欄_baseとdraftと作成ボタンが同じ行に並ぶ() async throws {
    // Given
    try await withPanel { http, _, host in
      // When
      try await creationForm(http, host)
      // Then
      let title = try screenFrame(find("Title", host, role: kAXStaticTextRole))
      let input = try screenFrame(find("Title", host, editable: true))
      XCTAssertLessThan(title.maxY, input.minY)
      let base = try frame("base", host)
      let draft = try frame("draft で作る", host)
      let create = try frame("PR を作成", host)
      XCTAssertEqual(base.midY, draft.midY, accuracy: 8)
      XCTAssertEqual(draft.midY, create.midY, accuracy: 8)
      XCTAssertGreaterThan(create.minX, draft.maxX)
      XCTAssertEqual(
        create.maxX, host.window.contentLayoutRect.width - UIStyle.spacing, accuracy: 12)
      try attachPanel(host, name: "PR creation layout")
    }
  }

  @MainActor func test_PR表示_件数と操作を右端に置きComment本文を名前にそろえる() async throws {
    // Given
    var pr = pullRequest()
    pr.checks.failed = 1
    pr.checks.listed = [
      .with {
        $0.name = "CI build"
        $0.bucket = .fail
        $0.failureSeconds = 240
        $0.logJobID = 123
      }
    ]
    pr.comments = [comment("Reviewer", body: "Review text", secondsAgo: 3600)]
    try await withPanel { http, _, host in
      // When
      try await show(pr, http, host)
      // Then
      let count = try frame("失敗 1", host)
      let log = try frame("ログを開く", host)
      let author = try frame("Reviewer", host)
      let body = try frame("Review text", host)
      XCTAssertEqual(body.minX, author.minX, accuracy: 2)
      XCTAssertGreaterThan(count.minX, host.window.contentLayoutRect.width * 0.8)
      XCTAssertEqual(log.maxX, count.maxX, accuracy: 12)
      let input = try screenFrame(find("pr-comment", host, editable: true))
      let submit = try screenFrame(find("Comment", host, role: kAXButtonRole))
      XCTAssertGreaterThan(input.height, 48)
      XCTAssertEqual(submit.maxX, input.maxX, accuracy: 4)
      XCTAssertEqual(
        attribute(try find("pr-comment", host, editable: true), kAXPlaceholderValueAttribute)
          as? String,
        "Comment を書く…（Markdown）")
      try attachPanel(host, name: "PR content layout")
    }
  }

  @MainActor func test_B002_B004_PRを選ぶと作成欄が出る() async throws {
    // Given
    try await withPanel { http, _, host in
      // When
      try await host.press("PR")
      try await deliver(.with { $0.absent = .init() }, http: http)
      // Then
      try await eventuallyView(host) {
        host.element("このブランチの PR はありません") != nil
      }
      for label in ["Title", "Description（任意）", "base", "draft で作る", "PR を作成"] {
        XCTAssertNotNil(host.element(label), host.debugDump())
      }
    }
  }

  @MainActor func test_B003_Worktreeを替えるとPR無しの表示に替わる() async throws {
    // Given
    try await withPanel { http, model, host in
      try await host.press("PR")
      try await deliver(.with { $0.present = pullRequest() }, http: http)
      try await eventuallyView(host) { host.element("Thread 面を作り直す") != nil }
      // When
      model.selectWorktree("/repo/b")
      try await deliver(.with { $0.absent = .init() }, http: http, path: "/repo/b")
      // Then
      try await eventuallyView(host) {
        host.element("このブランチの PR はありません") != nil
          && host.element("Thread 面を作り直す") == nil
      }
    }
  }

  @MainActor func test_B006_Titleが空なら作成できない() async throws {
    // Given
    try await withPanel { http, _, host in
      try await host.press("PR")
      try await deliver(.with { $0.absent = .init() }, http: http)
      // When / Then
      try await eventuallyView(host) { host.element("PR を作成") != nil }
      XCTAssertFalse(try XCTUnwrap(host.element("PR を作成")).enabled)
    }
  }

  @MainActor func test_B014_見出しにタイトルと番号が出て状態の文字は出ない() async throws {
    // Given
    try await withPanel { http, _, host in
      // When
      try await host.press("PR")
      try await deliver(.with { $0.present = pullRequest() }, http: http)
      // Then
      try await eventuallyView(host) { host.element("Thread 面を作り直す") != nil }
      XCTAssertNotNil(host.element("#2128"), host.debugDump())
      XCTAssertFalse(host.texts().contains("Open"), host.debugDump())
    }
  }

  @MainActor func test_B023_失敗と取消と実行中のcheckを表示する() async throws {
    // Given
    var pr = pullRequest()
    pr.checks.failed = 1
    pr.checks.cancelled = 1
    pr.checks.pending = 1
    pr.checks.listed = [
      .with {
        $0.name = "build failed"
        $0.bucket = .fail
        $0.failureSeconds = 240
      },
      .with {
        $0.name = "build cancelled"
        $0.bucket = .cancel
      },
      .with {
        $0.name = "build running"
        $0.bucket = .pending
        $0.startedAt = UInt64(Date().timeIntervalSince1970) - 360
      },
    ]
    try await withPanel { http, _, host in
      // When
      try await host.press("PR")
      try await deliver(.with { $0.present = pr }, http: http)
      // Then
      try await eventuallyView(host) { host.element("build running") != nil }
      for text in ["build failed", "build cancelled", "4 分で失敗", "実行中 6 分"] {
        XCTAssertNotNil(host.element(text), host.debugDump())
      }
      let icons = try ["build failed", "build cancelled", "build running"].map {
        try checkIcon(beside: $0, host).pixels
      }
      for left in icons.indices {
        for right in icons.indices where right > left {
          XCTAssertNotEqual(icons[left], icons[right], "分類ごとのアイコンを区別できない")
        }
      }
    }
  }

  @MainActor func test_B024_B025_通過とskipはたたまれ通過の行を開閉できる() async throws {
    // Given
    var pr = pullRequest()
    pr.checks.passed = 7
    pr.checks.skipped = 2
    pr.checks.passedNames = (1...7).map { "passed-check-\($0)" }
    pr.checks.skippedNames = (1...2).map { "skipped-check-\($0)" }
    try await withPanel { http, _, host in
      // When
      try await host.press("PR")
      try await deliver(.with { $0.present = pr }, http: http)
      // Then
      try await eventuallyView(host) { host.element("通過した 7 件") != nil }
      XCTAssertNotNil(host.element("skip した 2 件"), host.debugDump())
      for name in pr.checks.passedNames + pr.checks.skippedNames {
        XCTAssertNil(host.element(name), host.debugDump())
      }
      // When
      try await host.press("通過した 7 件")
      // Then
      try await eventuallyView(host) {
        pr.checks.passedNames.allSatisfy { host.element($0) != nil }
      }
      // When
      try await host.press("通過した 7 件")
      // Then
      try await eventuallyView(host) {
        host.element("通過した 7 件") != nil
          && pr.checks.passedNames.allSatisfy { host.element($0) == nil }
      }
    }
  }

  @MainActor func test_B027_外部サービスの失敗にはログを開く操作が出ない() async throws {
    // Given
    var pr = pullRequest()
    pr.checks.failed = 1
    pr.checks.listed = [
      .with {
        $0.name = "external-service"
        $0.bucket = .fail
      }
    ]
    try await withPanel { http, _, host in
      // When
      try await host.press("PR")
      try await deliver(.with { $0.present = pr }, http: http)
      // Then
      try await eventuallyView(host) { host.element("external-service") != nil }
      XCTAssertNil(host.element("ログを開く"), host.debugDump())
    }
  }

  @MainActor func test_B031_Commentが空なら投稿できない() async throws {
    // Given
    try await withPanel { http, _, host in
      try await host.press("PR")
      try await deliver(.with { $0.present = pullRequest() }, http: http)
      // When / Then
      try await eventuallyView(host) { host.element("Comment") != nil }
      let comment = try XCTUnwrap(
        elements(host).first {
          attribute($0, kAXRoleAttribute) as? String == kAXButtonRole
            && (attribute($0, kAXTitleAttribute) as? String
              ?? attribute($0, kAXDescriptionAttribute) as? String) == "Comment"
        })
      XCTAssertEqual(attribute(comment, kAXEnabledAttribute) as? Bool, false)
    }
  }

  @MainActor func test_B035_内容を取得できない理由がPRの面に出る() async throws {
    // Given
    try await withPanel { http, _, host in
      // When
      try await host.press("PR")
      try await deliver(.with { $0.readError = "GitHub authentication expired" }, http: http)
      // Then
      try await eventuallyView(host) {
        host.element("GitHub authentication expired") != nil
      }
    }
  }

  @MainActor func test_B001_PRのアイコンがSourceControlとThreadの間に出る() async throws {
    // Given / When
    try await withPanel { _, _, host in
      // Then
      try await eventuallyView(host) { host.element("PR") != nil }
      let pr = try frame("PR", host)
      XCTAssertLessThan(try frame("Source Control", host).maxX, pr.midX)
      XCTAssertLessThan(pr.midX, try frame("Thread", host).minX)
      try assertIcon("git-pull-request", host: host, region: pr)
    }
  }

  @MainActor func test_B005_baseはmainが選ばれoriginのブランチを選べる() async throws {
    // Given
    try await withPanel { http, _, host in
      // When
      try await creationForm(http, host)
      // Then
      try await eventuallyView(host) { host.element("base")?.value == "main" }
      XCTAssertEqual(try value("base", host), "main")
      try await MenuHarness().open(press: { try await host.press("base") }) { menu in
        XCTAssertEqual(
          Set(menu.items.filter { !$0.isSeparatorItem }.map(\.title)), ["main", "develop"])
      }
    }
  }

  @MainActor func test_B007_選んだbaseと入力内容で作成を要求しできたPRに替わる() async throws {
    // Given
    try await withPanel { http, _, host in
      try await creationForm(http, host)
      try await edit("Title", text: "New PR title", host)
      try await edit("Description（任意）", text: "New PR description", host)
      try await MenuHarness().open(selection: ["develop"], press: { try await host.press("base") })
      // When
      try await pressButton("PR を作成", host)
      // Then
      let request = try await creationRequest(http)
      XCTAssertEqual(request.worktreePath, "/repo/a")
      XCTAssertEqual(request.base, "develop")
      XCTAssertEqual(request.title, "New PR title")
      XCTAssertEqual(request.body, "New PR description")
      var created = pullRequest()
      created.title = "New PR title"
      created.body = "New PR description"
      try await deliver(.with { $0.present = created }, http: http)
      try await assertDetail(created, host)
    }
  }

  @MainActor func test_B008_draftで作成を要求しDraftアイコンを表示する() async throws {
    // Given
    try await withPanel { http, _, host in
      try await creationForm(http, host)
      try await edit("Title", text: "Draft PR", host)
      try await host.press("draft で作る")
      // When
      try await pressButton("PR を作成", host)
      // Then
      let request = try await creationRequest(http)
      XCTAssertTrue(request.draft)
      var pr = pullRequest()
      pr.state = .draft
      pr.title = "Draft PR"
      try await deliver(.with { $0.present = pr }, http: http)
      try await eventuallyView(host) {
        host.element(pr.title) != nil && host.element("PR を作成") == nil
      }
      try assertIcon("git-pull-request-draft", host: host, region: header(pr, host), color: .gray)
    }
  }

  @MainActor func test_B010_作成を拒まれた理由が出て入力内容が残る() async throws {
    try await assertCreationFailure("GitHub refused PR creation")
  }

  @MainActor func test_B013_見出し本文CHECKS_COMMENTSの順で下端に投稿欄が出る() async throws {
    // Given
    let pr = pullRequest()
    try await withPanel { http, _, host in
      // When
      try await show(pr, http, host)
      // Then
      let names = [pr.title, pr.body, "CHECKS", "COMMENTS"]
      let rows = try names.map { try screenFrame(find($0, host)) }
      for index in 1..<rows.count { XCTAssertLessThan(rows[index - 1].midY, rows[index].midY) }
      let composer = try screenFrame(find("Comment", host, editable: true))
      let button = try screenFrame(find("Comment", host, role: kAXButtonRole))
      XCTAssertGreaterThan(composer.midY, try XCTUnwrap(rows.last).midY)
      XCTAssertGreaterThan(button.midY, try XCTUnwrap(rows.last).midY)
    }
  }

  @MainActor func test_B015_PRの状態ごとに見出しのアイコンと色が替わる() async throws {
    for (state, icon, color) in [
      (Releash_Client_V1_PullRequestState.open, "git-pull-request", Color.green),
      (.draft, "git-pull-request-draft", .gray), (.merged, "git-merge", .purple),
      (.closed, "git-pull-request-closed", .red),
    ] {
      // Given
      var pr = pullRequest()
      pr.state = state
      try await withPanel { http, _, host in
        // When
        try await show(pr, http, host)
        // Then
        try assertIcon(icon, host: host, region: header(pr, host), color: color)
      }
    }
  }

  @MainActor func test_B016_ブラウザで開く操作でPRのURLを開く() async throws {
    // Given
    let http = TestHTTPClient()
    let model = try await testModel(http)
    model.selectWorktree("/repo/a")
    var opened: [URL] = []
    let host = await ViewHarness(
      RightPanelView(expanded: .constant(true), model: model)
        .environment(
          \.openURL,
          OpenURLAction { url in
            opened.append(url)
            return .handled
          }))
    defer { host.close() }
    let pr = pullRequest()
    try await show(pr, http, host)
    // When
    try await host.press("ブラウザで開く")
    // Then
    XCTAssertEqual(opened.map(\.absoluteString), [pr.url])
    await model.stop()
  }

  @MainActor func test_B017_Markdownの箇条書きがリストとして表示される() async throws {
    // Given
    var pr = pullRequest()
    pr.body = "- first item\n- second item"
    try await withPanel { http, _, host in
      // When
      try await show(pr, http, host)
      // Then
      try await eventuallyView(host) { host.element("second item") != nil }
      XCTAssertNotNil(host.element("first item"))
      XCTAssertFalse(host.texts().contains(pr.body))
      let list = elements(host).contains {
        attribute($0, kAXRoleAttribute) as? String == kAXListRole
      }
      XCTAssertTrue(
        list || host.texts().contains { $0.contains("•") || $0.contains("◦") }, host.debugDump())
    }
  }

  @MainActor func test_B018_B019_Closedから作成欄を開いて作成したPRに替わる() async throws {
    // Given
    var closed = pullRequest()
    closed.state = .closed
    closed.canCreateNew = true
    try await withPanel { http, _, host in
      try await show(closed, http, host)
      try await deliverBases(http)
      // When
      try await host.press("新しい PR を作る")
      // Then
      try await eventuallyView(host) { host.element("Title") != nil }
      for label in ["Description（任意）", "base", "draft で作る", "PR を作成"] {
        XCTAssertNotNil(host.element(label))
      }
      try await edit("Title", text: "Replacement PR", host)
      // When
      try await pressButton("PR を作成", host)
      _ = try await creationRequest(http)
      var created = pullRequest()
      created.number = 2129
      created.title = "Replacement PR"
      try await deliver(.with { $0.present = created }, http: http)
      // Then
      try await assertDetail(created, host)
      XCTAssertNotNil(host.element("#2129"))
      XCTAssertNil(host.element("#2128"))
    }
  }

  @MainActor func test_B020_Open_Draft_Mergedには新しいPRを作る操作が無い() async throws {
    for state in [Releash_Client_V1_PullRequestState.open, .draft, .merged] {
      // Given
      var pr = pullRequest()
      pr.state = state
      pr.canCreateNew = false
      try await withPanel { http, _, host in
        // When
        try await show(pr, http, host)
        // Then
        XCTAssertNil(host.element("新しい PR を作る"), host.debugDump())
      }
    }
  }

  @MainActor func test_B021_CHECKSの見出しの右に分類ごとの件数が出る() async throws {
    // Given
    var pr = pullRequest()
    pr.checks.failed = 1
    pr.checks.cancelled = 1
    pr.checks.pending = 1
    pr.checks.skipped = 2
    pr.checks.passed = 7
    pr.checks.listed = [
      .with {
        $0.name = "failed"
        $0.bucket = .fail
      },
      .with {
        $0.name = "cancelled"
        $0.bucket = .cancel
      },
      .with {
        $0.name = "pending"
        $0.bucket = .pending
      },
    ]
    pr.checks.skippedNames = ["skip one", "skip two"]
    pr.checks.passedNames = (1...7).map { "pass \($0)" }
    try await withPanel { http, _, host in
      // When
      try await show(pr, http, host)
      // Then
      for label in ["失敗 1", "取り消し 1", "実行中 1", "skip 2", "通過 7"] {
        try assertCount(label, heading: "CHECKS", host)
      }
    }
  }

  @MainActor func test_B022_通過18件だけなら他の分類の件数は出ない() async throws {
    // Given
    var pr = pullRequest()
    pr.checks.passed = 18
    pr.checks.passedNames = (1...18).map { "pass \($0)" }
    try await withPanel { http, _, host in
      // When
      try await show(pr, http, host)
      // Then
      try assertCount("通過 18", heading: "CHECKS", host)
      for label in ["失敗 0", "取り消し 0", "実行中 0", "skip 0"] {
        XCTAssertNil(host.element(label), host.debugDump())
      }
    }
  }

  @MainActor func test_B026_失敗jobのログを中央タブに開く() async throws {
    // Given
    var pr = pullRequest()
    pr.checks.failed = 1
    pr.checks.listed = [
      .with {
        $0.name = "CI build"
        $0.bucket = .fail
        $0.logJobID = 123
      }
    ]
    try await withPanel(central: true) { http, _, host in
      try await show(pr, http, host)
      // When
      try await host.press("ログを開く")
      try await waitForSubscription("check-log", args: ["/repo/a", "123"], http)
      try http.publish(
        .with { $0.checkLog.log = "CI build failed: acceptance log" },
        target: "check-log", args: ["/repo/a", "123"])
      // Then
      try await eventuallyView(host) {
        host.texts().contains { $0.contains("CI build failed: acceptance log") }
      }
      let log = try frame("CI build failed: acceptance log", host)
      XCTAssertLessThan(log.midX, host.window.contentLayoutRect.width / 2)
    }
  }

  @MainActor func test_B029_人のCommentに頭文字と相対時刻と整形されたコードを表示する() async throws {
    // Given
    var pr = pullRequest()
    pr.comments = [comment("Alice", body: "`let value = 1`", secondsAgo: 2400)]
    try await withPanel { http, _, host in
      // When
      try await show(pr, http, host)
      // Then
      for text in ["A", "Alice", "40 分前", "let value = 1"] {
        XCTAssertNotNil(host.element(text), host.debugDump())
      }
      XCTAssertNil(host.element("bot"))
      XCTAssertNil(host.element("`let value = 1`"))
      try assertMonospaced("let value = 1", host)
    }
  }

  @MainActor func test_B030_botのCommentに絵のアイコンと名前の右のbot印が出る() async throws {
    // Given
    var pr = pullRequest()
    var bot = comment("review-bot", body: "Automated review", secondsAgo: 7200)
    bot.bot = true
    pr.comments = [bot]
    try await withPanel { http, _, host in
      // When
      try await show(pr, http, host)
      // Then
      for text in ["review-bot", "2 時間前", "Automated review", "bot"] {
        XCTAssertNotNil(host.element(text), host.debugDump())
      }
      let author = try frame("review-bot", host)
      try assertIcon(
        "bot", host: host,
        region: NSRect(x: 0, y: author.minY - 8, width: author.minX, height: author.height + 16))
      XCTAssertGreaterThan(try frame("bot", host).midX, author.maxX)
    }
  }

  @MainActor func test_B032_Commentを投稿すると末尾に出て入力欄が空に戻る() async throws {
    // Given
    var pr = pullRequest()
    pr.comments = [comment("Alice", body: "existing comment", secondsAgo: 3600)]
    try await withPanel { http, _, host in
      try await show(pr, http, host)
      try await edit("Comment", text: "posted comment", host)
      // When
      try await pressButton("Comment", host)
      // Then
      try await eventually { !http.requests("PostPullRequestComment").isEmpty }
      let request = try Releash_Client_V1_PostPullRequestCommentRequest(
        serializedBytes: XCTUnwrap(http.requests("PostPullRequestComment").last?.message))
      XCTAssertEqual(request.worktreePath, "/repo/a")
      XCTAssertEqual(request.number, 2128)
      XCTAssertEqual(request.body, "posted comment")
      pr.comments.append(comment("Alice", body: "posted comment", secondsAgo: 0))
      try await deliver(.with { $0.present = pr }, http: http)
      try await eventuallyView(host) { host.element("posted comment") != nil }
      XCTAssertGreaterThan(
        try screenFrame(find("posted comment", host)).midY,
        try screenFrame(find("existing comment", host)).midY)
      try await eventuallyView(host) { (try? value("Comment", host, editable: true)) == "" }
    }
  }

  @MainActor func test_B033_Comment投稿を拒まれると理由が出て文が残る() async throws {
    // Given
    try await withPanel { http, _, host in
      try await show(pullRequest(), http, host)
      try await edit("Comment", text: "retained comment", host)
      http.state.withLock { $0.deferredTerminalMethod = "PostPullRequestComment" }
      // When
      try await pressButton("Comment", host)
      try await reject(http, reason: "GitHub refused comment")
      // Then
      try await eventuallyView(host) { host.element("GitHub refused comment") != nil }
      XCTAssertEqual(try value("Comment", host, editable: true), "retained comment")
    }
  }

  @MainActor func test_B009_originに無いブランチはPRもpushも作らず入力を保つ() async throws {
    // Given
    try await withDaemon(github: true, existingPR: false) { fixture, _, host in
      try await host.press("PR")
      try await eventuallyView(host) { host.element("PR を作成") != nil }
      try await edit("Title", text: "Retained title", host)
      try await edit("Description（任意）", text: "Retained description", host)
      // When
      try await pressButton("PR を作成", host)
      // Then
      try await eventuallyView(host) {
        host.texts().contains { $0.localizedCaseInsensitiveContains("push") }
      }
      XCTAssertEqual(try value("Title", host, editable: true), "Retained title")
      XCTAssertEqual(try value("Description（任意）", host, editable: true), "Retained description")
      let prs =
        try JSONSerialization.jsonObject(
          with: fixture.run(fixture.gh, ["pr", "list", "--state", "all", "--json", "number"]))
        as? [[String: Any]]
      XCTAssertEqual(try XCTUnwrap(prs).count, 0)
      let refs = try fixture.git(["ls-remote", fixture.origin.path, "refs/heads/feature"])
      XCTAssertEqual(
        String(decoding: refs, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines), "")
    }
  }

  @MainActor func test_B011_B012_GitHub以外の実リポジトリでは作成欄が出て作成は失敗する() async throws {
    // Given
    try await withDaemon(github: false, existingPR: false) { _, _, host in
      // When
      try await host.press("PR")
      // Then
      try await eventuallyView(host) { host.element("このブランチの PR はありません") != nil }
      for label in ["Title", "Description（任意）", "base", "draft で作る", "PR を作成"] {
        XCTAssertNotNil(host.element(label), host.debugDump())
      }
      // Given
      try await edit("Title", text: "Non GitHub PR", host)
      // When
      try await pressButton("PR を作成", host)
      // Then
      try await eventuallyView(host) {
        host.texts().contains { $0.contains("GitHub") && $0 != "Non GitHub PR" }
      }
    }
  }

  @MainActor func test_PR作成と投稿のRPC成功で外部状態と購読表示が更新される() async throws {
    // Given
    try await withDaemon(github: true, existingPR: false) { fixture, model, host in
      try fixture.git(["push", fixture.origin.path, "feature"])
      try fixture.git(["update-ref", "refs/remotes/origin/feature", "HEAD"])
      try await host.press("PR")
      try await eventuallyView(host) { host.element("PR を作成") != nil }
      let client = try XCTUnwrap(model.client)
      var request = Releash_Client_V1_CreatePullRequestRequest()
      request.worktreePath = fixture.repository.path
      request.title = "Created through daemon"
      request.body = "Description through daemon"
      request.base = "main"
      request.draft = true
      // When
      _ = try await client.createPullRequest(request: request, headers: model.headers).result.get()
      // Then
      try await eventuallyView(host) {
        host.element(request.title) != nil && host.element(request.body) != nil
          && host.element("PR を作成") == nil
      }
      let created = try XCTUnwrap(
        JSONSerialization.jsonObject(
          with:
            fixture.run(
              fixture.gh,
              ["pr", "view", "2128", "--json", "headRefName,baseRefName,isDraft,title,body"]))
          as? [String: Any])
      XCTAssertEqual(created["headRefName"] as? String, "feature")
      XCTAssertEqual(created["baseRefName"] as? String, request.base)
      XCTAssertEqual(created["title"] as? String, request.title)
      XCTAssertEqual(created["body"] as? String, request.body)
      XCTAssertEqual(created["isDraft"] as? Bool, true)
      // Given
      var post = Releash_Client_V1_PostPullRequestCommentRequest()
      post.worktreePath = fixture.repository.path
      post.number = 2128
      post.body = "Posted through daemon"
      // When
      _ = try await client.postPullRequestComment(request: post, headers: model.headers).result
        .get()
      // Then
      try await eventuallyView(host) { host.element(post.body) != nil }
      let comments = try XCTUnwrap(
        JSONSerialization.jsonObject(
          with:
            fixture.run(fixture.gh, ["api", "repos/acceptance/repository/issues/2128/comments"]))
          as? [[String: Any]])
      XCTAssertEqual(comments.last?["body"] as? String, post.body)
      XCTAssertGreaterThan(
        try screenFrame(find(post.body, host)).midY,
        try screenFrame(find("new comment", host)).midY)
      try assertCount("4", heading: "COMMENTS", host)
    }
  }

  @MainActor func test_B028_実サーバがCommentとレビュー本文を統合し対象外を除く() async throws {
    // Given: gh fixture に issue comments、本文あり/なしの reviews、コード行 comments を用意する。
    try await withDaemon(github: true, existingPR: true) { _, _, host in
      // When
      try await host.press("PR")
      // Then
      try await eventuallyView(host) { host.element("new comment") != nil }
      let frames = try ["old comment", "review body", "new comment"].map {
        try screenFrame(find($0, host))
      }
      XCTAssertLessThan(frames[0].midY, frames[1].midY)
      XCTAssertLessThan(frames[1].midY, frames[2].midY)
      try assertCount("3", heading: "COMMENTS", host)
      XCTAssertNil(host.element("EmptyReviewer"), host.debugDump())
      XCTAssertNil(host.element("code line comment"), host.debugDump())
    }
  }

  @MainActor func test_B034_実サーバがGitHub側のcheck更新を操作なしで取り込む() async throws {
    // Given
    try await withDaemon(github: true, existingPR: true) { fixture, _, host in
      try await host.press("PR")
      try await eventuallyView(host) { host.element("running check") != nil }
      try assertCount("通過 7", heading: "CHECKS", host)
      // When: 外部の check だけを変更し、RPC や snapshot 配信で更新を促さない。
      try fixture.completeCheck()
      // Then: 30 秒の取得周期に、通信と画面反映の猶予を加えて待つ。
      let deadline = ContinuousClock.now + .seconds(35)
      repeat {
        await host.refresh()
        if host.element("running check") == nil && host.element("通過 8") != nil { break }
        try await Task.sleep(for: .milliseconds(50))
      } while ContinuousClock.now < deadline
      XCTAssertNil(host.element("running check"), host.debugDump())
      try assertCount("通過 8", heading: "CHECKS", host)
    }
  }

  @MainActor private func withDaemon(
    github: Bool, existingPR: Bool,
    _ body: (PullRequestDaemonFixture, AppModel, ViewHarness) async throws -> Void
  ) async throws {
    let fixture = try PullRequestDaemonFixture(github: github, existingPR: existingPR)
    defer { fixture.close() }
    let discovery = fixture.data.appendingPathComponent("client-api.json")
    try await eventually { FileManager.default.fileExists(atPath: discovery.path) }
    let status = try JSONSerialization.data(withJSONObject: [
      "running": true, "compatibility": "compatible", "release_compatibility": "compatible",
      "discovery_file": discovery.path,
    ])
    let model = AppModel(
      cli: CLI(
        executable: fixture.helpers.appendingPathComponent("releash"), run: { _, _ in status }))
    await model.start()
    let host = await ViewHarness(
      RightPanelView(expanded: .constant(true), model: model),
      size: NSSize(width: 500, height: 1000))
    do {
      try await eventually { model.connected }
      var add = Releash_Client_V1_AddRepoPathRequest()
      add.path = fixture.repository.path
      _ = try await XCTUnwrap(model.client).addRepoPath(request: add, headers: model.headers).result
        .get()
      model.selectWorktree(fixture.repository.path)
      try await body(fixture, model, host)
    } catch {
      host.close()
      await model.stop()
      throw error
    }
    host.close()
    await model.stop()
  }
  @MainActor private func show(
    _ pr: Releash_Client_V1_PullRequestView, _ http: TestHTTPClient, _ host: ViewHarness
  ) async throws {
    try await host.press("PR")
    try await deliver(.with { $0.present = pr }, http: http)
    try await eventuallyView(host) {
      host.element(pr.title) != nil && host.element("PR を作成") == nil
    }
  }

  @MainActor private func creationForm(_ http: TestHTTPClient, _ host: ViewHarness) async throws {
    try await host.press("PR")
    try await deliver(.with { $0.absent = .init() }, http: http)
    try await deliverBases(http)
    try await eventuallyView(host) { host.element("PR を作成") != nil }
  }

  @MainActor private func deliverBases(_ http: TestHTTPClient) async throws {
    try await waitForSubscription("pull-request-bases", args: ["/repo/a"], http)
    try http.publish(
      .with {
        $0.pullRequestBases.branches = ["develop", "main"]
        $0.pullRequestBases.defaultBranch = "main"
      }, target: "pull-request-bases", args: ["/repo/a"])
  }

  @MainActor private func creationRequest(_ http: TestHTTPClient) async throws
    -> Releash_Client_V1_CreatePullRequestRequest
  {
    try await eventually { !http.requests("CreatePullRequest").isEmpty }
    return try .init(serializedBytes: XCTUnwrap(http.requests("CreatePullRequest").last?.message))
  }

  @MainActor private func assertDetail(
    _ pr: Releash_Client_V1_PullRequestView, _ host: ViewHarness
  ) async throws {
    try await eventuallyView(host) {
      host.element(pr.title) != nil && host.element("PR を作成") == nil
    }
    for text in [pr.body, "CHECKS", "COMMENTS"] {
      XCTAssertNotNil(host.element(text), host.debugDump())
    }
    XCTAssertNil(host.element("PR を作成"))
  }

  @MainActor private func assertCreationFailure(_ reason: String) async throws {
    // Given
    try await withPanel { http, _, host in
      try await creationForm(http, host)
      try await edit("Title", text: "Retained title", host)
      try await edit("Description（任意）", text: "Retained description", host)
      http.state.withLock { $0.deferredTerminalMethod = "CreatePullRequest" }
      // When
      try await pressButton("PR を作成", host)
      try await reject(http, reason: reason)
      // Then
      try await eventuallyView(host) { host.element(reason) != nil }
      XCTAssertEqual(try value("Title", host, editable: true), "Retained title")
      XCTAssertEqual(try value("Description（任意）", host, editable: true), "Retained description")
    }
  }

  @MainActor private func reject(_ http: TestHTTPClient, reason: String) async throws {
    try await eventually { http.state.withLock { $0.pendingTerminal != nil } }
    let callback = try XCTUnwrap(
      http.state.withLock { state in
        defer { state.pendingTerminal = nil }
        return state.pendingTerminal
      })
    callback(
      HTTPResponse(
        code: .failedPrecondition, headers: [:], message: nil, trailers: [:],
        error: ConnectError(code: .failedPrecondition, message: reason), tracingInfo: nil))
  }

  private func comment(_ author: String, body: String, secondsAgo: UInt64)
    -> Releash_Client_V1_PullRequestComment
  {
    .with {
      $0.author = author
      $0.body = body
      $0.writtenAt = UInt64(Date().timeIntervalSince1970) - secondsAgo
    }
  }

  @MainActor private func waitForSubscription(
    _ target: String, args: [String], _ http: TestHTTPClient
  ) async throws {
    try await eventually {
      http.state.withLock { $0.subscriptions.contains { $0.target == target && $0.args == args } }
    }
  }

  @MainActor private func find(
    _ name: String, _ host: ViewHarness, role: String? = nil, editable: Bool = false
  ) throws -> AXUIElement {
    try XCTUnwrap(
      elements(host).first { element in
        let actualRole = attribute(element, kAXRoleAttribute) as? String
        let matches = [
          kAXIdentifierAttribute, kAXTitleAttribute, kAXDescriptionAttribute,
          kAXValueAttribute, kAXPlaceholderValueAttribute,
        ].contains {
          attribute(element, $0) as? String == name
        }
        return matches && (role == nil || role == actualRole)
          && (!editable || [kAXTextFieldRole, kAXTextAreaRole].contains(actualRole ?? ""))
      }, name + ": " + host.debugDump())
  }

  @MainActor private func value(
    _ name: String, _ host: ViewHarness, editable: Bool = false
  ) throws -> String {
    try XCTUnwrap(attribute(find(name, host, editable: editable), kAXValueAttribute) as? String)
  }

  @MainActor private func edit(_ name: String, text: String, _ host: ViewHarness) async throws {
    await host.refresh()
    let element = try find(name, host, editable: true)
    try await eventually {
      NSApp.activate()
      host.window.makeKeyAndOrderFront(nil)
      AXUIElementSetAttributeValue(element, kAXFocusedAttribute as CFString, kCFBooleanTrue)
      return (attribute(element, kAXFocusedAttribute) as? Bool) == true
        && host.window.firstResponder is NSTextView
    }
    let editor = try XCTUnwrap(host.window.firstResponder as? NSTextView)
    editor.insertText(
      text, replacementRange: NSRange(location: 0, length: editor.string.utf16.count))
    XCTAssertTrue(host.window.makeFirstResponder(nil))
    try await eventuallyView(host) { (try? value(name, host, editable: true)) == text }
  }

  @MainActor private func pressButton(_ name: String, _ host: ViewHarness) async throws {
    try await eventuallyView(host) {
      guard let button = try? find(name, host, role: kAXButtonRole) else { return false }
      return attribute(button, kAXEnabledAttribute) as? Bool == true
    }
    let button = try find(name, host, role: kAXButtonRole)
    XCTAssertEqual(AXUIElementPerformAction(button, kAXPressAction as CFString), .success)
    await host.refresh()
  }

  @MainActor private func attachPanel(_ host: ViewHarness, name: String) throws {
    let view = try XCTUnwrap(host.window.contentView)
    let bitmap = try XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: view.bounds))
    view.cacheDisplay(in: view.bounds, to: bitmap)
    let image = NSImage(size: bitmap.size)
    image.addRepresentation(bitmap)
    let attachment = XCTAttachment(image: image)
    attachment.name = name
    attachment.lifetime = .keepAlways
    add(attachment)
  }

  @MainActor private func screenFrame(_ element: AXUIElement) throws -> NSRect {
    let value = try XCTUnwrap(attribute(element, "AXFrame"))
    var rect = CGRect.zero
    XCTAssertTrue(AXValueGetValue(value as! AXValue, .cgRect, &rect))
    return rect
  }

  @MainActor private func frame(_ name: String, _ host: ViewHarness) throws -> NSRect {
    var rect = try screenFrame(find(name, host))
    rect.origin.y = try XCTUnwrap(NSScreen.screens.first).frame.height - rect.maxY
    return try XCTUnwrap(host.window.contentView).convert(
      host.window.convertFromScreen(rect), from: nil)
  }

  @MainActor private func header(_ pr: Releash_Client_V1_PullRequestView, _ host: ViewHarness)
    throws -> NSRect
  {
    let title = try frame(pr.title, host)
    return NSRect(
      x: 0, y: title.minY - 8, width: host.window.contentLayoutRect.width,
      height: title.height + 16)
  }

  @MainActor private func assertCount(_ label: String, heading: String, _ host: ViewHarness) throws
  {
    let title = try frame(heading, host)
    let count = try frame(label, host)
    XCTAssertGreaterThan(count.midX, title.maxX)
    XCTAssertLessThan(abs(count.midY - title.midY), max(title.height, count.height))
  }

  @MainActor private func assertIcon(
    _ name: String, host: ViewHarness, region: NSRect, color: Color? = nil
  ) throws {
    host.window.appearance = NSAppearance(named: .aqua)
    let view = try XCTUnwrap(host.window.contentView)
    view.layoutSubtreeIfNeeded()
    host.window.displayIfNeeded()
    let region = region.integral.intersection(view.bounds)
    let bitmap = try XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: region))
    view.cacheDisplay(in: region, to: bitmap)
    let actual = try IconRaster(bitmap: bitmap)
    let svg = try XCTUnwrap(LucideAcceptanceReference.svg[name])
    let image = try XCTUnwrap(NSImage(data: Data(svg.utf8)))
    var matched = false
    for size in 10...24 {
      let reference = try IconRaster(
        image: image, size: size,
        scale: CGFloat(bitmap.pixelsWide) / region.width)
      if actual.find(reference, color: color) != nil {
        matched = true
        break
      }
    }
    XCTAssertTrue(matched, "Missing icon: " + name)
  }

  @MainActor private func checkIcon(beside name: String, _ host: ViewHarness) throws -> IconRaster {
    let row = try frame(name, host)
    let view = try XCTUnwrap(host.window.contentView)
    host.window.appearance = NSAppearance(named: .aqua)
    view.layoutSubtreeIfNeeded()
    host.window.displayIfNeeded()
    let region = NSRect(x: 0, y: row.minY - 2, width: row.minX, height: row.height + 4)
      .integral.intersection(view.bounds)
    let bitmap = try XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: region))
    view.cacheDisplay(in: region, to: bitmap)
    let icon = try IconRaster(bitmap: bitmap)
    let background = try XCTUnwrap(icon.pixels.first)
    XCTAssertTrue(
      icon.pixels.contains { pixel in
        let difference = pixel - background
        return (difference * difference).sum() > 0.03
      }, "Missing classification icon beside " + name)
    return icon
  }

  @MainActor private func assertMonospaced(_ text: String, _ host: ViewHarness) throws {
    let element = try find(text, host)
    var range = CFRange(location: 0, length: text.utf16.count)
    let parameter = try XCTUnwrap(AXValueCreate(.cfRange, &range))
    var output: CFTypeRef?
    XCTAssertEqual(
      AXUIElementCopyParameterizedAttributeValue(
        element,
        kAXAttributedStringForRangeParameterizedAttribute as CFString, parameter, &output), .success
    )
    let rendered = try XCTUnwrap(output as? NSAttributedString)
    let font = try XCTUnwrap(
      rendered.attribute(
        NSAttributedString.Key("AXFont"),
        at: 0, effectiveRange: nil) as? [String: Any])
    let name = try XCTUnwrap(font["AXFontName"] as? String)
    XCTAssertTrue(try XCTUnwrap(NSFont(name: name, size: 12)).isFixedPitch)
  }

  private func pullRequest() -> Releash_Client_V1_PullRequestView {
    .with {
      $0.number = 2128
      $0.title = "Thread 面を作り直す"
      $0.body = "PR body"
      $0.url = "https://github.com/example/repository/pull/2128"
      $0.state = .open
    }
  }

  @MainActor private func withPanel(
    central: Bool = false,
    _ body: (TestHTTPClient, AppModel, ViewHarness) async throws -> Void
  ) async throws {
    let http = TestHTTPClient()
    let model = try await testModel(http)
    model.selectWorktree("/repo/a")
    let host = await ViewHarness(
      HStack {
        if central { WorkspacePaneView(model: model).frame(width: 600) }
        RightPanelView(expanded: .constant(true), model: model).frame(width: 500)
      }, size: NSSize(width: central ? 1100 : 500, height: 1000))
    do {
      try await body(http, model, host)
    } catch {
      host.close()
      await model.stop()
      throw error
    }
    host.close()
    await model.stop()
  }

  @MainActor private func deliver(
    _ snapshot: Releash_Client_V1_PullRequestSnapshot, http: TestHTTPClient,
    path: String = "/repo/a"
  ) async throws {
    try await eventually {
      http.state.withLock {
        $0.subscriptions.contains { $0.target == "pull-request" && $0.args == [path] }
      }
    }
    try http.publish(.with { $0.pullRequest = snapshot }, target: "pull-request", args: [path])
  }

  @MainActor private func attribute(_ element: AXUIElement, _ name: String) -> CFTypeRef? {
    var value: CFTypeRef?
    guard AXUIElementCopyAttributeValue(element, name as CFString, &value) == .success else {
      return nil
    }
    return value
  }

  @MainActor private func elements(_ host: ViewHarness) -> [AXUIElement] {
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
      return [element]
        + (attribute(element, kAXChildrenAttribute) as? [AXUIElement] ?? []).flatMap(collect)
    }
    return collect(window)
  }
}

@MainActor private final class PullRequestDaemonFixture {
  let root: URL
  let data: URL
  let repository: URL
  let origin: URL
  let gh: URL
  let state: URL
  let helpers = Bundle.main.bundleURL.appendingPathComponent("Contents/Helpers")
  let daemon = Process()

  init(github: Bool, existingPR: Bool) throws {
    let temporary = try FileManager.default.temporaryDirectory.resourceValues(forKeys: [
      .canonicalPathKey
    ])
    root = URL(fileURLWithPath: try XCTUnwrap(temporary.canonicalPath))
      .appendingPathComponent("pr-acceptance-" + UUID().uuidString)
    data = root.appendingPathComponent("data")
    repository = root.appendingPathComponent("repository")
    origin = root.appendingPathComponent("origin.git")
    gh = root.appendingPathComponent("bin/gh")
    state = root.appendingPathComponent("github.json")
    do {
      for directory in [data, repository, gh.deletingLastPathComponent()] {
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
      }
      try git(["init", "-b", "main"])
      try git([
        "-c", "user.name=Acceptance", "-c", "user.email=acceptance@example.invalid",
        "-c", "commit.gpgsign=false", "commit", "--allow-empty", "-m", "initial",
      ])
      try git(["init", "--bare", origin.path])
      try git(["push", origin.path, "main"])
      try git([
        "remote", "add", "origin",
        github
          ? "https://github.com/acceptance/repository.git"
          : "https://gitlab.com/acceptance/repository.git",
      ])
      try git(["remote", "set-url", "--push", "origin", origin.path])
      try git(["update-ref", "refs/remotes/origin/main", "HEAD"])
      try git(["symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/main"])
      try git(["checkout", "-b", "feature"])
      if existingPR {
        try git(["push", origin.path, "feature"])
        try git(["update-ref", "refs/remotes/origin/feature", "HEAD"])
      }
      try JSONSerialization.data(withJSONObject: [
        "exists": existingPR, "passed": false, "now": Int(Date().timeIntervalSince1970),
      ]).write(to: state, options: .atomic)
      try "export PATH=\"$RELEASH_PR_BIN:$PATH\"\n".write(
        to: gh.deletingLastPathComponent().appendingPathComponent(".zprofile"), atomically: true,
        encoding: .utf8)
      try Self.script.write(to: gh, atomically: true, encoding: .utf8)
      try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: gh.path)
      daemon.executableURL = helpers.appendingPathComponent("releashd")
      daemon.arguments = ["--data-dir", data.path]
      daemon.currentDirectoryURL = repository
      var environment = ProcessInfo.processInfo.environment
      environment["RELEASH_DATA_DIR"] = data.path
      environment["RELEASH_WORKFLOWS_DIR"] = data.appendingPathComponent("workflows").path
      environment["RELEASH_PR_FIXTURE"] = state.path
      environment["SHELL"] = "/bin/zsh"
      environment["ZDOTDIR"] = gh.deletingLastPathComponent().path
      environment["RELEASH_PR_BIN"] = gh.deletingLastPathComponent().path
      environment["PATH"] =
        gh.deletingLastPathComponent().path + ":" + (environment["PATH"] ?? "/usr/bin:/bin")
      daemon.environment = environment
      daemon.standardInput = FileHandle.nullDevice
      daemon.standardOutput = FileHandle.nullDevice
      daemon.standardError = FileHandle.nullDevice
      try daemon.run()
    } catch {
      close()
      throw error
    }
  }

  func completeCheck() throws {
    var external = try XCTUnwrap(
      JSONSerialization.jsonObject(with: Data(contentsOf: state)) as? [String: Any])
    external["passed"] = true
    try JSONSerialization.data(withJSONObject: external).write(to: state, options: .atomic)
  }

  @discardableResult func git(_ arguments: [String]) throws -> Data {
    try run(URL(fileURLWithPath: "/usr/bin/git"), arguments)
  }

  @discardableResult func run(_ executable: URL, _ arguments: [String]) throws -> Data {
    let process = Process()
    process.executableURL = executable
    process.arguments = arguments
    process.currentDirectoryURL = repository
    var environment = ProcessInfo.processInfo.environment
    environment["RELEASH_PR_FIXTURE"] = state.path
    process.environment = environment
    let output = Pipe()
    process.standardOutput = output
    process.standardError = output
    try process.run()
    let bytes = output.fileHandleForReading.readDataToEndOfFile()
    process.waitUntilExit()
    guard process.terminationStatus == 0 else {
      throw CLIFailure(message: "\(arguments): \(String(decoding: bytes, as: UTF8.self))")
    }
    return bytes
  }

  func close() {
    if daemon.isRunning {
      daemon.terminate()
      daemon.waitUntilExit()
    }
    try? FileManager.default.removeItem(at: root)
  }

  private static let script = #"""
    #!/usr/bin/python3
    import datetime, json, os, sys
    from pathlib import Path

    path = Path(os.environ["RELEASH_PR_FIXTURE"])
    state = json.loads(path.read_text())
    args = sys.argv[1:]
    now = state["now"]
    def date(seconds):
        return datetime.datetime.fromtimestamp(now - seconds, datetime.timezone.utc).isoformat().replace("+00:00", "Z")
    def entry(author, body, seconds):
        return {"author": {"login": author, "__typename": "User"},
                "user": {"login": author, "type": "User"}, "body": body,
                "createdAt": date(seconds), "created_at": date(seconds),
                "submittedAt": date(seconds), "submitted_at": date(seconds)}
    comments = [entry("Last", "new comment", 3600), entry("First", "old comment", 10800)]
    comments += [entry("Poster", body, -1) for body in state.get("posted", [])]
    reviews = [entry("EmptyReviewer", "", 5400), entry("Reviewer", "review body", 7200)]
    lines = [dict(entry("LineReviewer", "code line comment", 1800), path="src/main.rs", line=1)]
    url = "https://github.com/acceptance/repository/pull/2128"
    checks = [{"name": "passed " + str(i), "bucket": "pass", "state": "SUCCESS",
               "startedAt": date(300), "completedAt": date(60), "link": "", "workflow": "CI"}
              for i in range(1, 8)]
    checks.append({"name": "running check", "bucket": "pass" if state["passed"] else "pending",
                   "state": "SUCCESS" if state["passed"] else "IN_PROGRESS",
                   "startedAt": date(300), "completedAt": date(0) if state["passed"] else "0001-01-01T00:00:00Z",
                   "link": "", "workflow": "CI"})
    rollup = [{"__typename": "CheckRun", "name": c["name"],
               "status": "COMPLETED" if c["bucket"] == "pass" else "IN_PROGRESS",
               "conclusion": "SUCCESS" if c["bucket"] == "pass" else None,
               "startedAt": c["startedAt"], "completedAt": c["completedAt"] or None,
               "detailsUrl": ""} for c in checks]
    pr = {"number": 2128, "title": "Thread 面を作り直す", "body": "PR body", "url": url,
          "html_url": url, "state": "OPEN", "isDraft": False, "draft": False,
          "headRefName": "feature", "baseRefName": "main", "head": {"ref": "feature"},
          "base": {"ref": "main"}, "comments": comments, "reviews": reviews,
          "statusCheckRollup": rollup, "reviewComments": lines}
    pr.update(state.get("created", {}))
    def save():
        temporary = path.with_suffix(".tmp")
        temporary.write_text(json.dumps(state))
        os.replace(temporary, path)
    def emit(value, status=0):
        if "--slurp" in args:
            value = [value]
        print(json.dumps(value))
        sys.exit(status)
    if args == ["--version"]:
        print("gh version 2.0.0 (acceptance fixture)")
    elif args[:2] == ["auth", "status"]:
        print("Logged in to github.com as acceptance")
    elif args[:2] == ["repo", "view"]:
        emit({"defaultBranchRef": {"name": "main"}, "nameWithOwner": "acceptance/repository"})
    elif args[:2] == ["pr", "list"]:
        requested = args[args.index("--state") + 1] if "--state" in args else "open"
        emit([pr] if state["exists"] and requested in ("all", "open") else [])
    elif args[:2] == ["pr", "view"]:
        if not state["exists"]:
            sys.exit("no pull request found")
        emit(pr)
    elif args[:2] == ["pr", "checks"]:
        emit(checks, 0 if state["passed"] else 8)
    elif args[:2] == ["pr", "create"]:
        state["exists"] = True
        state["created"] = {"title": args[args.index("--title") + 1],
                            "body": args[args.index("--body") + 1],
                            "headRefName": args[args.index("--head") + 1],
                            "baseRefName": args[args.index("--base") + 1],
                            "isDraft": "--draft" in args}
        save()
        print(url)
    elif args[:2] == ["pr", "comment"]:
        if args[2] != "2128":
            sys.exit("unknown pull request")
        state.setdefault("posted", []).append(args[args.index("--body") + 1])
        save()
        print(url + "#comment")
    elif args[:1] == ["api"]:
        endpoint = next((a for a in args[1:] if a.startswith("repos/")), "")
        method = args[args.index("--method") + 1] if "--method" in args else (
            args[args.index("-X") + 1] if "-X" in args else "GET")
        if method == "POST" and endpoint.endswith("/pulls"):
            state["exists"] = True
            path.write_text(json.dumps(state))
            emit(pr)
        elif endpoint.endswith("/issues/2128/comments"):
            emit(comments)
        elif endpoint.endswith("/pulls/2128/reviews"):
            emit(reviews)
        elif endpoint.endswith("/pulls/2128/comments"):
            emit(lines)
        elif endpoint.endswith("/pulls/2128"):
            emit(dict(pr, state="open"))
        else:
            sys.exit("unsupported fixture API: " + " ".join(args))
    else:
        sys.exit("unsupported fixture command: " + " ".join(args))
    """#
}
