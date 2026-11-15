import XCTest

@testable import Releash

final class CreateWorktreeFormTests: XCTestCase {
  func testListSubscriptionPassesSearchAndFiltersOfSelectedSource() {
    var form = CreateWorktreeForm()
    XCTAssertNil(form.list)
    form.repository = "/repo"
    XCTAssertEqual(form.list, .init(target: "available-branches", arguments: ["/repo"]))
    form.search = "  fix/side  "
    form.label = "bug"
    form.milestone = "04. ネイティブ UI"
    form[notionFilter: "Status"] = "未着手"
    form[notionFilter: "Assignee"] = "user-1"
    form[notionFilter: "Sprint"] = ""
    XCTAssertEqual(form[notionFilter: "Status"], "未着手")
    XCTAssertEqual(form[notionFilter: "Sprint"], "")
    XCTAssertEqual(
      form.list, .init(target: "available-branches", arguments: ["/repo", "search=fix/side"]))
    form.source = .issue
    XCTAssertEqual(
      form.list,
      .init(
        target: "issues",
        arguments: ["/repo", "label=bug", "milestone=04. ネイティブ UI", "search=fix/side"]))
    form.source = .notion
    XCTAssertEqual(
      form.list,
      .init(
        target: "notion-tasks",
        arguments: [
          "/repo", "100", "title=fix/side", #"labels={"Assignee":["user-1"],"Status":["未着手"]}"#,
        ]))
    form.search = " "
    form[notionFilter: "Status"] = ""
    form[notionFilter: "Assignee"] = ""
    XCTAssertEqual(form.list, .init(target: "notion-tasks", arguments: ["/repo", "100"]))
  }

  func testSwitchingSourceClearsChecksAndSwitchingRepositoryAlsoClearsBaseAndFilters() {
    var form = CreateWorktreeForm()
    form.repository = "/repo"
    form.baseBranch = "main"
    form.label = "bug"
    form.milestone = "v1"
    form.notionFilters = ["Status": "未着手"]
    form.selected.set(.branch("feature"), branch: "feature", selected: true)
    form.source = .branch
    XCTAssertTrue(form.canSubmit)
    XCTAssertNil(form.notionOptionsRepository)

    form.source = .notion
    XCTAssertFalse(form.canSubmit)
    XCTAssertEqual(form.baseBranch, "main")
    XCTAssertEqual(form.notionFilters, ["Status": "未着手"])
    XCTAssertEqual(form.notionOptionsRepository, "/repo")

    form.source = .issue
    XCTAssertEqual(form.notionFilters, ["Status": "未着手"])
    XCTAssertEqual(form.notionOptionsRepository, "/repo")

    form.selected.set(.issue(1), branch: "feat/issues/1", selected: true)
    form.repository = "/other"
    XCTAssertFalse(form.canSubmit)
    XCTAssertEqual(form.baseBranch, "")
    XCTAssertEqual(form.label, "")
    XCTAssertEqual(form.milestone, "")
    XCTAssertEqual(form.notionFilters, [:])
    XCTAssertNil(form.notionOptionsRepository)
  }
}
